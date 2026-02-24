use corre_core::capability::McpCaller;
use corre_core::config::McpServerConfig;
use corre_mcp::{McpPool, McpServerDef};
use std::collections::HashMap;

/// Start a temporary MCP server, call `list_tools`, and return the tool names.
/// Shuts the server down afterward regardless of outcome.
/// Times out after 30 seconds to avoid indefinite hangs in the dashboard.
pub async fn test_mcp_server(name: &str, config: &McpServerConfig) -> Result<Vec<String>, String> {
    let def = McpServerDef::from_config(name, config);

    let mut defs = HashMap::new();
    defs.insert(name.to_string(), def);

    let pool = McpPool::new(defs);

    let result = match tokio::time::timeout(std::time::Duration::from_secs(30), pool.list_tools(name)).await {
        Ok(inner) => inner.map_err(|e| format!("{e:#}")),
        Err(_) => Err("Timed out after 30s waiting for list_tools response".to_string()),
    };

    pool.shutdown().await;

    result
}
