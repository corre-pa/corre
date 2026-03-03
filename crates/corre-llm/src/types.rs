//! Crate-private wire-format types for the OpenAI chat completions HTTP API.
//!
//! These structs map directly to the JSON request and response bodies and are not re-exported.

use std::collections::HashMap;

use corre_core::capability::LlmRole;
use serde::{Deserialize, Serialize};

/// Wire format types for the OpenAI-compatible API.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatMessage {
    pub role: LlmRole,
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
    /// Provider-specific fields flattened into the top-level JSON body.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_body_flattens_into_api_request() {
        let mut extra = std::collections::HashMap::new();
        extra.insert("stream".into(), serde_json::Value::Bool(false));
        extra.insert("reasoning_effort".into(), serde_json::json!("minimal"));
        extra.insert("venice_parameters".into(), serde_json::json!({
            "include_venice_system_prompt": false,
            "strip_thinking_response": true
        }));

        let req = ApiRequest {
            model: "test-model".into(),
            messages: vec![],
            temperature: Some(0.3),
            max_completion_tokens: None,
            extra,
        };
        let json: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "test-model");
        assert!(json["temperature"].as_f64().unwrap() - 0.3 < 0.001);
        assert_eq!(json["stream"], false);
        assert_eq!(json["reasoning_effort"], "minimal");
        assert_eq!(json["venice_parameters"]["include_venice_system_prompt"], false);
        assert_eq!(json["venice_parameters"]["strip_thinking_response"], true);
        // extra_body keys are at the top level, not nested under "extra"
        assert!(json.get("extra").is_none());
    }
}
