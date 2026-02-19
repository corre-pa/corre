use crate::server_def::McpServerDef;
use anyhow::Context;
use corre_core::capability::McpCaller;
use rmcp::{ServiceExt, model::CallToolRequestParam, transport::child_process::TokioChildProcess};
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

    async fn ensure_client(&self, server_name: &str) -> anyhow::Result<()> {
        let mut clients = self.clients.lock().await;
        if !clients.contains_key(server_name) {
            let client = self.start_server(server_name).await?;
            clients.insert(server_name.to_string(), client);
        }
        Ok(())
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
        self.ensure_client(server_name).await?;
        let clients = self.clients.lock().await;
        let client = clients.get(server_name).unwrap();

        let result = client
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
        // returns one JSON object per result). Collect them into an array if multiple
        // blocks parse as JSON; otherwise fall back to a single concatenated string.
        let text_blocks: Vec<&str> = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.as_str())).collect();

        let json_values: Vec<serde_json::Value> = text_blocks.iter().filter_map(|t| serde_json::from_str(t).ok()).collect();

        if json_values.len() == text_blocks.len() && !json_values.is_empty() {
            // All blocks parsed as JSON
            if json_values.len() == 1 {
                Ok(json_values.into_iter().next().unwrap())
            } else {
                Ok(serde_json::Value::Array(json_values))
            }
        } else {
            // Fall back to plain text
            let joined = text_blocks.join("\n");
            Ok(serde_json::Value::String(joined))
        }
    }

    async fn list_tools(&self, server_name: &str) -> anyhow::Result<Vec<String>> {
        self.ensure_client(server_name).await?;
        let clients = self.clients.lock().await;
        let client = clients.get(server_name).unwrap();

        let tools = client.list_tools(None).await?;
        Ok(tools.tools.iter().map(|t| t.name.to_string()).collect())
    }
}
