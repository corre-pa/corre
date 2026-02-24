//! Operator dashboard for the Corre scheduler.
//!
//! Provides an Axum router (`server::build_router`) that serves a browser-based management
//! UI and a JSON/SSE API for monitoring capability execution, triggering manual runs,
//! managing MCP servers via the registry, and editing `corre.toml` at runtime.

pub mod server;
