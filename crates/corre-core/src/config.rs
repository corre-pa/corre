use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CorreConfig {
    pub general: GeneralConfig,
    pub llm: LlmConfig,
    pub news: NewsConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub capabilities: Vec<CapabilityConfig>,
    #[serde(default)]
    pub safety: SafetyConfig,
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
    pub api_key_env: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Maximum concurrent requests to the LLM API. Set this based on your
    /// provider's rate limits (e.g. Venice.ai M-tier = 50 req/min).
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_temperature() -> f32 {
    0.3
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_max_concurrent() -> usize {
    10
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
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
        let content = std::fs::read_to_string(path)?;
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
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
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
        assert_eq!(config.capabilities.len(), 1);
        assert_eq!(config.capabilities[0].name, "daily-brief");
        assert!(config.mcp.servers.contains_key("brave-search"));
        assert!(config.mcp.servers.contains_key("fetch"));
    }

    #[test]
    fn expand_tilde_works() {
        let expanded = expand_tilde("~/.local/share/corre");
        assert!(!expanded.to_string_lossy().starts_with('~'));
    }
}
