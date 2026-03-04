//! Prompt injection defense middleware for Corre.
//!
//! Wraps [`McpCaller`](corre_core::capability::McpCaller) and
//! [`LlmProvider`](corre_core::capability::LlmProvider) transparently with two decorator
//! types — [`SafeMcpCaller`] and [`SafeLlmProvider`] — so capability code requires no
//! modification. Safety is enabled by default and configured via the `[safety]` section
//! in `corre.toml`.
//!
//! # Pipeline
//!
//! Every MCP tool output passes through four sequential stages before reaching the LLM:
//!
//! 1. **Validation** ([`validator`]) — truncates oversized outputs, strips null bytes,
//!    collapses whitespace obfuscation, and truncates repeated-character runs (>500 chars).
//! 2. **Injection sanitization** ([`sanitizer`]) — ~45 known injection phrases detected via
//!    Aho-Corasick, `eval()`/`exec()` calls, base64 payloads, special LLM token escaping,
//!    and role marker prefixing with `[DATA]`.
//! 3. **Leak detection** ([`leak_detector`]) — two-phase scan (Aho-Corasick prefix match +
//!    regex confirmation) for API keys (OpenAI, Anthropic, AWS, GitHub, Stripe, Slack),
//!    PEM private keys, Bearer tokens, and high-entropy hex strings. Also applied to LLM
//!    responses via [`SafeLlmProvider`] to catch exfiltration.
//! 4. **Policy evaluation** ([`policy`]) — rule-based engine for shell injection, SQL
//!    injection, path traversal, XSS, and encoded exploits. User-supplied
//!    `custom_block_patterns` always escalate to `Block`.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`validator`] | Stage 1: structural validation (length, null bytes, whitespace, token stuffing) |
//! | [`sanitizer`] | Stage 2: injection phrase detection and redaction via Aho-Corasick + regex |
//! | [`leak_detector`] | Stage 3: credential detection and `[REDACTED:*]` replacement |
//! | [`policy`] | Stage 4: severity-rated regex rules with configurable `Warn` / `Sanitize` / `Block` actions |
//! | [`boundary`] | XML `<tool_output>` boundary wrapping for LLM prompt isolation |
//! | [`report`] | [`SanitizationReport`](report::SanitizationReport) accumulator tracking all findings per tool call |
//! | [`safe_mcp`] | [`SafeMcpCaller`] — applies the full 4-stage pipeline to `call_tool` results |
//! | [`safe_llm`] | [`SafeLlmProvider`] — scans LLM responses for leaked credentials |
//! | [`config`] | Re-exports [`SafetyConfig`](corre_core::config::SafetyConfig) and [`PolicyAction`](corre_core::config::PolicyAction) |

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
