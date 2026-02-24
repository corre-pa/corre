//! MCP server pool management for Corre.
//!
//! Exposes `McpPool` (a lazily-started, cached pool of MCP server child processes) and
//! `McpServerDef` (a runtime-ready server description built from config). `McpPool`
//! implements `corre-core::capability::McpCaller`.

pub mod pool;
pub mod server_def;

pub use pool::McpPool;
pub use server_def::McpServerDef;
