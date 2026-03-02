//! `corre-registry` -- MCP server registry client, installer, and tooling.
//!
//! Provides a [`RegistryClient`] that fetches a remote JSON manifest of available MCP
//! servers, an [`McpInstaller`] that materialises entries into runnable `McpServerConfig`
//! values, and helpers for dependency checking and live server testing.

pub mod client;
pub mod installer;
pub mod manifest;
pub mod tester;

pub use client::RegistryClient;
pub use installer::McpInstaller;
