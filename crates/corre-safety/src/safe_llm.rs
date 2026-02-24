//! `SafeLlmProvider`: a safety-wrapping decorator for `LlmProvider`.
//!
//! Scans LLM responses for leaked credentials after each completion, catching
//! exfiltration attacks where the model echoes secrets from MCP tool outputs.

use crate::config::SafetyConfig;
use crate::leak_detector;
use crate::report::SanitizationReport;
use corre_core::capability::{LlmProvider, LlmRequest, LlmResponse};

/// A safety-wrapping `LlmProvider` that scans LLM responses for leaked secrets.
///
/// This catches exfiltration attacks where the LLM was tricked into outputting
/// secrets that appeared in tool outputs.
pub struct SafeLlmProvider {
    inner: Box<dyn LlmProvider>,
    config: SafetyConfig,
}

impl SafeLlmProvider {
    pub fn new(inner: Box<dyn LlmProvider>, config: &SafetyConfig) -> Self {
        Self { inner, config: config.clone() }
    }
}

#[async_trait::async_trait]
impl LlmProvider for SafeLlmProvider {
    async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse> {
        let mut response = self.inner.complete(request).await?;

        if self.config.detect_leaks {
            let mut report = SanitizationReport { original_len: response.content.len(), ..Default::default() };

            response.content = leak_detector::detect_and_redact(&response.content, &mut report);

            report.final_len = response.content.len();
            if report.had_findings() {
                report.log("llm", "response");
            }
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockLlmProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockLlmProvider {
        async fn complete(&self, _: LlmRequest) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse { content: self.response.clone() })
        }
    }

    #[tokio::test]
    async fn passes_clean_response() {
        let mock = MockLlmProvider { response: "Here is a summary of the news article.".into() };
        let config = SafetyConfig::default_enabled();
        let safe = SafeLlmProvider::new(Box::new(mock), &config);

        let resp = safe.complete(LlmRequest::simple("system", "user")).await.unwrap();
        assert_eq!(resp.content, "Here is a summary of the news article.");
    }

    #[tokio::test]
    async fn redacts_leaked_key_in_response() {
        let mock = MockLlmProvider { response: "The API key found was sk-abc12345678901234567890123456".into() };
        let config = SafetyConfig::default_enabled();
        let safe = SafeLlmProvider::new(Box::new(mock), &config);

        let resp = safe.complete(LlmRequest::simple("system", "user")).await.unwrap();
        assert!(resp.content.contains("[REDACTED:api_key]"));
        assert!(!resp.content.contains("sk-abc"));
    }

    #[tokio::test]
    async fn skips_detection_when_disabled() {
        let key = "sk-abc12345678901234567890123456";
        let mock = MockLlmProvider { response: format!("key: {key}") };
        let mut config = SafetyConfig::default_enabled();
        config.detect_leaks = false;
        let safe = SafeLlmProvider::new(Box::new(mock), &config);

        let resp = safe.complete(LlmRequest::simple("system", "user")).await.unwrap();
        assert!(resp.content.contains(key));
    }
}
