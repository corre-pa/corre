//! `McpServerDef`: runtime-ready MCP server description used to spawn child processes.
//!
//! Bridges the TOML-deserialized `McpServerConfig` from `corre-core::config` and the
//! process-spawning logic in `crate::pool`.

use corre_core::config::McpServerConfig;
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
    pub fn from_config(name: impl Into<String>, config: &McpServerConfig) -> Self {
        Self { name: name.into(), command: config.command.clone(), args: config.args.clone(), env: config.env.clone() }
    }
}
