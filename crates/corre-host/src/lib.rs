//! Subprocess app host and plugin registry.
//!
//! This crate bridges the host process and external app binaries. It contains two
//! key components:
//!
//! - [`SubprocessApp`](subprocess::SubprocessApp) — implements the
//!   [`App`](corre_core::app::App) trait by spawning a plugin binary
//!   and brokering its CCPP (Corre App Plugin Protocol) JSON-RPC requests for MCP
//!   tool calls, LLM completions, and file output.
//!
//! - [`AppRegistry`](registry::AppRegistry) — maps app names to
//!   `Arc<dyn App>` trait objects, constructed from config entries and discovered
//!   plugins.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`registry`] | [`AppRegistry`](registry::AppRegistry) — builds a name-to-trait-object map from [`AppConfig`](corre_core::config::AppConfig) entries and [`DiscoveredPlugin`](corre_core::plugin::DiscoveredPlugin) values, instantiating a `SubprocessApp` for each plugin |
//! | [`subprocess`] | [`SubprocessApp`](subprocess::SubprocessApp) — spawns the plugin binary, sends the `initialize` handshake, runs a concurrent message loop dispatching `mcp/callTool`, `llm/complete`, and `output/write` requests via `FuturesUnordered`, and collects the final `app/result` notification |
//!
//! # CCPP protocol overview
//!
//! The host sends an `initialize` request with config paths, available MCP servers, and
//! concurrency limits. The plugin responds, then issues RPC requests (`mcp/callTool`,
//! `llm/complete`, `output/write`) and fire-and-forget notifications (`progress`, `log`,
//! `app/result`). Multiple RPC calls can be in-flight simultaneously.
//!
//! # Security
//!
//! - Plugin binaries are sandboxed with Landlock + seccomp via
//!   [`LandlockSandbox`](corre_core::sandbox::LandlockSandbox) when sandbox permissions
//!   are declared in the plugin manifest.
//! - Output paths are validated against the plugin's declared
//!   [`OutputDeclaration`](corre_sdk::manifest::OutputDeclaration) entries.
//! - Secret values (API keys, tokens, passwords) are redacted in debug logs.

pub mod registry;
pub mod subprocess;
