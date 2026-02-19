use crate::types::*;
use anyhow::Context;
use corre_core::capability::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmRole};

/// An LLM provider that speaks the OpenAI-compatible chat completions API.
/// Works with Venice.ai, OpenAI, Ollama, LM Studio, and others.
pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    default_temperature: f32,
    default_max_tokens: u32,
}

impl OpenAiCompatProvider {
    pub fn new(base_url: String, api_key: String, model: String, temperature: f32, max_tokens: u32) -> Self {
        let client = reqwest::Client::new();
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model: model,
            default_temperature: temperature,
            default_max_tokens: max_tokens,
        }
    }

    pub fn from_config(config: &corre_core::config::LlmConfig) -> anyhow::Result<Self> {
        let api_key =
            std::env::var(&config.api_key_env).with_context(|| format!("Missing env var `{}` for LLM API key", config.api_key_env))?;
        Ok(Self::new(config.base_url.clone(), api_key, config.model.clone(), config.temperature, config.max_tokens))
    }
}

fn role_to_string(role: &LlmRole) -> String {
    match role {
        LlmRole::System => "system".into(),
        LlmRole::User => "user".into(),
        LlmRole::Assistant => "assistant".into(),
    }
}

fn convert_messages(messages: &[LlmMessage]) -> Vec<ChatMessage> {
    messages.iter().map(|m| ChatMessage { role: role_to_string(&m.role), content: m.content.clone() }).collect()
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse> {
        // We don't send response_format since not all providers support it
        // (e.g. Venice.ai with llama models). The prompt itself asks for JSON when needed.
        let api_request = ApiRequest {
            model: self.default_model.clone(),
            messages: convert_messages(&request.messages),
            temperature: Some(request.temperature.unwrap_or(self.default_temperature)),
            max_tokens: Some(request.max_tokens.unwrap_or(self.default_max_tokens)),
            response_format: None,
        };

        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&api_request)
            .send()
            .await
            .context("Failed to send request to LLM API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("LLM API returned {status}: {body}");
        }

        let api_response: ApiResponse = response.json().await.context("Failed to parse LLM API response")?;

        let content = api_response.choices.into_iter().next().and_then(|c| c.message.content).unwrap_or_default();

        Ok(LlmResponse { content })
    }
}
