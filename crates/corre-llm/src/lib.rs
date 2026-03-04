//! LLM provider for Corre — speaks the OpenAI `/chat/completions` wire format.
//!
//! Contains [`OpenAiCompatProvider`], the single concrete implementation of the
//! [`LlmProvider`](corre_core::capability::LlmProvider) trait. Any service that exposes
//! an OpenAI-compatible chat completion endpoint works out of the box: Venice.ai, OpenAI,
//! Ollama, LM Studio, Together AI, and others.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`openai_compat`] | [`OpenAiCompatProvider`] — HTTP client, request mapping, rate-limit detection (429 + `Retry-After`), and context-length error handling |
//! | [`types`] | Crate-private wire-format structs (`ApiRequest`, `ApiResponse`) mapping directly to the OpenAI JSON schema |
//!
//! # Usage
//!
//! ```rust,ignore
//! let provider = OpenAiCompatProvider::from_config(&config.llm)?;
//! let response = provider.complete(LlmRequest::simple("system prompt", "user message")).await?;
//! ```
//!
//! The provider resolves `${VAR}` references in `api_key` at construction time. Per-capability
//! model and temperature overrides are applied via
//! [`LlmConfig::with_overrides`](corre_core::config::LlmConfig::with_overrides) before
//! constructing the provider.
//!
//! Provider-specific parameters (e.g. Venice system prompt toggles, reasoning effort) can be
//! passed through via the `extra_body` map in [`LlmConfig`](corre_core::config::LlmConfig),
//! which is flattened into the outgoing JSON request body.

pub mod openai_compat;
pub mod types;

pub use openai_compat::OpenAiCompatProvider;
