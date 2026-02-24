use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level registry manifest fetched from a remote URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryManifest {
    pub version: u32,
    pub updated_at: String,
    pub servers: Vec<RegistryEntry>,
}

/// A single MCP server entry in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub install: InstallMethod,
    #[serde(default)]
    pub config: Vec<EnvVarSpec>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub verified: bool,
}

/// How to install the MCP server. The `method` field is the serde tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum InstallMethod {
    Npx {
        package: String,
        command: String,
        args: Vec<String>,
    },
    Pip {
        package: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Binary {
        download_url_template: String,
        binary_name: String,
        sha256: HashMap<String, String>,
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

/// Describes an environment variable required by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
}
