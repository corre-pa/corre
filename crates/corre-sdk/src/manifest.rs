//! Plugin manifest (`manifest.toml`) types.

use crate::types::ContentType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level manifest file for a capability plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub mcp_dependencies: HashMap<String, McpDependency>,
}

/// Core plugin metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub min_host_version: Option<String>,
    #[serde(default = "default_protocol_version")]
    pub protocol_version: String,
    /// Name of the executable inside `bin/`. Defaults to `"capability"` for
    /// backward compatibility, but registry-installed plugins use their own name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_name: Option<String>,
    #[serde(default)]
    pub content_type: ContentType,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub defaults: PluginDefaults,
    #[serde(default)]
    pub permissions: PluginPermissions,
    #[serde(default)]
    pub services: Vec<ServiceDeclaration>,
    #[serde(default)]
    pub links: Vec<PluginLink>,
}

/// A link entry for the dashboard start menu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginLink {
    pub label: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Default configuration values that the host uses if not overridden.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginDefaults {
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub config_path: Option<String>,
    #[serde(default)]
    pub config_schema: Option<ConfigSchema>,
}

/// Schema describing the structure of a capability's config file, used by the
/// dashboard to render a form dynamically.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigSchema {
    /// Top-level key in the config file (e.g. "daily-briefing").
    #[serde(default)]
    pub root_key: Option<String>,
    /// File format. Defaults to "yaml".
    #[serde(default = "default_yaml")]
    pub format: String,
    /// Field descriptors.
    #[serde(default)]
    pub fields: Vec<ConfigField>,
}

/// A single field descriptor within a [`ConfigSchema`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    pub key: String,
    #[serde(rename = "type")]
    pub field_type: ConfigFieldType,
    #[serde(default)]
    pub label: Option<String>,
    /// For "select" fields: allowed values.
    #[serde(default)]
    pub options: Vec<String>,
    /// Default value for new entries.
    #[serde(default)]
    pub default: Option<String>,
    /// For "list" fields: the schema of each list item.
    #[serde(default)]
    pub fields: Vec<ConfigField>,
}

/// The type of a config field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigFieldType {
    Text,
    Textarea,
    Select,
    /// Comma-separated list stored as a YAML array.
    TextList,
    /// Repeatable group of sub-fields.
    List,
}

fn default_yaml() -> String {
    "yaml".into()
}

/// Permissions the plugin requests from the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPermissions {
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

impl Default for PluginPermissions {
    fn default() -> Self {
        Self { mcp_servers: Vec::new(), llm_access: true, max_concurrent_llm: 10, outputs: Vec::new(), sandbox: None }
    }
}

/// Declares an output destination that the capability is permitted to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputDeclaration {
    pub output_type: OutputType,
    /// Template path/URL. Supports `{date}`, `{data_dir}`, `{config_dir}`.
    pub target: String,
    /// Optional content type hint (e.g. "application/json", "text/html").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// The type of output destination.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputType {
    Filesystem,
    Stream,
    Rest,
    Webhook,
}

/// Sandbox permissions governing filesystem and network access for a plugin.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxPermissions {
    /// Allowed network destinations (host:port or CIDR).
    #[serde(default)]
    pub network: Vec<String>,
    /// Read-only filesystem paths (supports `{data_dir}`, `{config_dir}` templates).
    #[serde(default)]
    pub filesystem_read: Vec<String>,
    /// Read-write filesystem paths.
    #[serde(default)]
    pub filesystem_write: Vec<String>,
    /// Whether the sandbox has DNS resolution (default: true if network is non-empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns: Option<bool>,
    /// Max memory in MB (0 = unlimited).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u64>,
    /// Max CPU time in seconds (0 = unlimited).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cpu_secs: Option<u64>,
}

/// A service that the capability depends on or bundles (e.g. a web UI container).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDeclaration {
    pub name: String,
    pub description: String,
    /// Docker image.
    pub image: String,
    /// Port mappings ("host:container").
    #[serde(default)]
    pub ports: Vec<String>,
    /// Volume mounts ("{data_dir}/editions:/data/editions:ro").
    #[serde(default)]
    pub volumes: Vec<String>,
    /// Environment variable mappings.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether the user can decline this service.
    #[serde(default)]
    pub optional: bool,
    /// Endpoint to poll for health checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<String>,
}

/// Execution mode for a capability.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Run once, produce output, exit (current model).
    #[default]
    Oneshot,
    /// Long-running: no timeout, restart-on-crash, continuous output.
    Daemon,
}

/// MCP server dependency declared by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDependency {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_protocol_version() -> String {
    "1.0".into()
}

fn default_true() -> bool {
    true
}

fn default_max_concurrent_llm() -> usize {
    10
}

impl PluginManifest {
    /// Load a manifest from a TOML file.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let manifest: Self = toml::from_str(&content)?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml_str = r#"
            [plugin]
            name = "daily-brief"
            version = "1.0.0"
            description = "Daily research brief"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.plugin.name, "daily-brief");
        assert_eq!(manifest.plugin.protocol_version, "1.0");
        assert!(manifest.plugin.permissions.llm_access);
        assert_eq!(manifest.plugin.permissions.max_concurrent_llm, 10);
    }

    #[test]
    fn parse_full_manifest() {
        let toml_str = r#"
            [plugin]
            name = "daily-brief"
            version = "1.2.0"
            description = "Daily research brief from web searches"
            min_host_version = "2.0.0"
            protocol_version = "1.0"
            content_type = "newspaper"

            [plugin.defaults]
            schedule = "0 0 5 * * *"
            config_path = "config/topics.yml"

            [plugin.permissions]
            mcp_servers = ["brave-search"]
            llm_access = true
            max_concurrent_llm = 10

            [mcp_dependencies.brave-search]
            command = "npx"
            args = ["-y", "@brave/brave-search-mcp-server"]
            env = { BRAVE_API_KEY = "$BRAVE_API_KEY" }
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.plugin.name, "daily-brief");
        assert_eq!(manifest.plugin.version, "1.2.0");
        assert_eq!(manifest.plugin.permissions.mcp_servers, vec!["brave-search"]);
        assert!(manifest.mcp_dependencies.contains_key("brave-search"));
    }

    #[test]
    fn parse_custom_content_type() {
        let toml_str = r#"
            [plugin]
            name = "physics-course"
            version = "0.1.0"
            description = "Interactive physics course"
            content_type = "custom"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.plugin.content_type, ContentType::Custom);
    }

    #[test]
    fn parse_config_schema_round_trip() {
        let toml_str = r#"
            [plugin]
            name = "daily-brief"
            version = "1.0.0"
            description = "Brief"

            [plugin.defaults]
            schedule = "0 0 5 * * *"
            config_path = "config/topics.yml"

            [plugin.defaults.config_schema]
            root_key = "daily-briefing"
            format = "yaml"

            [[plugin.defaults.config_schema.fields]]
            key = "sections"
            type = "list"
            label = "Sections"

            [[plugin.defaults.config_schema.fields.fields]]
            key = "title"
            type = "text"
            label = "Section title"

            [[plugin.defaults.config_schema.fields.fields]]
            key = "sources"
            type = "list"
            label = "Sources"

            [[plugin.defaults.config_schema.fields.fields.fields]]
            key = "search"
            type = "text"
            label = "Search query"

            [[plugin.defaults.config_schema.fields.fields.fields]]
            key = "include"
            type = "text-list"
            label = "Include terms"

            [[plugin.defaults.config_schema.fields.fields.fields]]
            key = "freshness"
            type = "select"
            label = "Freshness"
            options = ["1d", "1w", "1m"]
            default = "1d"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        let schema = manifest.plugin.defaults.config_schema.as_ref().expect("config_schema should be present");
        assert_eq!(schema.root_key.as_deref(), Some("daily-briefing"));
        assert_eq!(schema.format, "yaml");
        assert_eq!(schema.fields.len(), 1);

        let sections_field = &schema.fields[0];
        assert_eq!(sections_field.key, "sections");
        assert_eq!(sections_field.field_type, ConfigFieldType::List);
        assert_eq!(sections_field.fields.len(), 2); // title + sources

        let title_field = &sections_field.fields[0];
        assert_eq!(title_field.key, "title");
        assert_eq!(title_field.field_type, ConfigFieldType::Text);

        let sources_field = &sections_field.fields[1];
        assert_eq!(sources_field.key, "sources");
        assert_eq!(sources_field.field_type, ConfigFieldType::List);
        assert_eq!(sources_field.fields.len(), 3); // search, include, freshness

        let freshness = &sources_field.fields[2];
        assert_eq!(freshness.field_type, ConfigFieldType::Select);
        assert_eq!(freshness.options, vec!["1d", "1w", "1m"]);
        assert_eq!(freshness.default.as_deref(), Some("1d"));

        // Round-trip: serialize back to JSON and re-parse
        let json = serde_json::to_string(&schema).unwrap();
        let reparsed: ConfigSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(reparsed.root_key, schema.root_key);
        assert_eq!(reparsed.fields.len(), schema.fields.len());
    }

    #[test]
    fn config_schema_defaults_when_absent() {
        let toml_str = r#"
            [plugin]
            name = "test"
            version = "1.0.0"
            description = "No schema"

            [plugin.defaults]
            schedule = "0 0 5 * * *"
        "#;
        let manifest: PluginManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.plugin.defaults.config_schema.is_none());
    }
}
