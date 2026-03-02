//! OpenAI-compatible chat completions provider.
//!
//! Implements `corre_core::capability::LlmProvider` by POSTing to any `/chat/completions`
//! endpoint that speaks the OpenAI wire format.

use crate::types::*;
use anyhow::Context;
use corre_core::capability::{LlmProvider, LlmRequest, LlmResponse};

/// An LLM provider that speaks the OpenAI-compatible chat completions API.
/// Works with Venice.ai, OpenAI, Ollama, LM Studio, and others.
pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    default_temperature: f32,
}

impl OpenAiCompatProvider {
    pub fn new(base_url: String, api_key: String, model: String, temperature: f32) -> Self {
        let client = reqwest::Client::new();
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model: model,
            default_temperature: temperature,
        }
    }

    pub fn from_config(config: &corre_core::config::LlmConfig) -> anyhow::Result<Self> {
        let api_key = corre_core::secret::resolve_value(&config.api_key).context("resolving LLM API key")?;
        Ok(Self::new(config.base_url.clone(), api_key, config.model.clone(), config.temperature))
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse> {
        let api_request = ApiRequest {
            model: self.default_model.clone(),
            messages: request.messages.iter().map(|m| ChatMessage { role: m.role.clone(), content: m.content.clone() }).collect(),
            temperature: Some(request.temperature.unwrap_or(self.default_temperature)),
            max_completion_tokens: request.max_completion_tokens,
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
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry_after = response.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).map(|v| v.to_string());
                let body = response.text().await.unwrap_or_default();
                match retry_after {
                    Some(secs) => anyhow::bail!("LLM API rate limited (429), Retry-After: {secs}s. {body}"),
                    None => anyhow::bail!("LLM API rate limited (429). {body}"),
                }
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("LLM API returned {status}: {body}");
        }

        let api_response: ApiResponse = response.json().await.context("Failed to parse LLM API response")?;

        let choice = api_response.choices.into_iter().next().context("LLM API returned no choices")?;

        if let Some(reason) = &choice.finish_reason {
            if reason == "length" {
                let truncated = choice.message.content.as_deref().unwrap_or("");
                anyhow::bail!(
                    "LLM response truncated (finish_reason=length, got {} chars). Consider increasing max_completion_tokens",
                    truncated.len()
                );
            }
        }

        let content = choice.message.content.context("LLM API returned null content")?;
        if content.trim().is_empty() {
            anyhow::bail!("LLM API returned empty content");
        }

        Ok(LlmResponse { content })
    }
}
