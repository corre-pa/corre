//! MCP server and capability registry client, installer, and tooling.
//!
//! Fetches a remote JSON manifest of available MCP servers and capabilities, caches it
//! in memory with a configurable TTL, and provides installation (npx, pip/uvx, binary
//! download with SHA-256 verification), dependency checking, and live connection testing.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`client`] | [`RegistryClient`] — HTTP client with in-memory TTL cache. Fetches `{url}/mcp/registry.json`, supports search by name/description/tags, and provides lookup for individual MCP server and capability entries |
//! | [`installer`] | [`McpInstaller`] — installs MCP servers via npx (auto-download), pip/uvx, or direct binary download (SHA-256 verified, written to `{data_dir}/bin/`). Writes per-server config to `{data_dir}/config/mcp/` |
//! | [`manifest`] | Serde types for the registry JSON: [`RegistryManifest`](manifest::RegistryManifest), [`McpRegistryEntry`](manifest::McpRegistryEntry), [`CapabilityEntry`](manifest::CapabilityEntry), [`InstallMethod`](manifest::InstallMethod) (Npx / Pip / Binary) |
//! | [`tester`] | Connection tester — spins up a temporary MCP server, calls `list_tools`, shuts it down, and reports the discovered tool names |

pub mod client;
pub mod installer;
pub mod manifest;
pub mod tester;

pub use client::RegistryClient;
pub use installer::McpInstaller;
