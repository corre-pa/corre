//! News-specific configuration types.
//!
//! `NewsConfig` was formerly in `corre-core::config` but only this crate and
//! `corre-dashboard` consume it, so it lives here now.

use serde::{Deserialize, Serialize};

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
    "127.0.0.1:5510".into()
}

fn default_title() -> String {
    "Corre News".into()
}

impl Default for NewsConfig {
    fn default() -> Self {
        Self { bind: default_bind(), title: default_title(), editor_token: None }
    }
}

impl NewsConfig {
    /// Parse a `NewsConfig` from a TOML table. Returns the default if the
    /// table is `None` or parsing fails.
    pub fn from_toml_table(table: Option<&toml::Value>) -> Self {
        table.and_then(|v| v.clone().try_into().ok()).unwrap_or_default()
    }
}
