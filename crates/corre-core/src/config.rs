use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CorreConfig {
    pub general: GeneralConfig,
    pub llm: LlmConfig,
    pub news: NewsConfig,
    #[serde(default, skip_serializing_if = "McpConfig::is_empty")]
    pub mcp: McpConfig,
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
}

fn default_max_output_bytes() -> usize {
    100_000
}

fn default_true() -> bool {
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
        }
    }
}

impl SafetyConfig {
    /// Convenience constructor for an enabled config with sensible defaults.
    pub fn default_enabled() -> Self {
        Self { enabled: true, ..Default::default() }
    }
}

/// Configuration for the MCP server registry.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RegistryConfig {
    #[serde(default = "default_registry_url")]
    pub url: String,
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
}

fn default_registry_url() -> String {
    "http://localhost:5580".to_string()
}

fn default_cache_ttl_secs() -> u64 {
    3600
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self { url: default_registry_url(), cache_ttl_secs: default_cache_ttl_secs() }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GeneralConfig {
    pub data_dir: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_mcp_repository")]
    pub mcp_repository: String,
}

fn default_log_level() -> String {
    "info".into()
}

pub fn default_mcp_repository() -> String {
    "http://localhost:5580".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
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
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CapabilityLlmConfig {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    pub max_concurrent: Option<usize>,
}

impl LlmConfig {
    /// Return a new `LlmConfig` where any `Some` field in `overrides` replaces
    /// the corresponding field in `self`.
    pub fn with_overrides(&self, overrides: &CapabilityLlmConfig) -> LlmConfig {
        LlmConfig {
            provider: overrides.provider.clone().unwrap_or_else(|| self.provider.clone()),
            base_url: overrides.base_url.clone().unwrap_or_else(|| self.base_url.clone()),
            model: overrides.model.clone().unwrap_or_else(|| self.model.clone()),
            api_key_env: overrides.api_key_env.clone().unwrap_or_else(|| self.api_key_env.clone()),
            temperature: overrides.temperature.unwrap_or(self.temperature),
            max_concurrent: overrides.max_concurrent.unwrap_or(self.max_concurrent),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

impl McpConfig {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// If this server was installed from the registry, tracks the registry entry ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CapabilityConfig {
    pub name: String,
    pub description: String,
    pub schedule: String,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    pub config_path: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub llm: Option<CapabilityLlmConfig>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct NewsConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_token: Option<String>,
}

fn default_bind() -> String {
    "127.0.0.1:3200".into()
}

fn default_title() -> String {
    "Corre News".into()
}

impl CorreConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let content = crate::secret::interpolate_env_vars(&raw);
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Serialize this config to TOML and write it to the given path.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
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

    /// Load per-MCP config files where `installed = true` and return them as
    /// the legacy `McpServerConfig` map that downstream consumers expect.
    /// Bare command names are resolved against `{data_dir}/bin/`.
    pub fn resolved_mcp_servers(&self) -> anyhow::Result<HashMap<String, McpServerConfig>> {
        let mcp_dir = self.mcp_dir();
        let bin_dir = self.data_dir().join("bin");
        let file_configs = load_mcp_configs(&mcp_dir)?;
        Ok(file_configs
            .into_iter()
            .filter(|(_, cfg)| cfg.installed)
            .map(|(name, cfg)| {
                let server_cfg = cfg.to_server_config_with_bin_dir(Some(&bin_dir));
                (name, server_cfg)
            })
            .collect())
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
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
pub struct McpServerFileConfig {
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

impl McpServerFileConfig {
    /// Convert to the legacy `McpServerConfig` used by downstream consumers.
    /// When `bin_dir` is provided, bare command names (no path separator) are
    /// resolved to `{bin_dir}/{command}` if the file exists there.
    pub fn to_server_config_with_bin_dir(&self, bin_dir: Option<&Path>) -> McpServerConfig {
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
        McpServerConfig { command, args: self.args.clone(), env: self.env.clone(), registry_id: self.registry_id.clone() }
    }

    /// Convert to the legacy `McpServerConfig` without resolving bare commands.
    pub fn to_server_config(&self) -> McpServerConfig {
        self.to_server_config_with_bin_dir(None)
    }
}

/// Scan `{mcp_dir}/*.toml`, interpolate env vars, and parse each file.
/// Keys are the file stem (e.g. `brave-search` from `brave-search.toml`).
pub fn load_mcp_configs(mcp_dir: &Path) -> anyhow::Result<HashMap<String, McpServerFileConfig>> {
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
            let cfg: McpServerFileConfig = toml::from_str(&interpolated).with_context(|| format!("parsing {}", path.display()))?;
            configs.insert(name, cfg);
        }
    }
    Ok(configs)
}

/// Load a single MCP config file *without* env var interpolation, so the
/// caller can see raw `${VAR}` references (used by the configure modal).
pub fn load_mcp_config_raw(mcp_dir: &Path, name: &str) -> anyhow::Result<McpServerFileConfig> {
    let path = mcp_dir.join(format!("{name}.toml"));
    let raw = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: McpServerFileConfig = toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(cfg)
}

/// Serialize and write a per-MCP config file.
pub fn save_mcp_config(mcp_dir: &Path, name: &str, config: &McpServerFileConfig) -> anyhow::Result<()> {
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
        assert_eq!(config.news.bind, "192.168.1.101:5555");
        assert_eq!(config.llm.model, "llama-3.3-70b");
        assert_eq!(config.capabilities.len(), 2);
        assert_eq!(config.capabilities[0].name, "daily-brief");
        assert_eq!(config.capabilities[1].name, "rolodex");
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
            api_key_env = "BASE_KEY"
            [news]
            bind = "127.0.0.1:3200"
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
        assert!(overrides.base_url.is_none());

        let effective = config.llm.with_overrides(overrides);
        assert_eq!(effective.model, "override-model");
        assert_eq!(effective.temperature, 0.9);
        assert_eq!(effective.base_url, "https://api.example.com/v1");
        assert_eq!(effective.api_key_env, "BASE_KEY");
    }

    #[test]
    fn expand_tilde_works() {
        let expanded = expand_tilde("~/.local/share/corre");
        assert!(!expanded.to_string_lossy().starts_with('~'));
    }
}
