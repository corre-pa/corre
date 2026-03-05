//! Serde-serialisable types that mirror the registry JSON manifest format.
//!
//! [`RegistryManifest`] is the top-level document (V1 or V2). V1 contains only MCP servers.
//! V2 extends it with app entries. The [`McpRegistryEntry`] type describes an MCP server
//! with its [`InstallMethod`] and required [`EnvVarSpec`] entries. [`AppEntry`] describes
//! an app with its inline manifest, install method, and metadata.

use corre_sdk::manifest::{ExecutionMode, OutputDeclaration, PluginLink, SandboxPermissions, ServiceDeclaration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level registry manifest fetched from a remote URL.
///
/// Supports both V1 (servers only) and V2 (servers + apps) formats.
/// When deserializing a V1 manifest, `apps` defaults to an empty vec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryManifest {
    pub version: u32,
    pub updated_at: String,
    pub servers: Vec<McpRegistryEntry>,
    #[serde(default, alias = "capabilities")]
    pub apps: Vec<AppEntry>,
}

/// A single MCP server entry in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub install: InstallMethod,
    #[serde(default)]
    pub config: Vec<EnvVarSpec>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub verified: bool,
}

/// An app entry in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default = "default_protocol_version")]
    pub protocol_version: String,
    pub install: InstallMethod,
    pub manifest: AppManifestInline,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub verified: bool,
}

fn default_protocol_version() -> String {
    "1.0".into()
}

/// Inline manifest data embedded in an app registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifestInline {
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub defaults: AppDefaults,
    #[serde(default)]
    pub permissions: AppPermissions,
    /// References to MCP server IDs in the same registry.
    #[serde(default)]
    pub mcp_dependencies: Vec<String>,
    #[serde(default)]
    pub services: Vec<ServiceDeclaration>,
    #[serde(default)]
    pub links: Vec<PluginLink>,
}

/// Default config values for a registry-hosted app.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_schema: Option<corre_sdk::manifest::ConfigSchema>,
}

/// Extended permissions for a registry-hosted app.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppPermissions {
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default = "default_true")]
    pub llm_access: bool,
    #[serde(default = "default_max_concurrent_llm")]
    pub max_concurrent_llm: usize,
    #[serde(default)]
    pub outputs: Vec<OutputDeclaration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxPermissions>,
}

fn default_true() -> bool {
    true
}

fn default_max_concurrent_llm() -> usize {
    10
}

/// How to install an MCP server or app binary. The `method` field is the serde tag.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_manifest_parses_without_apps() {
        let json = r#"{
            "version": 1,
            "updated_at": "2026-02-26T00:00:00Z",
            "servers": []
        }"#;
        let manifest: RegistryManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.version, 1);
        assert!(manifest.apps.is_empty());
    }

    #[test]
    fn v2_manifest_parses_with_apps() {
        let json = r#"{
            "version": 2,
            "updated_at": "2026-02-26T00:00:00Z",
            "servers": [],
            "apps": [{
                "id": "daily-brief",
                "name": "Daily Brief",
                "description": "Web search + LLM scoring",
                "version": "1.0.0",
                "install": {
                    "method": "binary",
                    "download_url_template": "https://example.com/{id}",
                    "binary_name": "daily-brief",
                    "sha256": {},
                    "command": "daily-brief"
                },
                "manifest": {
                    "content_type": "newspaper",
                    "mcp_dependencies": ["brave-search"]
                }
            }]
        }"#;
        let manifest: RegistryManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.version, 2);
        assert_eq!(manifest.apps.len(), 1);
        assert_eq!(manifest.apps[0].id, "daily-brief");
        assert_eq!(manifest.apps[0].manifest.mcp_dependencies, vec!["brave-search"]);
    }
}
