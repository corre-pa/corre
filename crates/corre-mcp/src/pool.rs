//! `McpPool`: a lazily-started, connection-caching pool of MCP server child processes.
//!
//! Each server is spawned on first use via `rmcp`'s `TokioChildProcess` transport and kept
//! alive until `McpPool::shutdown` is called. Implements `McpCaller` from `corre-core`.

use crate::server_def::McpServerDef;
use anyhow::Context;
use corre_core::capability::{McpCallError, McpCaller};
use rmcp::service::Peer;
use rmcp::{RoleClient, ServiceExt, model::CallToolRequestParam, transport::child_process::TokioChildProcess};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;

type RunningClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

/// Manages a pool of MCP server connections. Servers start lazily on first use
/// and are cached for the duration of the pool's lifetime.
#[derive(Clone)]
pub struct McpPool {
    definitions: HashMap<String, McpServerDef>,
    clients: Arc<Mutex<HashMap<String, RunningClient>>>,
}

impl McpPool {
    pub fn new(definitions: HashMap<String, McpServerDef>) -> Self {
        Self { definitions, clients: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Return a cloned `Peer` handle for the given server, starting it if needed.
    /// The `Peer` is `Clone` and safe to use outside the lock.
    ///
    /// The lock is held across the entire get-or-start operation to prevent a
    /// TOCTOU race where two callers both spawn the same server, orphaning the
    /// duplicate child process.
    async fn get_peer(&self, server_name: &str) -> anyhow::Result<Peer<RoleClient>> {
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(server_name) {
            return Ok(client.peer().clone());
        }
        let client = self.start_server(server_name).await?;
        let peer = client.peer().clone();
        clients.insert(server_name.to_string(), client);
        Ok(peer)
    }

    async fn start_server(&self, server_name: &str) -> anyhow::Result<RunningClient> {
        let def = self.definitions.get(server_name).with_context(|| format!("No MCP server definition for `{server_name}`"))?;

        tracing::info!("Starting MCP server `{server_name}`");

        let mut cmd = Command::new(&def.command);
        cmd.args(&def.args);

        // Clear all host env vars so MCP servers only see their declared secrets.
        cmd.env_clear();
        // Re-add minimal required vars for child process operation
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", home);
        }
        if let Ok(node_path) = std::env::var("NODE_PATH") {
            cmd.env("NODE_PATH", node_path);
        }
        // Add only the declared env vars for this server
        for (key, value) in &def.env {
            cmd.env(key, value);
        }

        let transport = TokioChildProcess::new(&mut cmd)
            .with_context(|| format!("failed to spawn MCP server `{server_name}` (command: `{} {}`)", def.command, def.args.join(" ")))?;
        let client = ().serve(transport).await.with_context(|| format!("failed to initialize MCP server `{server_name}`"))?;

        Ok(client)
    }

    /// Shutdown all running MCP servers.
    pub async fn shutdown(&self) {
        let mut clients = self.clients.lock().await;
        for (name, client) in clients.drain() {
            tracing::info!("Shutting down MCP server `{name}`");
            let _ = client.cancel().await;
        }
    }
}

#[async_trait::async_trait]
impl McpCaller for McpPool {
    async fn call_tool(&self, server_name: &str, tool_name: &str, args: serde_json::Value) -> Result<serde_json::Value, McpCallError> {
        let peer = self.get_peer(server_name).await?;

        let result = peer
            .call_tool(CallToolRequestParam {
                name: tool_name.to_string().into(),
                arguments: match args {
                    serde_json::Value::Object(map) => Some(map),
                    _ => None,
                },
            })
            .await
            .with_context(|| format!("Failed to call tool `{tool_name}` on MCP server `{server_name}`"))?;

        // Check if the tool itself reported an error
        if result.is_error == Some(true) {
            let message = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.as_str())).collect::<Vec<_>>().join("\n");
            return Err(McpCallError::ToolError { server: server_name.to_string(), tool: tool_name.to_string(), message });
        }

        // Each text content block may be an independent JSON object (e.g. brave-search
        // returns one JSON object per result). Collect all blocks that parse as JSON,
        // skipping non-JSON blocks (e.g. "Summarizer key: ..."). If no blocks parse as
        // JSON, fall back to a single concatenated string.
        let text_blocks: Vec<&str> = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.as_str())).collect();

        let json_values: Vec<serde_json::Value> = text_blocks.iter().filter_map(|t| serde_json::from_str(t).ok()).collect();

        if !json_values.is_empty() {
            if json_values.len() == 1 { Ok(json_values.into_iter().next().unwrap()) } else { Ok(serde_json::Value::Array(json_values)) }
        } else {
            // No blocks parsed as JSON — fall back to plain text
            let joined = text_blocks.join("\n");
            Ok(serde_json::Value::String(joined))
        }
    }

    async fn list_tools(&self, server_name: &str) -> Result<Vec<String>, McpCallError> {
        let peer = self.get_peer(server_name).await?;
        let tools = peer.list_all_tools().await.with_context(|| format!("Failed to list tools on MCP server `{server_name}`"))?;
        Ok(tools.iter().map(|t| t.name.to_string()).collect())
    }
}
