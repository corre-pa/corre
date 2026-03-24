use anyhow::Context as _;
use corre_core::config::AppLlmConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GymConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    pub telegram_bot_token: String,
    /// Telegram user IDs allowed to use the bot. Empty = allow all (dev mode).
    #[serde(default)]
    pub telegram_allowed_ids: Vec<i64>,
    #[serde(default = "default_timezone")]
    pub default_timezone: String,
    #[serde(default = "default_history_limit")]
    pub conversation_history_limit: usize,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_max_message_length")]
    pub max_message_length: usize,
    #[serde(default = "default_session_timeout_hours")]
    pub session_timeout_hours: u32,
    #[serde(default)]
    pub llm: Option<AppLlmConfig>,
}

fn default_bind() -> String {
    "127.0.0.1:5520".into()
}

fn default_timezone() -> String {
    "Europe/London".into()
}

fn default_history_limit() -> usize {
    20
}

fn default_db_path() -> String {
    "gym-tracker.db".into()
}

fn default_max_message_length() -> usize {
    2000
}

fn default_session_timeout_hours() -> u32 {
    4
}

impl GymConfig {
    pub fn from_toml_table(table: Option<&toml::Value>) -> anyhow::Result<Self> {
        table.cloned().ok_or_else(|| anyhow::anyhow!("missing [gym] section in corre.toml")).and_then(|v| v.try_into().map_err(Into::into))
    }

    /// Resolve `${VAR}` references in secret fields.
    pub fn resolve_secrets(&mut self) -> anyhow::Result<()> {
        self.telegram_bot_token =
            corre_core::secret::resolve_value(&self.telegram_bot_token).context("resolving TELEGRAM_GYM_BOT_TOKEN")?;
        Ok(())
    }
}
