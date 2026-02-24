//! LLM provider crate for Corre.
//!
//! Re-exports `OpenAiCompatProvider`, the single concrete implementation of the
//! `LlmProvider` trait from `corre-core`. Speaks the OpenAI `/chat/completions` wire
//! format, which Venice.ai, Ollama, and many others support.

pub mod openai_compat;
pub mod types;

pub use openai_compat::OpenAiCompatProvider;
