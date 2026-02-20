//! Prompt injection defense middleware for Corre.
//!
//! Wraps `McpCaller` and `LlmProvider` transparently to sanitize untrusted
//! content from MCP tool outputs and scan LLM responses for secret leakage.

pub mod boundary;
pub mod config;
pub mod leak_detector;
pub mod policy;
pub mod report;
pub mod safe_llm;
pub mod safe_mcp;
pub mod sanitizer;
pub mod validator;

pub use safe_llm::SafeLlmProvider;
pub use safe_mcp::SafeMcpCaller;
