//! CCPP v1 JSON-RPC 2.0 message types.

use crate::types::CapabilityOutput;
use serde::{Deserialize, Serialize};

// ── Error codes ──────────────────────────────────────────────────────────

/// Protocol error codes (CCPP v1).
pub struct ErrorCode;

impl ErrorCode {
    // Standard JSON-RPC errors
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;

    // CCPP-specific errors
    pub const INIT_FAILED: i64 = -32001;
    pub const VERSION_MISMATCH: i64 = -32002;
    pub const MCP_TOOL_ERROR: i64 = -32010;
    pub const MCP_SERVER_DENIED: i64 = -32011;
    pub const MCP_SERVER_UNAVAILABLE: i64 = -32012;
    pub const LLM_RATE_LIMITED: i64 = -32020;
    pub const LLM_TRUNCATED: i64 = -32021;
    pub const LLM_PROVIDER_ERROR: i64 = -32022;
    pub const SAFETY_BLOCKED: i64 = -32030;
}

/// Current protocol version string.
pub const PROTOCOL_VERSION: &str = "1.0";

/// Maximum message size in bytes (10 MB).
pub const MAX_MESSAGE_BYTES: usize = 10 * 1024 * 1024;

// ── Envelope ─────────────────────────────────────────────────────────────

/// A JSON-RPC 2.0 message — request, response, or notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ── Method parameter/result structs ──────────────────────────────────────

/// `initialize` (Host → Capability) params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capability_name: String,
    pub config_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(default)]
    pub seen_urls: Vec<String>,
    #[serde(default = "default_max_concurrent_llm")]
    pub max_concurrent_llm: usize,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_max_concurrent_llm() -> usize {
    10
}

fn default_timeout_secs() -> u64 {
    600
}

/// `initialize` response result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: serde_json::Value,
}

/// `mcp/callTool` (Capability → Host) params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallToolParams {
    pub server_name: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// `mcp/listTools` (Capability → Host) params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpListToolsParams {
    pub server_name: String,
}

/// `progress` notification params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressParams {
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `log` notification params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogParams {
    pub level: String,
    pub message: String,
}

/// `capability/result` notification params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResultParams {
    pub output: CapabilityOutput,
}

/// `capability/error` notification params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityErrorParams {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<CapabilityOutput>,
}

// ── Output method params ─────────────────────────────────────────────────

/// `output/write` (Capability → Host) params: write a file to a permitted path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputWriteParams {
    /// Relative path under `data_dir` (e.g. "editions/2026-02-26/edition.json").
    pub path: String,
    /// File contents (base64 for binary, UTF-8 string for text).
    pub data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// `output/stream` (Capability → Host, Notification): stream text chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputStreamParams {
    pub chunk: String,
    #[serde(default)]
    pub r#final: bool,
}

/// `output/rest` (Capability → Host) params: POST to a permitted REST endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputRestParams {
    pub url: String,
    pub body: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// `output/webhook` (Capability → Host) params: fire a webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputWebhookParams {
    pub url: String,
    pub body: serde_json::Value,
}

// ── Error codes for output operations ────────────────────────────────────

impl ErrorCode {
    pub const OUTPUT_DENIED: i64 = -32040;
    pub const OUTPUT_FAILED: i64 = -32041;
}

// ── Helpers ──────────────────────────────────────────────────────────────

impl Request {
    pub fn new(id: u64, method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self { jsonrpc: "2.0".into(), id, method: method.into(), params }
    }
}

impl Response {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }

    pub fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: None, error: Some(RpcError { code, message: message.into(), data: None }) }
    }

    pub fn err_with_data(id: u64, code: i64, message: impl Into<String>, data: serde_json::Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: None, error: Some(RpcError { code, message: message.into(), data: Some(data) }) }
    }
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self { jsonrpc: "2.0".into(), method: method.into(), params }
    }
}

/// Known CCPP method names for whitelist validation.
pub const ALLOWED_METHODS: &[&str] = &[
    "initialize",
    "mcp/callTool",
    "mcp/listTools",
    "llm/complete",
    "progress",
    "log",
    "capability/result",
    "capability/error",
    "shutdown",
    "cancel",
    // Output methods (v2)
    "output/write",
    "output/stream",
    "output/rest",
    "output/webhook",
    // Daemon-mode methods (v2, dispatch deferred)
    "heartbeat",
    "output/emit",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let req = Request::new(42, "mcp/callTool", Some(serde_json::json!({"server_name": "brave-search"})));
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.method, "mcp/callTool");
    }

    #[test]
    fn response_ok_round_trip() {
        let resp = Response::ok(1, serde_json::json!({"protocol_version": "1.0"}));
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&json).unwrap();
        assert!(parsed.result.is_some());
        assert!(parsed.error.is_none());
    }

    #[test]
    fn response_err_round_trip() {
        let resp = Response::err(1, ErrorCode::MCP_SERVER_DENIED, "not allowed");
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&json).unwrap();
        assert!(parsed.error.is_some());
        assert_eq!(parsed.error.unwrap().code, -32011);
    }

    #[test]
    fn notification_round_trip() {
        let notif = Notification::new("progress", Some(serde_json::json!({"phase": "scoring", "percent": 45})));
        let json = serde_json::to_string(&notif).unwrap();
        let parsed: Notification = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.method, "progress");
    }

    #[test]
    fn initialize_params_round_trip() {
        let params = InitializeParams {
            protocol_version: "1.0".into(),
            capability_name: "daily-brief".into(),
            config_dir: "/home/user/.local/share/corre".into(),
            config_path: Some("config/topics.yml".into()),
            seen_urls: vec!["https://example.com".into()],
            max_concurrent_llm: 10,
            mcp_servers: vec!["brave-search".into()],
            timeout_secs: 600,
        };
        let json = serde_json::to_value(&params).unwrap();
        let parsed: InitializeParams = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.capability_name, "daily-brief");
    }
}
