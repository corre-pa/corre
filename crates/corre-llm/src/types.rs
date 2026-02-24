use serde::{Deserialize, Serialize};

/// Wire format types for the OpenAI-compatible API.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponseFormat {
    pub r#type: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiResponse {
    pub choices: Vec<ApiChoice>,
    #[allow(dead_code)]
    pub usage: Option<Usage>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
