use crate::publish::Section;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

/// Metadata describing a capability's identity, schedule, and dependencies.
#[derive(Debug, Clone)]
pub struct CapabilityManifest {
    pub name: String,
    pub description: String,
    /// Cron expression with seconds field (e.g. "0 0 5 * * *" for 05:00 daily).
    pub schedule: String,
    /// Names of MCP servers this capability requires (references `[mcp.servers.*]` in config).
    pub mcp_servers: Vec<String>,
    /// Optional path to a user-editable config file (relative to project root).
    pub config_path: Option<String>,
}

/// Trait for calling tools on MCP servers, decoupling corre-core from corre-mcp.
#[async_trait::async_trait]
pub trait McpCaller: Send + Sync {
    async fn call_tool(&self, server_name: &str, tool_name: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
    async fn list_tools(&self, server_name: &str) -> anyhow::Result<Vec<String>>;
}

/// Trait for LLM completions, decoupling corre-core from corre-llm.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse>;
}

/// A simplified LLM request used by capabilities.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub messages: Vec<LlmMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub json_mode: bool,
}

#[derive(Debug, Clone)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: String,
}

#[derive(Debug, Clone)]
pub enum LlmRole {
    System,
    User,
    Assistant,
}

impl LlmRequest {
    pub fn simple(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            messages: vec![
                LlmMessage { role: LlmRole::System, content: system.into() },
                LlmMessage { role: LlmRole::User, content: user.into() },
            ],
            temperature: None,
            max_tokens: None,
            json_mode: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
}

/// Runtime context provided to a capability during execution.
pub struct CapabilityContext {
    pub mcp: Box<dyn McpCaller>,
    pub llm: Box<dyn LlmProvider>,
    pub config_dir: PathBuf,
    /// Maximum concurrent LLM requests (from `llm.max_concurrent` in config).
    pub max_concurrent_llm: usize,
}

/// The output produced by a capability after execution.
#[derive(Debug, Clone)]
pub struct CapabilityOutput {
    pub capability_name: String,
    pub produced_at: DateTime<Utc>,
    pub sections: Vec<Section>,
}

/// Trait implemented by each capability (daily brief, stock review, etc.).
#[async_trait::async_trait]
pub trait Capability: Send + Sync {
    fn manifest(&self) -> &CapabilityManifest;
    async fn execute(&self, ctx: &CapabilityContext) -> anyhow::Result<CapabilityOutput>;
}
