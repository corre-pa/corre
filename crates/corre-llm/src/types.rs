//! Crate-private wire-format types for the OpenAI chat completions HTTP API.
//!
//! These structs map directly to the JSON request and response bodies and are not re-exported.

use serde::{Deserialize, Serialize};

/// Wire format types for the OpenAI-compatible API.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ApiRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiResponse {
    pub choices: Vec<ApiChoice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiChoice {
    pub message: ApiMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiMessage {
    pub content: Option<String>,
}
