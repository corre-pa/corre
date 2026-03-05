//! Async helper for app authors to interact with the host via CCPP.
//!
//! [`AppClient`] spawns a background reader task that demultiplexes incoming
//! messages: responses are routed to their waiting callers via oneshot channels, while
//! requests and notifications from the host (e.g. shutdown, cancel) are forwarded to an
//! mpsc channel readable via [`read_message`](AppClient::read_message). This
//! allows multiple RPC round-trips to be in-flight simultaneously.

use crate::codec::{CodecReader, CodecWriter};
use crate::llm::{LlmRequest, LlmResponse};
use crate::protocol::{
    AppErrorParams, AppResultParams, InitializeParams, InitializeResult, LogParams, McpCallToolParams, McpListToolsParams, Message,
    Notification, OutputRestParams, OutputStreamParams, OutputWebhookParams, OutputWriteParams, ProgressParams, Request, Response,
};
use crate::types::AppOutput;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

/// A CCPP client that app binaries use to communicate with the host.
///
/// Handles the JSON-RPC transport, request ID tracking, and response demultiplexing.
/// A background reader task reads all incoming messages and dispatches responses to
/// the correct waiter, enabling true concurrent RPC calls.
pub struct AppClient<W: AsyncWrite + Unpin + Send> {
    writer: Mutex<CodecWriter<W>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>,
    incoming_rx: Mutex<mpsc::UnboundedReceiver<Message>>,
    reader_handle: JoinHandle<()>,
}

impl AppClient<tokio::io::Stdout> {
    /// Create a client connected to stdin/stdout (the standard CCPP transport).
    pub fn from_stdio() -> Self {
        Self::new(tokio::io::stdin(), tokio::io::stdout())
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> AppClient<W> {
    pub fn new<R: AsyncRead + Unpin + Send + 'static>(reader: R, writer: W) -> Self {
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();

        let pending_for_reader = pending.clone();
        let reader_handle = tokio::spawn(reader_loop(CodecReader::new(reader), pending_for_reader, incoming_tx));

        Self {
            writer: Mutex::new(CodecWriter::new(writer)),
            next_id: AtomicU64::new(100),
            pending,
            incoming_rx: Mutex::new(incoming_rx),
            reader_handle,
        }
    }
}

impl<W: AsyncWrite + Unpin + Send> AppClient<W> {
    /// Wait for the `initialize` request from the host and return the params.
    /// Automatically sends the initialize response.
    pub async fn accept_initialize(&self) -> anyhow::Result<InitializeParams> {
        let msg = self.incoming_rx.lock().await.recv().await.ok_or_else(|| anyhow::anyhow!("unexpected EOF waiting for initialize"))?;

        match msg {
            Message::Request(req) if req.method == "initialize" => {
                let params: InitializeParams =
                    serde_json::from_value(req.params.ok_or_else(|| anyhow::anyhow!("initialize missing params"))?)?;

                let result = InitializeResult { protocol_version: params.protocol_version.clone(), capabilities: serde_json::json!({}) };

                let resp = Response::ok(req.id, serde_json::to_value(&result)?);
                self.writer.lock().await.write_response(&resp).await?;

                Ok(params)
            }
            _ => anyhow::bail!("expected initialize request, got: {msg:?}"),
        }
    }

    /// Call an MCP tool on the host and wait for the result.
    pub async fn call_tool(&self, server_name: &str, tool_name: &str, arguments: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let params = McpCallToolParams { server_name: server_name.into(), tool_name: tool_name.into(), arguments };
        let resp = self.send_request("mcp/callTool", serde_json::to_value(&params)?).await?;
        match resp.error {
            Some(err) => anyhow::bail!("mcp/callTool error {}: {}", err.code, err.message),
            None => Ok(resp.result.unwrap_or(serde_json::Value::Null)),
        }
    }

    /// List available tools on an MCP server.
    pub async fn list_tools(&self, server_name: &str) -> anyhow::Result<Vec<String>> {
        let params = McpListToolsParams { server_name: server_name.into() };
        let resp = self.send_request("mcp/listTools", serde_json::to_value(&params)?).await?;
        match resp.error {
            Some(err) => anyhow::bail!("mcp/listTools error {}: {}", err.code, err.message),
            None => {
                let result = resp.result.unwrap_or(serde_json::Value::Null);
                Ok(serde_json::from_value(result).unwrap_or_default())
            }
        }
    }

    /// Send an LLM completion request to the host and wait for the response.
    pub async fn llm_complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse> {
        let resp = self.send_request("llm/complete", serde_json::to_value(&request)?).await?;
        match resp.error {
            Some(err) => anyhow::bail!("llm/complete error {}: {}", err.code, err.message),
            None => {
                let result = resp.result.ok_or_else(|| anyhow::anyhow!("llm/complete returned no result"))?;
                Ok(serde_json::from_value(result)?)
            }
        }
    }

    /// Send a progress notification to the host.
    pub async fn report_progress(&self, phase: &str, percent: Option<u8>, message: Option<&str>) -> anyhow::Result<()> {
        let params = ProgressParams { phase: phase.into(), percent, message: message.map(Into::into) };
        self.send_notification("progress", serde_json::to_value(&params)?).await
    }

    /// Send a log notification to the host.
    pub async fn log(&self, level: &str, message: &str) -> anyhow::Result<()> {
        let params = LogParams { level: level.into(), message: message.into() };
        self.send_notification("log", serde_json::to_value(&params)?).await
    }

    /// Send the final app result to the host.
    pub async fn send_result(&self, output: AppOutput) -> anyhow::Result<()> {
        let params = AppResultParams { output };
        self.send_notification("app/result", serde_json::to_value(&params)?).await
    }

    /// Send an error notification to the host.
    pub async fn send_error(&self, message: &str, phase: Option<&str>, partial_output: Option<AppOutput>) -> anyhow::Result<()> {
        let params = AppErrorParams { message: message.into(), phase: phase.map(Into::into), partial_output };
        self.send_notification("app/error", serde_json::to_value(&params)?).await
    }

    // ── Output methods ─────────────────────────────────────────────────

    /// Write a file to a permitted filesystem path on the host.
    pub async fn write_file(&self, path: &str, data: &str, content_type: Option<&str>) -> anyhow::Result<()> {
        let params = OutputWriteParams { path: path.into(), data: data.into(), content_type: content_type.map(Into::into) };
        let resp = self.send_request("output/write", serde_json::to_value(&params)?).await?;
        if let Some(err) = resp.error {
            anyhow::bail!("output/write error {}: {}", err.code, err.message);
        }
        Ok(())
    }

    /// Stream a text chunk to the host (notification, no response).
    pub async fn stream_text(&self, chunk: &str, is_final: bool) -> anyhow::Result<()> {
        let params = OutputStreamParams { chunk: chunk.into(), r#final: is_final };
        self.send_notification("output/stream", serde_json::to_value(&params)?).await
    }

    /// POST output to a permitted REST endpoint via the host.
    pub async fn post_rest(&self, url: &str, body: serde_json::Value, content_type: Option<&str>) -> anyhow::Result<()> {
        let params = OutputRestParams { url: url.into(), body, content_type: content_type.map(Into::into) };
        let resp = self.send_request("output/rest", serde_json::to_value(&params)?).await?;
        if let Some(err) = resp.error {
            anyhow::bail!("output/rest error {}: {}", err.code, err.message);
        }
        Ok(())
    }

    /// Fire a webhook via the host.
    pub async fn fire_webhook(&self, url: &str, body: serde_json::Value) -> anyhow::Result<()> {
        let params = OutputWebhookParams { url: url.into(), body };
        let resp = self.send_request("output/webhook", serde_json::to_value(&params)?).await?;
        if let Some(err) = resp.error {
            anyhow::bail!("output/webhook error {}: {}", err.code, err.message);
        }
        Ok(())
    }

    /// Read the next incoming message from the host (e.g. shutdown, cancel).
    pub async fn read_message(&self) -> anyhow::Result<Option<Message>> {
        Ok(self.incoming_rx.lock().await.recv().await)
    }

    // ── Internal ──────────────────────────────────────────────────────

    async fn send_request(&self, method: &str, params: serde_json::Value) -> anyhow::Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        tracing::debug!("Sending {method} (id={id}) params: {}", serde_json::to_string(&params).unwrap_or_default());
        let req = Request::new(id, method, Some(params));

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        // Write the request — writer lock is held only for the duration of the write
        self.writer.lock().await.write_request(&req).await?;
        tracing::debug!("Sent {method} (id={id}), awaiting response");

        // Wait for the reader task to deliver our response — no locks held
        let resp = rx.await.map_err(|_| anyhow::anyhow!("reader task shut down while waiting for response to {method} (id={id})"))?;
        if let Some(ref result) = resp.result {
            let preview = serde_json::to_string(result).unwrap_or_default();
            let truncated = if preview.len() > 500 { &preview[..500] } else { &preview };
            tracing::debug!("Response {method} (id={id}) result: {truncated}");
        }
        if let Some(ref err) = resp.error {
            tracing::debug!("Response {method} (id={id}) error: code={} msg={}", err.code, err.message);
        }
        Ok(resp)
    }

    async fn send_notification(&self, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
        let notif = Notification::new(method, Some(params));
        self.writer.lock().await.write_notification(&notif).await
    }
}

impl<W: AsyncWrite + Unpin + Send> Drop for AppClient<W> {
    fn drop(&mut self) {
        self.reader_handle.abort();
    }
}

// ── Background reader task ───────────────────────────────────────────────

/// Reads messages from the transport and dispatches them:
/// - Responses go to the matching oneshot sender in `pending`
/// - Requests and notifications go to `incoming_tx`
async fn reader_loop<R: AsyncRead + Unpin>(
    mut reader: CodecReader<R>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>,
    incoming_tx: mpsc::UnboundedSender<Message>,
) {
    tracing::debug!("reader_loop started");
    loop {
        match reader.read_message().await {
            Ok(Some(Message::Response(resp))) => {
                let id = resp.id;
                let is_err = resp.error.is_some();
                if let Some(sender) = pending.lock().await.remove(&id) {
                    let _ = sender.send(resp);
                    tracing::debug!("Delivered response id={id}{}", if is_err { " [error]" } else { "" });
                } else {
                    tracing::debug!("received response for unknown request id {id}");
                }
            }
            Ok(Some(msg)) => {
                let desc = match &msg {
                    Message::Request(r) => format!("request {}", r.method),
                    Message::Notification(n) => format!("notification {}", n.method),
                    Message::Response(r) => format!("response id={}", r.id),
                };
                tracing::debug!("reader_loop forwarding {desc}");
                if incoming_tx.send(msg).is_err() {
                    tracing::debug!("reader_loop: incoming channel closed, exiting");
                    break;
                }
            }
            Ok(None) => {
                let remaining = pending.lock().await.len();
                tracing::debug!("reader_loop: EOF with {remaining} pending requests, draining");
                pending.lock().await.drain();
                break;
            }
            Err(e) => {
                let remaining = pending.lock().await.len();
                tracing::debug!("reader_loop: codec error with {remaining} pending: {e}");
                pending.lock().await.drain();
                break;
            }
        }
    }
    tracing::debug!("reader_loop exited");
}
