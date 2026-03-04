//! SDK for writing Corre capability plugins.
//!
//! A capability plugin is a standalone binary that communicates with the Corre host over
//! stdin/stdout using CCPP v1 (Corre Capability Plugin Protocol), a JSON-RPC 2.0 protocol.
//! This crate provides everything a plugin needs: the [`CapabilityClient`] async helper,
//! output types, LLM request/response structs, HTML sanitization, and search-result parsing
//! utilities. Plugin authors should depend on `corre-sdk` and **only** `corre-sdk` from the
//! Corre workspace.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use corre_sdk::{CapabilityClient, LlmRequest};
//! use corre_sdk::types::{Article, CapabilityOutput, Section, Source};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = Arc::new(CapabilityClient::from_stdio());
//!     let params = client.accept_initialize().await?;
//!     let _guard = corre_sdk::init_tracing(
//!         &params.capability_name, params.log_dir.as_deref(), params.log_level.as_deref(),
//!     );
//!
//!     client.report_progress("working", Some(50), None).await?;
//!     // ... call MCP tools, make LLM requests ...
//!
//!     let output = CapabilityOutput { /* ... */ };
//!     client.send_result(output).await?;
//!     Ok(())
//! }
//! ```
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`client`] | [`CapabilityClient`] — async CCPP client with background reader, request multiplexing, and typed RPC methods for MCP tools, LLM completions, file output, and progress reporting |
//! | [`codec`] | Newline-delimited JSON codec for the stdin/stdout transport layer |
//! | [`html`] | HTML sanitization: [`sanitize_html`](html::sanitize_html) (strict allowlist for article content), [`sanitize_custom_html`](html::sanitize_custom_html) (wider allowlist for plugin pages), and [`sanitize_url`](html::sanitize_url) (http/https only) |
//! | [`llm`] | [`LlmRequest`] / [`LlmResponse`] types for the `llm/complete` RPC method, plus the convenience constructor [`LlmRequest::simple`] |
//! | [`manifest`] | Serde types for `manifest.toml`: [`PluginManifest`], permissions, sandbox declarations, config schema, service and link declarations |
//! | [`protocol`] | CCPP v1 JSON-RPC 2.0 wire types ([`Message`](protocol::Message), [`Request`](protocol::Request), [`Response`](protocol::Response)), all method parameter structs, and [`ErrorCode`] constants |
//! | [`tools`] | Utilities: [`parse_search_results`](tools::parse_search_results), [`extract_json`](tools::extract_json) (strips markdown fences), [`is_retryable_overload`](tools::is_retryable_overload), [`parse_context_length_limit`](tools::parse_context_length_limit) |
//! | [`types`] | Core output types: [`CapabilityOutput`](types::CapabilityOutput), [`Section`](types::Section), [`Article`](types::Article), [`Source`](types::Source), [`ContentType`](types::ContentType), [`CustomContent`](types::CustomContent) |
//!
//! # CCPP error codes
//!
//! Plugins should handle these codes when making RPC calls (see [`ErrorCode`]):
//!
//! | Code | Meaning | Recommended action |
//! |------|---------|--------------------|
//! | -32020 | Rate limited | Retry with exponential backoff |
//! | -32021 | Context too long | Reduce `max_completion_tokens` |
//! | -32022 | Provider error | Retry or fail gracefully |
//! | -32023 | Auth failed | **Fatal** — abort immediately |
//! | -32024 | Payment required | **Fatal** — abort immediately |
//! | -32010 | MCP tool error | Log and continue |
//! | -32030 | Safety blocked | Content was filtered; skip item |

pub mod client;
pub mod codec;
pub mod html;
pub mod llm;
pub mod manifest;
pub mod protocol;
pub mod tools;
pub mod types;

// Re-export key types at crate root for convenience.
pub use client::CapabilityClient;
pub use llm::{LlmMessage, LlmRequest, LlmResponse, LlmRole};
pub use manifest::{ExecutionMode, OutputDeclaration, OutputType, PluginLink, PluginManifest, SandboxPermissions, ServiceDeclaration};
pub use protocol::ErrorCode;
pub use types::{Article, CapabilityManifest, CapabilityOutput, ContentType, CustomContent, Section, Source};

use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

/// Set up tracing for a capability plugin.
///
/// - stderr layer: human-readable, no ANSI, no timestamps (host captures this)
/// - file layer (if `log_dir` is `Some`): daily-rotating log file at
///   `{log_dir}/{capability_name}-yyyy-mm-dd.log`
///
/// Both layers use the provided `log_level` filter (defaults to `"info"`).
/// Returns a guard that must be held alive for the file writer to flush.
pub fn init_tracing(
    capability_name: &str,
    log_dir: Option<&str>,
    log_level: Option<&str>,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let level = log_level.unwrap_or("info");
    let filter = || tracing_subscriber::EnvFilter::new(level);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_filter(filter());

    let (file_layer, guard) = match log_dir {
        Some(dir) => {
            let appender = tracing_appender::rolling::daily(dir, capability_name);
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer().with_writer(non_blocking).with_ansi(false).with_filter(filter());
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };

    tracing_subscriber::registry().with(stderr_layer).with(file_layer).init();

    guard
}
