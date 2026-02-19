use crate::types::{ChatCompletionRequest, ChatCompletionResponse};

#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: ChatCompletionRequest) -> anyhow::Result<ChatCompletionResponse>;
}
