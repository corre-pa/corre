//! Subprocess capability host and plugin registry.
//!
//! This crate bridges the host process and external capability binaries. It contains two
//! key components:
//!
//! - [`SubprocessCapability`](subprocess::SubprocessCapability) — implements the
//!   [`Capability`](corre_core::capability::Capability) trait by spawning a plugin binary
//!   and brokering its CCPP (Corre Capability Plugin Protocol) JSON-RPC requests for MCP
//!   tool calls, LLM completions, and file output.
//!
//! - [`CapabilityRegistry`](registry::CapabilityRegistry) — maps capability names to
//!   `Arc<dyn Capability>` trait objects, constructed from config entries and discovered
//!   plugins.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`registry`] | [`CapabilityRegistry`](registry::CapabilityRegistry) — builds a name-to-trait-object map from [`CapabilityConfig`](corre_core::config::CapabilityConfig) entries and [`DiscoveredPlugin`](corre_core::plugin::DiscoveredPlugin) values, instantiating a `SubprocessCapability` for each plugin |
//! | [`subprocess`] | [`SubprocessCapability`](subprocess::SubprocessCapability) — spawns the plugin binary, sends the `initialize` handshake, runs a concurrent message loop dispatching `mcp/callTool`, `llm/complete`, and `output/write` requests via `FuturesUnordered`, and collects the final `capability/result` notification |
//!
//! # CCPP protocol overview
//!
//! The host sends an `initialize` request with config paths, available MCP servers, and
//! concurrency limits. The plugin responds, then issues RPC requests (`mcp/callTool`,
//! `llm/complete`, `output/write`) and fire-and-forget notifications (`progress`, `log`,
//! `capability/result`). Multiple RPC calls can be in-flight simultaneously.
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
