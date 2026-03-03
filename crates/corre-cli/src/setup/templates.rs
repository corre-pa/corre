//! Configuration file rendering and OS data-directory resolution for the setup wizard.
//!
//! Builds a `CorreConfig` from collected `SetupState`, serialises it to TOML, and provides
//! the platform-appropriate default data directory paths.

use anyhow::Context as _;
use corre_core::config::*;
use std::collections::HashMap;
use std::path::PathBuf;

use super::state::SetupState;

/// Returns the OS-appropriate default data directory as a tilde-prefixed string for config files.
pub fn default_data_dir() -> String {
    if cfg!(target_os = "macos") {
        "~/Library/Application Support/corre".into()
    } else if cfg!(target_os = "windows") {
        "~/AppData/Local/corre".into()
    } else {
        "~/.local/share/corre".into()
    }
}

/// Returns the OS-appropriate resolved data directory path.
///
/// Returns an error if `$HOME` is not set rather than silently falling back to
/// the current working directory (which could write secrets to unexpected locations).
pub fn resolved_data_dir() -> anyhow::Result<PathBuf> {
    if cfg!(target_os = "macos") {
        Ok(dirs::home_dir().context("$HOME is not set; cannot determine data directory")?.join("Library/Application Support/corre"))
    } else if cfg!(target_os = "windows") {
        let base =
            dirs::data_local_dir().or_else(dirs::home_dir).context("$HOME / LOCALAPPDATA is not set; cannot determine data directory")?;
        Ok(base.join("AppData/Local/corre"))
    } else {
        Ok(dirs::home_dir().context("$HOME is not set; cannot determine data directory")?.join(".local/share/corre"))
    }
}

/// Returns the OS-appropriate command name for npx.
fn npx_command() -> &'static str {
    if cfg!(target_os = "windows") { "npx.cmd" } else { "npx" }
}

/// Build a `CorreConfig` from the wizard state, ready for TOML serialization.
pub fn build_config(state: &SetupState) -> CorreConfig {
    let hour = state.schedule_hour.unwrap_or(5);
    let port = state.news_port.unwrap_or(5510);

    let capabilities: Vec<CapabilityConfig> = state
        .enabled_capabilities
        .iter()
        .map(|name| {
            let llm_override = state.capability_llm_overrides.get(name.as_str()).map(|ovr| CapabilityLlmConfig {
                model: ovr.model.clone(),
                temperature: None,
                max_completion_tokens: None,
                max_concurrent: None,
                extra_body: None,
            });

            match name.as_str() {
                "daily-brief" => CapabilityConfig {
                    name: "daily-brief".into(),
                    description: "Researches topics and produces a daily news briefing".into(),
                    schedule: format!("0 0 {hour} * * *"),
                    mcp_servers: vec!["brave-search".into()],
                    config_path: Some("config/topics.yml".into()),
                    enabled: true,
                    llm: llm_override,
                    plugin: None,
                    log_level: None,
                },
                other => CapabilityConfig {
                    name: other.into(),
                    description: String::new(),
                    schedule: format!("0 0 {hour} * * *"),
                    mcp_servers: Vec::new(),
                    config_path: None,
                    enabled: true,
                    llm: llm_override,
                    plugin: None,
                    log_level: None,
                },
            }
        })
        .collect();

    CorreConfig {
        general: GeneralConfig { data_dir: default_data_dir(), log_level: "info".into() },
        llm: LlmConfig {
            provider: "openai-compatible".into(),
            base_url: state.llm_base_url.clone().unwrap_or_else(|| "https://api.venice.ai/api/v1".into()),
            model: state.llm_model.clone().unwrap_or_else(|| "openai-gpt-oss-120b".into()),
            api_key: state.llm_api_key.clone().unwrap_or_else(|| "${VENICE_API_KEY}".into()),
            temperature: 0.3,
            max_concurrent: 10,
            extra_body: HashMap::new(),
        },
        news: {
            let mut table = toml::map::Map::new();
            table.insert("bind".into(), toml::Value::String(format!("127.0.0.1:{port}")));
            table.insert("title".into(), toml::Value::String(state.news_title.clone().unwrap_or_else(|| "Corre News".into())));
            toml::Value::Table(table)
        },
        capabilities,
        safety: SafetyConfig::default(),
        registry: RegistryConfig::default(),
    }
}

/// Write per-MCP config files for MCP servers referenced by the setup state.
pub fn write_mcp_configs(state: &SetupState, data_dir: &std::path::Path) -> anyhow::Result<()> {
    // Always include brave-search if daily-brief is enabled
    if state.enabled_capabilities.iter().any(|c| c == "daily-brief") {
        let config = McpServerConfig {
            command: npx_command().into(),
            args: vec!["-y".into(), "@brave/brave-search-mcp-server".into()],
            env: {
                let mut env = HashMap::new();
                env.insert("BRAVE_API_KEY".into(), state.brave_api_key_env.clone().unwrap_or_else(|| "BRAVE_API_KEY".into()));
                env
            },
            registry_id: None,
            installed: true,
        };
        let mcp_dir = data_dir.join("config").join("mcp");
        save_mcp_config(&mcp_dir, "brave-search", &config)?;
    }
    Ok(())
}

/// Default topics.md content for new users.
pub const DEFAULT_TOPICS: &str = "\
# Daily Brief Topics

## Technology
Find the latest developments in technology, programming, and software engineering.

## World News
The latest important developments in world news and geopolitics.

## Science
Interesting scientific discoveries, research papers, and breakthroughs.
";

/// Format a `CorreConfig` as a TOML string with helpful comments.
pub fn format_config_toml(config: &CorreConfig) -> anyhow::Result<String> {
    // Use toml serialization, then add comments
    let raw = toml::to_string_pretty(config)?;

    // Add a header comment
    let data_dir = default_data_dir();
    let header = format!(
        "\
# Corre configuration — generated by `corre setup`
# Edit this file to change your LLM provider, topics, schedules, etc.
# API keys are stored in {data_dir}/.env (not in this file)

"
    );
    Ok(format!("{header}{raw}"))
}
