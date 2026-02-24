# corre-dashboard

An operator dashboard for the Corre scheduler, providing a browser-based management interface
styled as a old-school OS desktop.

## Role in the Corre project

`corre-dashboard` is the control plane for the Corre runtime. It does not produce or archive
information; it lets an operator observe the running system and act on it:

- Watch capabilities execute in real time via SSE
- Trigger a capability immediately
- Browse and install MCP servers from `corre-registry`
- Edit and save `corre.toml` settings at runtime
- Inspect historical capability logs

The crate exposes an Axum router that is merged into the main web server by `corre-cli`.

## Key types

### `DashboardState`

Shared state injected into every handler: `ExecutionTracker`, `CorreConfig` (behind `RwLock`),
run trigger channel, `RegistryClient`, and `McpInstaller`.

### `DashboardAssets`

`rust-embed`-backed struct that bundles `dashboard.css` and `dashboard.js` into the binary.

## HTTP routes

All routes require an editor token (query parameter or `Authorization: Bearer` header).

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/dashboard` | HTML dashboard page |
| `GET` | `/api/dashboard/status` | Capability states and system metrics |
| `POST` | `/api/dashboard/run/{name}` | Trigger immediate capability run |
| `GET` | `/api/dashboard/events` | SSE stream of live updates |
| `GET` | `/api/dashboard/logs/{date}` | Historical logs |
| `GET/PUT` | `/api/settings` | Read/write `CorreConfig` |
| `GET` | `/api/registry/catalog` | Full MCP registry manifest |
| `GET` | `/api/registry/search?q=...` | Search registry entries |
| `POST` | `/api/mcp/install` | Install an MCP server |
| `POST` | `/api/mcp/test/{name}` | Test an MCP server |

## Usage

```rust
use corre_dashboard::server::{DashboardState, build_router, spawn_metrics_broadcaster};

let app = Router::new()
    .merge(news_router)
    .merge(build_router(dashboard_state));
```
