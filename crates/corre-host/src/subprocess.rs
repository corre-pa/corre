//! Subprocess-based capability that implements the CCPP v1/v2 protocol.
//!
//! Spawns a child process, sends `initialize`, then serves `mcp/callTool`,
//! `mcp/listTools`, `llm/complete`, and `output/*` requests from the plugin
//! using the host's safety-wrapped MCP and LLM providers.

use corre_core::capability::{Capability, CapabilityContext, CapabilityManifest, CapabilityOutput, ProgressEvent, ProgressStatus};
use corre_core::sandbox::LandlockSandbox;
use corre_sdk::manifest::{OutputDeclaration, OutputType, SandboxPermissions};
use corre_sdk::protocol::{
    self, CapabilityErrorParams, CapabilityResultParams, ErrorCode, InitializeParams, LogParams, McpCallToolParams, McpListToolsParams,
    Message, Notification, OutputRestParams, OutputStreamParams, OutputWebhookParams, OutputWriteParams, PROTOCOL_VERSION, ProgressParams,
    Request, Response,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

/// Keys whose string values should be replaced with `[REDACTED]` in debug logs.
const REDACT_KEYS: &[&str] = &["api_key", "secret", "token", "password", "authorization", "key"];

/// Recursively walk a JSON value and replace secret-looking strings with `[REDACTED]`.
///
/// A value is considered secret if:
/// - It sits under a key matching one of `REDACT_KEYS` (case-insensitive), or
/// - It is a string starting with `Bearer ` or `sk-`.
fn redact_secrets(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let redacted = map.iter().map(|(k, v)| {
                let key_lower = k.to_lowercase();
                if REDACT_KEYS.iter().any(|&rk| key_lower.contains(rk)) {
                    if v.is_string() {
                        return (k.clone(), serde_json::Value::String("[REDACTED]".into()));
                    }
                }
                (k.clone(), redact_secrets(v))
            });
            serde_json::Value::Object(redacted.collect())
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(arr.iter().map(redact_secrets).collect()),
        serde_json::Value::String(s) => {
            if s.starts_with("Bearer ") || s.starts_with("sk-") {
                serde_json::Value::String("[REDACTED]".into())
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

/// A capability backed by an external subprocess speaking CCPP v1/v2.
pub struct SubprocessCapability {
    manifest: CapabilityManifest,
    binary: PathBuf,
    plugin_dir: PathBuf,
    /// Declared output destinations from the plugin manifest.
    output_declarations: Vec<OutputDeclaration>,
    /// Sandbox permissions from the plugin manifest (None = no sandbox).
    sandbox_perms: Option<SandboxPermissions>,
    /// Data directory root for resolving output paths.
    data_dir: PathBuf,
    /// Resolved log level for this plugin (per-capability override or global fallback).
    log_level: Option<String>,
    progress: RwLock<SubprocessProgress>,
}

struct SubprocessProgress {
    phase: String,
    percent: Option<u8>,
    output: Option<CapabilityOutput>,
    error: Option<String>,
}

/// Send a newline-terminated JSON message to a child's stdin.
async fn write_msg(stdin: &tokio::sync::Mutex<tokio::process::ChildStdin>, value: &serde_json::Value) -> anyhow::Result<()> {
    let mut json = serde_json::to_string(value)?;
    json.push('\n');
    let mut guard = stdin.lock().await;
    guard.write_all(json.as_bytes()).await?;
    guard.flush().await?;
    Ok(())
}

impl SubprocessCapability {
    pub fn new(manifest: CapabilityManifest, binary: PathBuf, plugin_dir: PathBuf) -> Self {
        Self {
            manifest,
            binary,
            plugin_dir,
            output_declarations: Vec::new(),
            sandbox_perms: None,
            data_dir: PathBuf::new(),
            log_level: None,
            progress: RwLock::new(SubprocessProgress { phase: "init".into(), percent: None, output: None, error: None }),
        }
    }

    /// Set the output declarations from the plugin manifest.
    pub fn with_outputs(mut self, outputs: Vec<OutputDeclaration>) -> Self {
        self.output_declarations = outputs;
        self
    }

    /// Set the sandbox permissions from the plugin manifest.
    pub fn with_sandbox(mut self, perms: Option<SandboxPermissions>) -> Self {
        self.sandbox_perms = perms;
        self
    }

    /// Set the data directory for resolving output paths.
    pub fn with_data_dir(mut self, data_dir: PathBuf) -> Self {
        self.data_dir = data_dir;
        self
    }

    /// Set the resolved log level for this plugin.
    pub fn with_log_level(mut self, level: String) -> Self {
        self.log_level = Some(level);
        self
    }

    /// Read one newline-terminated JSON message from the child's stdout.
    /// Skips blank lines; returns `None` only on true EOF (zero bytes read).
    async fn read_msg(reader: &mut BufReader<tokio::process::ChildStdout>) -> anyhow::Result<Option<serde_json::Value>> {
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(None);
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() > protocol::MAX_MESSAGE_BYTES {
                anyhow::bail!("message exceeds {} byte limit", protocol::MAX_MESSAGE_BYTES);
            }
            return Ok(Some(serde_json::from_str(trimmed)?));
        }
    }

    /// Send `initialize` and wait for the response.
    async fn initialize(
        stdin: &tokio::sync::Mutex<tokio::process::ChildStdin>,
        reader: &mut BufReader<tokio::process::ChildStdout>,
        params: &InitializeParams,
    ) -> anyhow::Result<()> {
        let req = Request::new(1, "initialize", Some(serde_json::to_value(params)?));
        write_msg(stdin, &serde_json::to_value(&req)?).await?;

        let resp_val = Self::read_msg(reader).await?.ok_or_else(|| anyhow::anyhow!("plugin closed before initialize response"))?;

        let resp: Response = serde_json::from_value(resp_val)?;
        if let Some(err) = resp.error {
            anyhow::bail!("plugin initialize failed ({}): {}", err.code, err.message);
        }

        Ok(())
    }

    /// Send `shutdown` notification and wait for the process to exit.
    async fn shutdown(stdin: &tokio::sync::Mutex<tokio::process::ChildStdin>, child: &mut Child) {
        let notif = Notification::new("shutdown", None);
        let _ = write_msg(stdin, &serde_json::to_value(&notif).unwrap()).await;

        // 5s grace period
        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                tracing::warn!("plugin did not exit in 5s, killing");
                let _ = child.kill().await;
            }
        }
    }

    /// Dispatch a single request from the plugin.
    async fn dispatch_request(ctx: &CapabilityContext, req: &Request, outputs: &[OutputDeclaration], data_dir: &Path) -> Response {
        match req.method.as_str() {
            "mcp/callTool" => Self::handle_mcp_call_tool(ctx, req).await,
            "mcp/listTools" => Self::handle_mcp_list_tools(ctx, req).await,
            "llm/complete" => Self::handle_llm_complete(ctx, req).await,
            "output/write" => Self::handle_output_write(req, outputs, data_dir).await,
            "output/rest" => Self::handle_output_rest(req, outputs).await,
            "output/webhook" => Self::handle_output_webhook(req, outputs).await,
            _ => Response::err(req.id, ErrorCode::METHOD_NOT_FOUND, format!("unknown method: {}", req.method)),
        }
    }

    async fn handle_mcp_call_tool(ctx: &CapabilityContext, req: &Request) -> Response {
        let params: McpCallToolParams = match req.params.as_ref().and_then(|p| serde_json::from_value(p.clone()).ok()) {
            Some(p) => p,
            None => return Response::err(req.id, ErrorCode::INVALID_PARAMS, "invalid mcp/callTool params"),
        };

        match ctx.mcp.call_tool(&params.server_name, &params.tool_name, params.arguments).await {
            Ok(result) => Response::ok(req.id, result),
            Err(e) => Response::err(req.id, ErrorCode::MCP_TOOL_ERROR, format!("{e:#}")),
        }
    }

    async fn handle_mcp_list_tools(ctx: &CapabilityContext, req: &Request) -> Response {
        let params: McpListToolsParams = match req.params.as_ref().and_then(|p| serde_json::from_value(p.clone()).ok()) {
            Some(p) => p,
            None => return Response::err(req.id, ErrorCode::INVALID_PARAMS, "invalid mcp/listTools params"),
        };

        match ctx.mcp.list_tools(&params.server_name).await {
            Ok(tools) => Response::ok(req.id, serde_json::to_value(&tools).unwrap_or_default()),
            Err(e) => Response::err(req.id, ErrorCode::MCP_SERVER_UNAVAILABLE, format!("{e:#}")),
        }
    }

    async fn handle_llm_complete(ctx: &CapabilityContext, req: &Request) -> Response {
        // SDK and core LLM types are identical (core re-exports from SDK).
        let llm_req: corre_core::capability::LlmRequest = match req.params.as_ref().and_then(|p| serde_json::from_value(p.clone()).ok()) {
            Some(r) => r,
            None => return Response::err(req.id, ErrorCode::INVALID_PARAMS, "invalid llm/complete params"),
        };

        match ctx.llm.complete(llm_req).await {
            Ok(resp) => Response::ok(req.id, serde_json::to_value(&resp).unwrap_or_default()),
            Err(e) => {
                let err_str = format!("{e:#}");
                let lower = err_str.to_lowercase();
                if lower.contains("429") || lower.contains("rate limit") {
                    Response::err(req.id, ErrorCode::LLM_RATE_LIMITED, err_str)
                } else if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") || lower.contains("forbidden") {
                    Response::fatal_err(req.id, ErrorCode::LLM_AUTH_FAILED, err_str)
                } else if lower.contains("402") || lower.contains("payment required") || lower.contains("insufficient") {
                    Response::fatal_err(req.id, ErrorCode::LLM_PAYMENT_REQUIRED, err_str)
                } else if lower.contains("finish_reason=length") || lower.contains("max_tokens") {
                    Response::err(req.id, ErrorCode::LLM_TRUNCATED, err_str)
                } else {
                    Response::err(req.id, ErrorCode::LLM_PROVIDER_ERROR, err_str)
                }
            }
        }
    }

    // ── Output handlers ──────────────────────────────────────────────────

    async fn handle_output_write(req: &Request, outputs: &[OutputDeclaration], data_dir: &Path) -> Response {
        let params: OutputWriteParams = match req.params.as_ref().and_then(|p| serde_json::from_value(p.clone()).ok()) {
            Some(p) => p,
            None => return Response::err(req.id, ErrorCode::INVALID_PARAMS, "invalid output/write params"),
        };

        if !is_output_permitted(&params.path, OutputType::Filesystem, outputs) {
            return Response::err(req.id, ErrorCode::OUTPUT_DENIED, format!("output/write to `{}` not declared in manifest", params.path));
        }

        let full_path = data_dir.join(&params.path);
        if let Some(parent) = full_path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return Response::err(
                req.id,
                ErrorCode::OUTPUT_FAILED,
                format!("failed to create directories for {}: {e}", full_path.display()),
            );
        }

        match tokio::fs::write(&full_path, params.data.as_bytes()).await {
            Ok(()) => Response::ok(req.id, serde_json::json!({"written": full_path.display().to_string()})),
            Err(e) => Response::err(req.id, ErrorCode::OUTPUT_FAILED, format!("write to {} failed: {e}", full_path.display())),
        }
    }

    async fn handle_output_rest(req: &Request, outputs: &[OutputDeclaration]) -> Response {
        let params: OutputRestParams = match req.params.as_ref().and_then(|p| serde_json::from_value(p.clone()).ok()) {
            Some(p) => p,
            None => return Response::err(req.id, ErrorCode::INVALID_PARAMS, "invalid output/rest params"),
        };

        if !is_output_permitted(&params.url, OutputType::Rest, outputs) {
            return Response::err(req.id, ErrorCode::OUTPUT_DENIED, format!("output/rest to `{}` not declared in manifest", params.url));
        }

        let client = reqwest::Client::new();
        let mut request = client.post(&params.url).json(&params.body);
        if let Some(ref ct) = params.content_type {
            request = request.header("Content-Type", ct);
        }

        match request.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                Response::ok(req.id, serde_json::json!({"status": status}))
            }
            Err(e) => Response::err(req.id, ErrorCode::OUTPUT_FAILED, format!("REST POST failed: {e}")),
        }
    }

    async fn handle_output_webhook(req: &Request, outputs: &[OutputDeclaration]) -> Response {
        let params: OutputWebhookParams = match req.params.as_ref().and_then(|p| serde_json::from_value(p.clone()).ok()) {
            Some(p) => p,
            None => return Response::err(req.id, ErrorCode::INVALID_PARAMS, "invalid output/webhook params"),
        };

        if !is_output_permitted(&params.url, OutputType::Webhook, outputs) {
            return Response::err(req.id, ErrorCode::OUTPUT_DENIED, format!("output/webhook to `{}` not declared in manifest", params.url));
        }

        let client = reqwest::Client::new();
        match client.post(&params.url).json(&params.body).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                Response::ok(req.id, serde_json::json!({"status": status}))
            }
            Err(e) => Response::err(req.id, ErrorCode::OUTPUT_FAILED, format!("webhook failed: {e}")),
        }
    }

    /// Compute the per-capability scoped data directory.
    fn scoped_data_dir(&self) -> PathBuf {
        self.data_dir.join(&self.manifest.name)
    }

    /// Build the `Command` for spawning the plugin, optionally sandboxed with Landlock + seccomp.
    fn build_spawn_command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.current_dir(&self.plugin_dir);
        if let Some(ref perms) = self.sandbox_perms {
            let sandbox = LandlockSandbox::from_permissions(perms, &self.plugin_dir, &self.data_dir, &self.manifest.name);
            sandbox.apply_to_command(&mut cmd);
        }
        cmd
    }
}

/// Check whether a given target (path or URL) matches a declared output.
fn is_output_permitted(target: &str, output_type: OutputType, declarations: &[OutputDeclaration]) -> bool {
    // If no outputs are declared, deny all output requests
    declarations.iter().any(|d| d.output_type == output_type && target_matches_declaration(target, &d.target))
}

/// Simple glob-style matching: a declaration target is a prefix/pattern.
/// For filesystem: "editions/{date}/edition.json" matches "editions/2026-02-26/edition.json"
/// For URLs: exact prefix match.
fn target_matches_declaration(target: &str, pattern: &str) -> bool {
    if pattern.contains('{') {
        // Convert template to a simple prefix match: everything before the first `{`
        let prefix = pattern.split('{').next().unwrap_or("");
        target.starts_with(prefix)
    } else {
        target == pattern || target.starts_with(pattern)
    }
}

#[async_trait::async_trait]
impl Capability for SubprocessCapability {
    fn manifest(&self) -> &CapabilityManifest {
        &self.manifest
    }

    async fn execute(&self, ctx: &CapabilityContext) -> anyhow::Result<CapabilityOutput> {
        // Reset progress state
        {
            let mut progress = self.progress.write().unwrap_or_else(|e| e.into_inner());
            progress.phase = "init".into();
            progress.percent = None;
            progress.output = None;
            progress.error = None;
        }

        // Per-capability scoped directory — create it, config/ and logs/ subdirs before spawning
        let scoped_data_dir = self.scoped_data_dir();
        let log_dir = scoped_data_dir.join("logs");
        tokio::fs::create_dir_all(scoped_data_dir.join("config"))
            .await
            .map_err(|e| anyhow::anyhow!("failed to create capability dir {}: {e}", scoped_data_dir.display()))?;
        tokio::fs::create_dir_all(&log_dir).await.map_err(|e| anyhow::anyhow!("failed to create log dir {}: {e}", log_dir.display()))?;

        let mut child = self
            .build_spawn_command()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                anyhow::anyhow!("failed to spawn plugin {} (workdir: {}): {e}", self.binary.display(), self.plugin_dir.display())
            })?;

        let raw_stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("no stderr"))?;

        let stdin = Arc::new(tokio::sync::Mutex::new(raw_stdin));
        let mut reader = BufReader::new(stdout);

        // Capture stderr in background, parsing child tracing output to preserve
        // log levels and coalesce multi-line messages into single entries.
        let cap_name = self.manifest.name.clone();
        tokio::spawn(async move {
            let mut stderr_reader = BufReader::new(stderr);
            let mut line = String::new();
            let mut pending_level = tracing::Level::DEBUG;
            let mut pending_msg = String::new();

            while let Ok(n) = stderr_reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let clean = strip_ansi(&line);
                let trimmed = clean.trim_end();
                if trimmed.is_empty() {
                    line.clear();
                    continue;
                }

                if let Some((level, msg)) = parse_plugin_level(trimmed) {
                    // New log entry — flush the previous one
                    if !pending_msg.is_empty() {
                        log_plugin_msg(&cap_name, pending_level, &pending_msg);
                    }
                    pending_level = level;
                    pending_msg = msg.to_string();
                } else {
                    // Continuation line (multi-line message)
                    if !pending_msg.is_empty() {
                        pending_msg.push('\n');
                    }
                    pending_msg.push_str(trimmed);
                }

                line.clear();
            }

            if !pending_msg.is_empty() {
                log_plugin_msg(&cap_name, pending_level, &pending_msg);
            }
        });

        // Send initialize — config_dir points at the capability's scoped directory
        let init_params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.into(),
            capability_name: self.manifest.name.clone(),
            config_dir: scoped_data_dir.to_string_lossy().into_owned(),
            config_path: self.manifest.config_path.clone(),
            seen_urls: ctx.seen_urls.iter().cloned().collect(),
            max_concurrent_llm: ctx.max_concurrent_llm,
            mcp_servers: self.manifest.mcp_servers.clone(),
            timeout_secs: 600,
            log_dir: Some(log_dir.to_string_lossy().into_owned()),
            log_level: self.log_level.clone(),
        };

        Self::initialize(&stdin, &mut reader, &init_params).await?;

        // SAFETY: all spawned request tasks are joined before this function returns,
        // so the references remain valid for the duration of every task.
        let ctx: &'static CapabilityContext = unsafe { std::mem::transmute(ctx) };
        let output_declarations: &'static [OutputDeclaration] = unsafe { std::mem::transmute(self.output_declarations.as_slice()) };
        let scoped_data_dir: &'static Path = unsafe { std::mem::transmute(scoped_data_dir.as_path()) };

        let mut request_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
        let fatal_error: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));

        // Main message loop: read from plugin, dispatch, respond
        loop {
            // Check if a fatal error was reported by a dispatch task
            let maybe_fatal = fatal_error.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(msg) = maybe_fatal {
                tracing::error!("Fatal error from dispatch task, aborting: {msg}");
                for handle in &mut request_handles {
                    let _ = handle.await;
                }
                Self::shutdown(&stdin, &mut child).await;
                anyhow::bail!("{msg}");
            }

            let msg_val = match Self::read_msg(&mut reader).await? {
                Some(v) => v,
                None => {
                    // EOF — plugin exited; join outstanding tasks before returning
                    tracing::debug!("Plugin stdout EOF, joining {} outstanding request handles", request_handles.len());
                    for handle in &mut request_handles {
                        let _ = handle.await;
                    }
                    break;
                }
            };

            let msg: Message = serde_json::from_value(msg_val)?;

            match msg {
                Message::Request(req) => {
                    // Validate method is in the allowlist
                    if !protocol::ALLOWED_METHODS.contains(&req.method.as_str()) {
                        let resp = Response::err(req.id, ErrorCode::METHOD_NOT_FOUND, format!("method not allowed: {}", req.method));
                        write_msg(&stdin, &serde_json::to_value(&resp)?).await?;
                        continue;
                    }

                    // Spawn a task to dispatch concurrently so the read loop is not blocked.
                    let stdin = Arc::clone(&stdin);
                    let fatal_error = Arc::clone(&fatal_error);

                    let in_flight = request_handles.iter().filter(|h| !h.is_finished()).count() + 1;
                    tracing::info!("Dispatching {} (id={}, in_flight={in_flight})", req.method, req.id);

                    request_handles.push(tokio::spawn(async move {
                        if let Some(ref params) = req.params {
                            let redacted = serde_json::to_string(&redact_secrets(params)).unwrap_or_default();
                            tracing::debug!("Request {} (id={}) params: {redacted}", req.method, req.id);
                        }

                        let start = std::time::Instant::now();
                        let resp = Self::dispatch_request(ctx, &req, output_declarations, scoped_data_dir).await;
                        let elapsed = start.elapsed();
                        let is_err = resp.error.is_some();
                        tracing::info!("Completed {} (id={}) in {elapsed:.1?}{}", req.method, req.id, if is_err { " [error]" } else { "" });

                        // If the response carries a fatal error, record it for the main loop
                        if let Some(ref err) = resp.error {
                            if err.fatal {
                                tracing::error!("Fatal error on {} (id={}): {}", req.method, req.id, err.message);
                                *fatal_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(err.message.clone());
                            }
                        }

                        let resp_val = serde_json::to_value(&resp).unwrap();

                        if let Some(ref result) = resp.result {
                            let redacted = serde_json::to_string(&redact_secrets(result)).unwrap_or_default();
                            tracing::debug!("Response {} (id={}) result: {redacted}", req.method, req.id);
                        }
                        if let Some(ref err) = resp.error {
                            tracing::debug!("Response {} (id={}) error: code={} msg={}", req.method, req.id, err.code, err.message);
                        }

                        let resp_bytes = serde_json::to_string(&resp_val).map(|s| s.len()).unwrap_or(0);
                        match write_msg(&stdin, &resp_val).await {
                            Ok(()) => tracing::info!("Wrote response for {} (id={}, {resp_bytes} bytes)", req.method, req.id),
                            Err(e) => tracing::error!("Failed to write response for {} (id={}): {e}", req.method, req.id),
                        }
                    }));
                }
                Message::Notification(notif) => match notif.method.as_str() {
                    "capability/result" => {
                        if let Some(params) = notif.params
                            && let Ok(result) = serde_json::from_value::<CapabilityResultParams>(params)
                        {
                            let mut progress = self.progress.write().unwrap_or_else(|e| e.into_inner());
                            progress.output = Some(result.output);
                            progress.phase = "done".into();
                        }
                        // Wait for any outstanding request tasks before shutting down.
                        for handle in &mut request_handles {
                            let _ = handle.await;
                        }
                        Self::shutdown(&stdin, &mut child).await;
                        break;
                    }
                    "capability/error" => {
                        if let Some(params) = notif.params
                            && let Ok(err_params) = serde_json::from_value::<CapabilityErrorParams>(params)
                        {
                            let mut progress = self.progress.write().unwrap_or_else(|e| e.into_inner());
                            progress.error = Some(err_params.message.clone());
                            if let Some(partial) = err_params.partial_output {
                                progress.output = Some(partial);
                            }
                        }
                        for handle in &mut request_handles {
                            let _ = handle.await;
                        }
                        Self::shutdown(&stdin, &mut child).await;
                        break;
                    }
                    "output/stream" => {
                        if let Some(params) = notif.params
                            && let Ok(stream) = serde_json::from_value::<OutputStreamParams>(params)
                        {
                            tracing::info!("[plugin:{}] stream: {}", self.manifest.name, stream.chunk.trim_end());
                        }
                    }
                    "progress" => {
                        if let Some(params) = notif.params
                            && let Ok(p) = serde_json::from_value::<ProgressParams>(params)
                        {
                            if let Some(ref tx) = ctx.progress_tx {
                                let _ = tx.send(ProgressEvent::Progress { pct: p.percent, phase: p.phase.clone() });
                            }
                            let mut progress = self.progress.write().unwrap_or_else(|e| e.into_inner());
                            match p.message {
                                Some(ref msg) => tracing::info!("[plugin:{}] progress: {} — {msg}", self.manifest.name, p.phase),
                                None => tracing::info!("[plugin:{}] progress: {}", self.manifest.name, p.phase),
                            }
                            progress.phase = p.phase;
                            progress.percent = p.percent;
                        }
                    }
                    "log" => {
                        if let Some(params) = notif.params
                            && let Ok(log) = serde_json::from_value::<LogParams>(params)
                        {
                            if let Some(ref tx) = ctx.progress_tx {
                                let _ = tx.send(ProgressEvent::Log { level: log.level.clone(), message: log.message.clone() });
                            }
                            match log.level.to_uppercase().as_str() {
                                "ERROR" => tracing::error!("[plugin:{}] {}", self.manifest.name, log.message),
                                "WARN" => tracing::warn!("[plugin:{}] {}", self.manifest.name, log.message),
                                "DEBUG" => tracing::debug!("[plugin:{}] {}", self.manifest.name, log.message),
                                _ => tracing::info!("[plugin:{}] {}", self.manifest.name, log.message),
                            }
                        }
                    }
                    _ => {
                        tracing::debug!("ignoring unknown notification: {}", notif.method);
                    }
                },
                Message::Response(_) => {
                    tracing::debug!("ignoring unexpected response from plugin");
                }
            }
        }

        // Extract result
        let progress = self.progress.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref error) = progress.error {
            if let Some(ref partial) = progress.output {
                tracing::warn!("[plugin:{}] error with partial output: {error}", self.manifest.name);
                return Ok(partial.clone());
            }
            anyhow::bail!("plugin error: {error}");
        }

        progress.output.clone().ok_or_else(|| anyhow::anyhow!("plugin exited without sending capability/result"))
    }

    async fn in_progress(&self) -> ProgressStatus {
        let progress = self.progress.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref output) = progress.output {
            return ProgressStatus::Done(output.clone());
        }
        ProgressStatus::StillBusy(progress.percent)
    }
}

/// Strip ANSI escape sequences (e.g. `\x1b[32m`) from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse a tracing-subscriber level prefix from a line (e.g. " INFO message" -> Some((INFO, "message"))).
/// Returns `None` for lines without a recognised prefix (continuation lines, raw output).
fn parse_plugin_level(line: &str) -> Option<(tracing::Level, &str)> {
    let trimmed = line.trim_start();
    let levels = [
        ("ERROR ", tracing::Level::ERROR),
        ("WARN ", tracing::Level::WARN),
        ("INFO ", tracing::Level::INFO),
        ("DEBUG ", tracing::Level::DEBUG),
        ("TRACE ", tracing::Level::TRACE),
    ];
    levels.into_iter().find_map(|(prefix, level)| trimmed.strip_prefix(prefix).map(|rest| (level, rest)))
}

/// Log a plugin message at the given tracing level.
fn log_plugin_msg(cap_name: &str, level: tracing::Level, msg: &str) {
    match level {
        l if l == tracing::Level::ERROR => tracing::error!("[plugin:{cap_name}] {msg}"),
        l if l == tracing::Level::WARN => tracing::warn!("[plugin:{cap_name}] {msg}"),
        l if l == tracing::Level::INFO => tracing::info!("[plugin:{cap_name}] {msg}"),
        l if l == tracing::Level::TRACE => tracing::trace!("[plugin:{cap_name}] {msg}"),
        _ => tracing::debug!("[plugin:{cap_name}] {msg}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corre_sdk::manifest::{OutputDeclaration, OutputType};

    #[test]
    fn output_permission_filesystem() {
        let decls = vec![OutputDeclaration {
            output_type: OutputType::Filesystem,
            target: "editions/{date}/edition.json".into(),
            content_type: None,
        }];
        assert!(is_output_permitted("editions/2026-02-26/edition.json", OutputType::Filesystem, &decls));
        assert!(!is_output_permitted("secrets/keys.json", OutputType::Filesystem, &decls));
    }

    #[test]
    fn output_permission_url() {
        let decls = vec![OutputDeclaration {
            output_type: OutputType::Rest,
            target: "http://localhost:5510/api/editions".into(),
            content_type: None,
        }];
        assert!(is_output_permitted("http://localhost:5510/api/editions", OutputType::Rest, &decls));
        assert!(!is_output_permitted("http://evil.com/steal", OutputType::Rest, &decls));
    }

    #[test]
    fn no_declarations_denies_all() {
        assert!(!is_output_permitted("anything", OutputType::Filesystem, &[]));
    }

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        assert_eq!(strip_ansi("\x1b[2m2026-03-02T08:48:03\x1b[0m \x1b[32m INFO\x1b[0m hello"), "2026-03-02T08:48:03  INFO hello");
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn redact_secrets_replaces_sensitive_keys() {
        let input = serde_json::json!({
            "api_key": "sk-1234",
            "model": "gpt-4",
            "nested": {
                "Authorization": "Bearer tok_abc",
                "data": "safe"
            },
            "items": [
                {"token": "secret123", "name": "test"},
                "plain string"
            ],
            "password": "hunter2"
        });
        let redacted = redact_secrets(&input);
        assert_eq!(redacted["api_key"], "[REDACTED]");
        assert_eq!(redacted["model"], "gpt-4");
        assert_eq!(redacted["nested"]["Authorization"], "[REDACTED]");
        assert_eq!(redacted["nested"]["data"], "safe");
        assert_eq!(redacted["items"][0]["token"], "[REDACTED]");
        assert_eq!(redacted["items"][0]["name"], "test");
        assert_eq!(redacted["items"][1], "plain string");
        assert_eq!(redacted["password"], "[REDACTED]");
    }

    #[test]
    fn redact_secrets_catches_bearer_and_sk_patterns() {
        let input = serde_json::json!({
            "value": "Bearer eyJhbGciOiJSUzI1NiJ9",
            "other": "sk-proj-abc123",
            "safe": "normal string"
        });
        let redacted = redact_secrets(&input);
        assert_eq!(redacted["value"], "[REDACTED]");
        assert_eq!(redacted["other"], "[REDACTED]");
        assert_eq!(redacted["safe"], "normal string");
    }

    #[test]
    fn parse_plugin_level_extracts_level_and_message() {
        let (level, msg) = parse_plugin_level(" INFO Got 5 results").unwrap();
        assert_eq!(level, tracing::Level::INFO);
        assert_eq!(msg, "Got 5 results");

        let (level, msg) = parse_plugin_level("ERROR something broke").unwrap();
        assert_eq!(level, tracing::Level::ERROR);
        assert_eq!(msg, "something broke");

        assert!(parse_plugin_level("just a plain line").is_none());
        assert!(parse_plugin_level("INFORMATION not a level").is_none());
    }
}
