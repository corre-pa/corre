use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Tracks which step the user has reached so the wizard can resume after interruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupState {
    pub completed_step: u8,
    pub llm_provider: Option<String>,
    pub llm_base_url: Option<String>,
    pub llm_model: Option<String>,
    pub llm_api_key_env: Option<String>,
    pub brave_api_key_env: Option<String>,
    pub enabled_capabilities: Vec<String>,
    pub topics_md: Option<String>,
    pub schedule_hour: Option<u8>,
    pub news_port: Option<u16>,
    pub news_title: Option<String>,
    /// Actual API key values (stored temporarily in the state file, cleared on completion).
    #[serde(default)]
    pub api_keys: std::collections::HashMap<String, String>,
}

impl Default for SetupState {
    fn default() -> Self {
        Self {
            completed_step: 0,
            llm_provider: None,
            llm_base_url: None,
            llm_model: None,
            llm_api_key_env: None,
            brave_api_key_env: None,
            enabled_capabilities: Vec::new(),
            topics_md: None,
            schedule_hour: None,
            news_port: None,
            news_title: None,
            api_keys: std::collections::HashMap::new(),
        }
    }
}

impl SetupState {
    /// Path to the persisted state file.
    pub fn state_path() -> PathBuf {
        super::templates::resolved_data_dir().join(".setup-state.json")
    }

    /// Load from disk, or return None if no state file exists.
    pub fn load() -> Option<Self> {
        let path = Self::state_path();
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Persist current state to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// Remove the state file (called on successful completion).
    pub fn cleanup() {
        let _ = std::fs::remove_file(Self::state_path());
    }
}
