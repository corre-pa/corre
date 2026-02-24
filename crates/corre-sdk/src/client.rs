//! Async helper for capability authors to interact with the host via CCPP.

use crate::codec::MessageCodec;
use crate::llm::{LlmRequest, LlmResponse};
use crate::protocol::{
    CapabilityErrorParams, CapabilityResultParams, InitializeParams, InitializeResult, LogParams, McpCallToolParams, McpListToolsParams,
    Message, Notification, OutputRestParams, OutputStreamParams, OutputWebhookParams, OutputWriteParams, ProgressParams, Request, Response,
};
use crate::types::CapabilityOutput;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, oneshot};

/// A CCPP client that capability binaries use to communicate with the host.
///
/// Handles the JSON-RPC transport, request ID tracking, and response demultiplexing.
pub struct CapabilityClient<R: AsyncRead + Unpin + Send, W: AsyncWrite + Unpin + Send> {
    codec: Mutex<MessageCodec<R, W>>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Response>>>,
}

impl CapabilityClient<tokio::io::Stdin, tokio::io::Stdout> {
    /// Create a client connected to stdin/stdout (the standard CCPP transport).
    pub fn from_stdio() -> Self {
        Self::new(tokio::io::stdin(), tokio::io::stdout())
    }
}

impl<R: AsyncRead + Unpin + Send, W: AsyncWrite + Unpin + Send> CapabilityClient<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { codec: Mutex::new(MessageCodec::new(reader, writer)), next_id: AtomicU64::new(100), pending: Mutex::new(HashMap::new()) }
    }

    /// Wait for the `initialize` request from the host and return the params.
    /// Automatically sends the initialize response.
    pub async fn accept_initialize(&self) -> anyhow::Result<InitializeParams> {
        let msg = {
            let mut codec = self.codec.lock().await;
            codec.read_message().await?
        };
        let msg = msg.ok_or_else(|| anyhow::anyhow!("unexpected EOF waiting for initialize"))?;

        match msg {
            Message::Request(req) if req.method == "initialize" => {
                let params: InitializeParams =
                    serde_json::from_value(req.params.ok_or_else(|| anyhow::anyhow!("initialize missing params"))?)?;

                let result = InitializeResult { protocol_version: params.protocol_version.clone(), capabilities: serde_json::json!({}) };

                let resp = Response::ok(req.id, serde_json::to_value(&result)?);
                let mut codec = self.codec.lock().await;
                codec.write_response(&resp).await?;

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

    /// Send the final capability result to the host.
    pub async fn send_result(&self, output: CapabilityOutput) -> anyhow::Result<()> {
        let params = CapabilityResultParams { output };
        self.send_notification("capability/result", serde_json::to_value(&params)?).await
    }

    /// Send an error notification to the host.
    pub async fn send_error(&self, message: &str, phase: Option<&str>, partial_output: Option<CapabilityOutput>) -> anyhow::Result<()> {
        let params = CapabilityErrorParams { message: message.into(), phase: phase.map(Into::into), partial_output };
        self.send_notification("capability/error", serde_json::to_value(&params)?).await
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

    /// Read the next incoming message (used for shutdown/cancel from host).
    pub async fn read_message(&self) -> anyhow::Result<Option<Message>> {
        let mut codec = self.codec.lock().await;
        codec.read_message().await
    }

    // ── Internal ──────────────────────────────────────────────────────

    async fn send_request(&self, method: &str, params: serde_json::Value) -> anyhow::Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request::new(id, method, Some(params));

        let (tx, mut rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        {
            let mut codec = self.codec.lock().await;
            codec.write_request(&req).await?;
        }

        // Read messages until we get our response (dispatching others along the way)
        loop {
            let msg = {
                let mut codec = self.codec.lock().await;
                codec.read_message().await?
            };

            match msg {
                Some(Message::Response(resp)) => {
                    if resp.id == id {
                        return Ok(resp);
                    }
                    // Dispatch to other waiters
                    if let Some(sender) = self.pending.lock().await.remove(&resp.id) {
                        let _ = sender.send(resp);
                    }
                }
                Some(Message::Request(req)) => {
                    tracing::debug!("received unexpected request from host: {}", req.method);
                }
                Some(Message::Notification(_)) => {
                    // Notifications from host (shutdown, cancel) — handled by caller
                }
                None => anyhow::bail!("unexpected EOF while waiting for response to {method} (id={id})"),
            }

            // Check if our response arrived via another reader
            if let Ok(resp) = rx.try_recv() {
                return Ok(resp);
            }
        }
    }

    async fn send_notification(&self, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
        let notif = Notification::new(method, Some(params));
        let mut codec = self.codec.lock().await;
        codec.write_notification(&notif).await
    }
}
