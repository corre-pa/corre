use corre_core::config::McpServerConfig;
use corre_core::secret::resolve_env_vars;
use std::collections::HashMap;

/// Runtime definition of an MCP server, ready to be spawned.
#[derive(Debug, Clone)]
pub struct McpServerDef {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl McpServerDef {
    pub fn from_config(name: &str, config: &McpServerConfig) -> Self {
        Self { name: name.to_string(), command: config.command.clone(), args: config.args.clone(), env: resolve_env_vars(&config.env) }
    }
}
