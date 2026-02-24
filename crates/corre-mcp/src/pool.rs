use crate::server_def::McpServerDef;
use anyhow::Context;
use corre_core::capability::McpCaller;
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
    async fn get_peer(&self, server_name: &str) -> anyhow::Result<Peer<RoleClient>> {
        // Fast path: server already running
        {
            let clients = self.clients.lock().await;
            if let Some(client) = clients.get(server_name) {
                return Ok(client.peer().clone());
            }
        }

        // Slow path: start the server without holding the lock
        let client = self.start_server(server_name).await?;
        let peer = client.peer().clone();

        let mut clients = self.clients.lock().await;
        // Another task may have started the same server concurrently
        if !clients.contains_key(server_name) {
            clients.insert(server_name.to_string(), client);
        }

        Ok(peer)
    }

    async fn start_server(&self, server_name: &str) -> anyhow::Result<RunningClient> {
        let def = self.definitions.get(server_name).with_context(|| format!("No MCP server definition for `{server_name}`"))?;

        tracing::info!("Starting MCP server `{server_name}`");

        let mut cmd = Command::new(&def.command);
        cmd.args(&def.args);
        for (key, value) in &def.env {
            cmd.env(key, value);
        }

        let transport = TokioChildProcess::new(&mut cmd)?;
        let client = ().serve(transport).await.with_context(|| format!("Failed to start MCP server `{server_name}`"))?;

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
    async fn call_tool(&self, server_name: &str, tool_name: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
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

    async fn list_tools(&self, server_name: &str) -> anyhow::Result<Vec<String>> {
        let peer = self.get_peer(server_name).await?;
        let tools = peer.list_all_tools().await?;
        Ok(tools.iter().map(|t| t.name.to_string()).collect())
    }
}
