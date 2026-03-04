//! Operator dashboard for the Corre scheduler.
//!
//! Provides an Axum router ([`server::build_router`]) serving a browser-based management
//! UI styled as a retro desktop, plus a JSON/SSE API. The dashboard is embedded in the
//! `corre run` process (default port 5500) and lets an operator:
//!
//! - Watch capability execution in real time via Server-Sent Events
//! - Trigger a capability run on demand
//! - Browse and install MCP servers from `corre-registry`
//! - Install and manage capability plugins
//! - Edit `corre.toml` settings and per-capability config at runtime
//! - Start/stop Docker companion services declared by plugins
//! - Inspect system metrics (CPU, memory) and historical capability logs
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`server`] | [`DashboardState`](server::DashboardState) (shared Axum state: execution tracker, config `RwLock`, registry client, MCP installer, service manager, shutdown signal), [`build_router`](server::build_router), and route handlers for the dashboard UI, settings, registry, MCP management, capability install, services, and system restart |
//!
//! # Authentication
//!
//! All API routes require an `editor_token` (configured in `[news]`), passed as a
//! `Bearer` header or `?token=` query parameter.

pub mod server;
