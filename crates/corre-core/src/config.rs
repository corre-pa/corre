//! Deserialization and persistence of `corre.toml` and per-MCP config files.
//!
//! `CorreConfig` is the top-level config struct. `CorreConfig::load` reads the TOML file and
//! parses it directly. Values that reference environment variables use `${VAR}` syntax and are
//! resolved at point of use via `corre_core::secret::resolve_value`.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CorreConfig {
    pub general: GeneralConfig,
    pub llm: LlmConfig,
    /// Raw `[news]` table — parsed into `corre_news::NewsConfig` by consumers.
    #[serde(default = "default_empty_table")]
    pub news: toml::Value,
    #[serde(default)]
    pub capabilities: Vec<CapabilityConfig>,
    #[serde(default)]
    pub safety: SafetyConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
}

/// Action to take when a policy rule matches.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    Warn,
    Sanitize,
    Block,
}

impl Default for PolicyAction {
    fn default() -> Self {
        Self::Sanitize
    }
}

/// Configuration for the prompt-injection defense layer.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SafetyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default = "default_true")]
    pub sanitize_injections: bool,
    #[serde(default = "default_true")]
    pub detect_leaks: bool,
    #[serde(default = "default_true")]
    pub boundary_wrap: bool,
    #[serde(default)]
    pub high_severity_action: PolicyAction,
    #[serde(default)]
    pub custom_block_patterns: Vec<String>,
    /// Require sandbox for plugins and MCP servers. When false (default),
    /// falls back to unsandboxed execution with a warning if Landlock is unavailable.
    #[serde(default)]
    pub require_sandbox: bool,
}

fn default_empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

fn default_max_output_bytes() -> usize {
    100_000
}

pub(crate) fn default_true() -> bool {
    true
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_output_bytes: default_max_output_bytes(),
            sanitize_injections: true,
            detect_leaks: true,
            boundary_wrap: true,
            high_severity_action: PolicyAction::Sanitize,
            custom_block_patterns: Vec::new(),
            require_sandbox: false,
        }
    }
}

/// Configuration for the MCP server registry.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RegistryConfig {
    #[serde(default = "default_registry_url")]
    pub url: String,
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Docker registry prefix for capability images (default: `ghcr.io/tree-corre`).
    #[serde(default = "default_docker_registry")]
    pub docker_registry: String,
}

fn default_registry_url() -> String {
    "http://localhost:5580".to_string()
}

fn default_cache_ttl_secs() -> u64 {
    3600
}

fn default_docker_registry() -> String {
    "ghcr.io/tree-corre".to_string()
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self { url: default_registry_url(), cache_ttl_secs: default_cache_ttl_secs(), docker_registry: default_docker_registry() }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GeneralConfig {
    pub data_dir: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_log_level() -> String {
    "info".into()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Maximum concurrent requests to the LLM API. Set this based on your
    /// provider's rate limits (e.g. Venice.ai M-tier = 50 req/min).
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_temperature() -> f32 {
    0.3
}

fn default_max_concurrent() -> usize {
    10
}

/// Per-capability LLM overrides. Every field is optional — only specified
/// fields replace the corresponding global `[llm]` value.
///
/// Capabilities must not control provider URL or credentials — those are
/// host-level concerns. Only model selection and generation parameters
/// can be overridden.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CapabilityLlmConfig {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    pub max_concurrent: Option<usize>,
}

impl LlmConfig {
    /// Return a new `LlmConfig` where any `Some` field in `overrides` replaces
    /// the corresponding field in `self`. Connection params (provider, base_url,
    /// api_key) are always inherited from the host config.
    pub fn with_overrides(&self, overrides: &CapabilityLlmConfig) -> LlmConfig {
        LlmConfig {
            provider: self.provider.clone(),
            base_url: self.base_url.clone(),
            model: overrides.model.clone().unwrap_or_else(|| self.model.clone()),
            api_key: self.api_key.clone(),
            temperature: overrides.temperature.unwrap_or(self.temperature),
            max_concurrent: overrides.max_concurrent.unwrap_or(self.max_concurrent),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CapabilityConfig {
    pub name: String,
    pub description: String,
    pub schedule: String,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    pub config_path: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub llm: Option<CapabilityLlmConfig>,
    /// Path to a plugin directory (relative to data_dir/plugins/). When set, this
    /// capability is backed by a subprocess plugin instead of a built-in implementation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// Per-capability log level override (e.g. "debug", "info"). Falls back to
    /// `[general] log_level` when not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
}

impl CorreConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&raw)?;

        // Allow env var overrides for Docker — avoids sed-patching config files.
        if let Ok(dir) = std::env::var("CORRE_DATA_DIR") {
            config.general.data_dir = dir;
        }
        if let Ok(bind) = std::env::var("CORRE_NEWS_BIND") {
            // Patch the raw `[news]` table directly
            if let Some(table) = config.news.as_table_mut() {
                table.insert("bind".into(), toml::Value::String(bind));
            }
        }
        if let Ok(url) = std::env::var("CORRE_REGISTRY_URL") {
            config.registry.url = url;
        }

        Ok(config)
    }

    /// Serialize this config to TOML and write it to the given path.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, &content).with_context(|| format!("failed to write config to {}", path.display()))?;
        Ok(())
    }

    /// Returns the resolved data directory, expanding `~` to the user's home.
    pub fn data_dir(&self) -> PathBuf {
        expand_tilde(&self.general.data_dir)
    }

    /// Returns the path to the per-MCP config directory.
    pub fn mcp_dir(&self) -> PathBuf {
        self.data_dir().join("config").join("mcp")
    }

    /// Load per-MCP config files where `installed = true` and resolve bare
    /// command names against `{data_dir}/bin/`.
    pub fn resolved_mcp_servers(&self) -> anyhow::Result<HashMap<String, McpServerConfig>> {
        let mcp_dir = self.mcp_dir();
        let bin_dir = self.data_dir().join("bin");
        let file_configs = load_mcp_configs(&mcp_dir)?;
        Ok(file_configs
            .into_iter()
            .filter(|(_, cfg)| cfg.installed)
            .map(|(name, cfg)| {
                let resolved = cfg.with_resolved_command(Some(&bin_dir));
                (name, resolved)
            })
            .collect())
    }
}

pub(crate) fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

// =========================================================================
// Per-MCP config files ({data_dir}/config/mcp/{name}.toml)
// =========================================================================

/// Configuration for a single MCP server stored as its own file.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpServerConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_id: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Config files persist after removal so settings are preserved.
    /// This flag tracks whether the server is actually installed.
    #[serde(default)]
    pub installed: bool,
}

impl McpServerConfig {
    /// Return a copy with bare command names resolved against `bin_dir`.
    /// When `bin_dir` is provided, bare command names (no path separator) are
    /// resolved to `{bin_dir}/{command}` if the file exists there.
    pub fn with_resolved_command(&self, bin_dir: Option<&Path>) -> McpServerConfig {
        let command = if let Some(dir) = bin_dir {
            if !self.command.contains(std::path::MAIN_SEPARATOR) && !self.command.contains('/') {
                let candidate = dir.join(&self.command);
                if candidate.exists() { candidate.to_string_lossy().into_owned() } else { self.command.clone() }
            } else {
                self.command.clone()
            }
        } else {
            self.command.clone()
        };
        McpServerConfig {
            command,
            args: self.args.clone(),
            env: self.env.clone(),
            registry_id: self.registry_id.clone(),
            installed: self.installed,
        }
    }
}

/// Scan `{mcp_dir}/*.toml`, interpolate env vars, and parse each file.
/// Keys are the file stem (e.g. `brave-search` from `brave-search.toml`).
pub fn load_mcp_configs(mcp_dir: &Path) -> anyhow::Result<HashMap<String, McpServerConfig>> {
    let mut configs = HashMap::new();
    let entries = match std::fs::read_dir(mcp_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(configs),
        Err(e) => return Err(e).context("reading MCP config directory"),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let raw = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let interpolated = crate::secret::interpolate_env_vars(&raw);
            let cfg: McpServerConfig = toml::from_str(&interpolated).with_context(|| format!("parsing {}", path.display()))?;
            configs.insert(name, cfg);
        }
    }
    Ok(configs)
}

/// Load a single MCP config file *without* env var interpolation, so the
/// caller can see raw `${VAR}` references (used by the configure modal).
pub fn load_mcp_config_raw(mcp_dir: &Path, name: &str) -> anyhow::Result<McpServerConfig> {
    let path = mcp_dir.join(format!("{name}.toml"));
    let raw = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: McpServerConfig = toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(cfg)
}

/// Serialize and write a per-MCP config file.
pub fn save_mcp_config(mcp_dir: &Path, name: &str, config: &McpServerConfig) -> anyhow::Result<()> {
    std::fs::create_dir_all(mcp_dir).with_context(|| format!("creating {}", mcp_dir.display()))?;
    let path = mcp_dir.join(format!("{name}.toml"));
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_config() {
        let toml_str = include_str!("../../../corre.toml");
        let config: CorreConfig = toml::from_str(toml_str).expect("Failed to parse config");
        // NewsConfig is parsed by corre-news, here we just check the raw table
        let news_table = config.news.as_table().expect("[news] should be a table");
        assert_eq!(news_table.get("bind").and_then(|v| v.as_str()), Some("192.168.1.101:5510"));
        assert_eq!(config.llm.model, "zai-org-glm-4.7-flash");
        assert_eq!(config.capabilities.len(), 1);
        assert_eq!(config.capabilities[0].name, "daily-brief");
    }

    #[test]
    fn parse_capability_llm_overrides() {
        let toml_str = r#"
            [general]
            data_dir = "/tmp/corre"
            [llm]
            provider = "openai-compatible"
            base_url = "https://api.example.com/v1"
            model = "base-model"
            api_key = "BASE_KEY"
            [news]
            bind = "127.0.0.1:5510"
            [[capabilities]]
            name = "test-cap"
            description = "test"
            schedule = "0 * * * * *"
            [capabilities.llm]
            model = "override-model"
            temperature = 0.9
        "#;
        let config: CorreConfig = toml::from_str(toml_str).expect("Failed to parse config with capability LLM overrides");
        let cap = &config.capabilities[0];
        let overrides = cap.llm.as_ref().expect("capability llm overrides should be present");
        assert_eq!(overrides.model.as_deref(), Some("override-model"));
        assert_eq!(overrides.temperature, Some(0.9));

        let effective = config.llm.with_overrides(overrides);
        assert_eq!(effective.model, "override-model");
        assert_eq!(effective.temperature, 0.9);
        // Connection params are always inherited from global config
        assert_eq!(effective.base_url, "https://api.example.com/v1");
        assert_eq!(effective.api_key, "BASE_KEY");
    }

    #[test]
    fn expand_tilde_works() {
        let expanded = expand_tilde("~/.local/share/corre");
        assert!(!expanded.to_string_lossy().starts_with('~'));
    }
}
