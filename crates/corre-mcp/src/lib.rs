//! MCP server pool management for Corre.
//!
//! Manages the lifecycle of MCP server child processes on behalf of capabilities.
//! Servers are spawned lazily on first tool call, cached for the duration of a capability
//! run, and shut down afterward. The pool implements
//! [`McpCaller`](corre_core::capability::McpCaller) from `corre-core`, so the rest of the
//! system depends only on that abstraction.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`pool`] | [`McpPool`] — lazy-start, connection-caching pool with environment isolation. Spawns each server as a stdio child process via `rmcp::TokioChildProcess`, clears inherited env vars for security, and re-adds only `PATH`, `HOME`, `NODE_PATH` plus the server's declared env |
//! | [`server_def`] | [`McpServerDef`] — runtime-ready server description built from [`McpServerConfig`](corre_core::config::McpServerConfig), with all `${VAR}` references resolved to concrete values |
//!
//! # Key types
//!
//! - **[`McpPool`]** — the pool. `call_tool` auto-starts the named server if not yet running,
//!   collects text content blocks from the MCP response, and returns them as a
//!   `serde_json::Value` (single object or array). `shutdown()` tears down all running servers.
//! - **[`McpServerDef`]** — one entry per configured server, holding the resolved command,
//!   args, and environment variables ready for `tokio::process::Command`.

pub mod pool;
pub mod server_def;

pub use pool::McpPool;
pub use server_def::McpServerDef;
