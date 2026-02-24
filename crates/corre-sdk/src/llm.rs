//! LLM request and response types for the `llm/complete` CCPP method.
//!
//! [`LlmRequest`] is serialised as the params of a `llm/complete` JSON-RPC request sent from
//! a plugin to the host. The host proxies it to the configured LLM provider and returns an
//! [`LlmResponse`]. Use [`LlmRequest::simple`] for the common two-message (system + user) case.

use serde::{Deserialize, Serialize};

/// A simplified LLM request used by capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub messages: Vec<LlmMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(default)]
    pub json_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmRole {
    System,
    User,
    Assistant,
}

impl LlmRequest {
    pub fn simple(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            messages: vec![
                LlmMessage { role: LlmRole::System, content: system.into() },
                LlmMessage { role: LlmRole::User, content: user.into() },
            ],
            temperature: None,
            max_completion_tokens: None,
            json_mode: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
}
