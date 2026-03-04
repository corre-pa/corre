//! Shared traits, types, and configuration for the Corre workspace.
//!
//! `corre-core` depends only on [`corre_sdk`] and sits near the bottom of the workspace
//! dependency graph. Every other host-side crate imports its abstractions rather than
//! depending on one another, keeping the compile graph shallow and the boundaries clean.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`capability`] | Core traits ([`Capability`](capability::Capability), [`McpCaller`](capability::McpCaller), [`LlmProvider`](capability::LlmProvider)), the [`CapabilityContext`](capability::CapabilityContext) bundle passed to every run, and [`ProgressTracker`](capability::ProgressTracker) for timeout decisions |
//! | [`config`] | Full `corre.toml` deserialization ([`CorreConfig`](config::CorreConfig)), per-MCP-server file configs ([`McpServerConfig`](config::McpServerConfig)), env-var interpolation, and safety / registry settings |
//! | [`plugin`] | Plugin discovery — scans `{data_dir}/plugins/` for valid subprocess capabilities with a `manifest.toml` and binary |
//! | [`sandbox`] | Landlock filesystem restrictions + seccomp network filtering applied to subprocess capability binaries on Linux |
//! | [`scheduler`] | Thin async wrapper around `tokio-cron-scheduler` accepting 6-field cron expressions (sec min hour day month weekday) |
//! | [`secret`] | `${VAR}` interpolation engine for config values, resolving references from the host environment |
//! | [`service`] | Docker container lifecycle ([`ServiceManager`](service::ServiceManager)) for plugin companion services declared in `manifest.toml` |
//! | [`tracker`] | Real-time execution state ([`ExecutionTracker`](tracker::ExecutionTracker)), per-capability progress and logs, system metrics, and broadcast channel for the dashboard SSE stream |
//!
//! # Key traits
//!
//! **[`Capability`](capability::Capability)** is the unit of work. Each implementation
//! declares a manifest (name, cron schedule, required MCP servers) and an async `execute`
//! method that receives a [`CapabilityContext`](capability::CapabilityContext) and returns
//! a [`CapabilityOutput`](corre_sdk::types::CapabilityOutput). Built-in capabilities
//! implement the trait directly; subprocess plugins are wrapped by
//! `corre_host::SubprocessCapability`.
//!
//! **[`McpCaller`](capability::McpCaller)** (`call_tool`, `list_tools`) and
//! **[`LlmProvider`](capability::LlmProvider)** (`complete`) decouple capability code from
//! the concrete `corre-mcp` and `corre-llm` implementations. The safety layer in
//! `corre-safety` wraps both transparently.
//!
//! # Configuration
//!
//! [`CorreConfig`](config::CorreConfig) models the entire `corre.toml` file: global LLM
//! settings, per-capability schedule and LLM overrides, safety policy, and registry URL.
//! Per-MCP-server configs live in individual `*.toml` files under `{data_dir}/config/mcp/`
//! and are loaded separately via [`config::load_mcp_configs`].

pub mod capability;
pub mod config;
pub mod plugin;
pub mod sandbox;
pub mod scheduler;
pub mod secret;
pub mod service;
pub mod tracker;
