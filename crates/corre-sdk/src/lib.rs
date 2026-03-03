//! `corre-sdk` — the library for writing Corre capability plugins.
//!
//! Provides the CCPP v1 protocol types, the [`client::CapabilityClient`] async helper,
//! shared output types, LLM request/response structs, and utility functions for search
//! result parsing and HTML sanitization. Link this crate in your plugin binary and use
//! [`client::CapabilityClient::from_stdio`] as the entry point.

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
