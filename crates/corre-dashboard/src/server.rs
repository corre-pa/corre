//! Axum handlers and router for the Corre operator dashboard.
//!
//! `build_router` returns a fully-configured `Router` ready to be merged into the main
//! application. All routes require a bearer editor token. The SSE endpoint streams
//! `DashboardEvent` updates from `ExecutionTracker`.

use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::Event;
use axum::response::{Html, IntoResponse, Response, Sse};
use axum::routing::{get, post};
use corre_core::config::CorreConfig;
use corre_core::plugin::DiscoveredPlugin;
use corre_core::service::ServiceManager;
use corre_core::tracker::{DashboardEvent, ExecutionTracker};
use corre_registry::McpInstaller;
use corre_registry::RegistryClient;
use rust_embed::RustEmbed;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

#[derive(RustEmbed)]
#[folder = "static/"]
struct DashboardAssets;

#[derive(askama::Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate<'a> {
    title: &'a str,
    token: &'a str,
    config_json: &'a str,
    plugin_links_json: &'a str,
    config_editors_json: &'a str,
}

pub struct DashboardState {
    pub tracker: Arc<ExecutionTracker>,
    pub config: Arc<RwLock<CorreConfig>>,
    pub config_path: std::path::PathBuf,
    pub run_trigger: mpsc::Sender<String>,
    pub registry_client: Arc<RegistryClient>,
    pub installer: Arc<McpInstaller>,
    pub service_manager: Arc<ServiceManager>,
    pub shutdown_signal: tokio::sync::watch::Sender<bool>,
    pub plugins: Arc<Vec<DiscoveredPlugin>>,
}

pub fn build_router(state: Arc<DashboardState>) -> Router {
    Router::new()
        .route("/", get(dashboard_page_handler))
        .route("/api/dashboard/status", get(status_handler))
        .route("/api/dashboard/run/{name}", post(run_now_handler))
        .route("/api/dashboard/events", get(sse_handler))
        .route("/api/dashboard/logs/{date}", get(historical_logs_handler))
        .route("/api/settings", get(get_settings_handler).put(put_settings_handler))
        // Registry & MCP management routes
        .route("/api/registry/catalog", get(registry_catalog_handler))
        .route("/api/registry/search", get(registry_search_handler))
        .route("/api/registry/refresh", post(registry_refresh_handler))
        .route("/api/mcp/installed", get(mcp_installed_handler))
        .route("/api/mcp/install", post(mcp_install_handler))
        .route("/api/mcp/uninstall/{name}", post(mcp_uninstall_handler))
        .route("/api/mcp/test/{name}", post(mcp_test_handler))
        .route("/api/mcp/config/{name}", get(mcp_config_handler))
        .route("/api/mcp/configure/{name}", axum::routing::put(mcp_configure_handler))
        // Capability install/uninstall routes
        .route("/api/capabilities/installed", get(capabilities_installed_handler))
        .route("/api/capabilities/install", post(capability_install_handler))
        .route("/api/capabilities/uninstall/{name}", post(capability_uninstall_handler))
        // System management routes
        .route("/api/system/restart", post(system_restart_handler))
        // Service management routes
        .route("/api/services", get(services_list_handler))
        .route("/api/services/start", post(service_start_handler))
        .route("/api/services/stop/{name}", post(service_stop_handler))
        // Config editor routes (generic per-capability config read/write)
        .route("/api/config/{name}", get(get_config_handler).put(put_config_handler))
        .route("/dashboard/static/{*path}", get(static_handler))
        .with_state(state)
}

// ── Inline helpers replacing NewsConfig ──────────────────────────────────

fn editor_token(news: &toml::Value) -> Option<String> {
    news.get("editor_token").and_then(|v| v.as_str()).map(String::from)
}

fn news_title(news: &toml::Value) -> String {
    news.get("title").and_then(|v| v.as_str()).unwrap_or("Corre News").to_string()
}

fn news_config_from_toml(config: &CorreConfig) -> (Option<String>, String) {
    (editor_token(&config.news), news_title(&config.news))
}

/// Collect plugin links from all discovered plugins in the data directory.
///
/// Template variables in link URLs are expanded:
/// - `{service:NAME}` → the hostname from the request's `Host` header (so links
///   resolve correctly whether the user reaches the dashboard via localhost,
///   a LAN IP, or a Tailscale DNS name). The port is taken from the service's
///   first port mapping.
fn collect_plugin_links(plugins: &[DiscoveredPlugin], request_host: &str) -> Vec<serde_json::Value> {
    // Strip the port from the request host to get the bare hostname/IP.
    let host_without_port = request_host.rsplit_once(':').map_or(request_host, |(h, _)| h);
    plugins
        .iter()
        .flat_map(|p| {
            let services = &p.manifest.plugin.services;
            p.manifest.plugin.links.iter().map(move |link| {
                let url = expand_link_url(&link.url, services, host_without_port);
                serde_json::json!({
                    "label": link.label,
                    "url": url,
                    "icon": link.icon,
                })
            })
        })
        .collect()
}

/// Expand `{service:NAME}` placeholders in a link URL.
///
/// The placeholder is replaced with the requesting user's hostname so that the
/// link works from whichever network the browser is on (localhost, LAN,
/// Tailscale). The port is preserved from the original URL template.
fn expand_link_url(url: &str, services: &[corre_sdk::manifest::ServiceDeclaration], request_host: &str) -> String {
    let mut result = url.to_string();
    for svc in services {
        let placeholder = format!("{{service:{}}}", svc.name);
        result = result.replace(&placeholder, request_host);
    }
    result
}

/// Build a list of config editor entries for capabilities that declare a `config_path`
/// and a `config_schema` in their plugin manifest. The returned JSON includes the
/// serialised schema so the dashboard JS can build a form dynamically.
fn collect_config_editors(config: &CorreConfig, plugins: &[DiscoveredPlugin]) -> Vec<serde_json::Value> {
    // Index plugins by name for fast lookup.
    let plugin_map: std::collections::HashMap<&str, &DiscoveredPlugin> =
        plugins.iter().map(|p| (p.manifest.plugin.name.as_str(), p)).collect();
    config
        .capabilities
        .iter()
        .filter_map(|c| {
            let plugin = plugin_map.get(c.name.as_str())?;
            let defaults = &plugin.manifest.plugin.defaults;
            // Only include capabilities that have both a config_path and a config_schema.
            let _config_path = defaults.config_path.as_ref().or(c.config_path.as_ref())?;
            let schema = defaults.config_schema.as_ref()?;
            Some(serde_json::json!({
                "name": c.name,
                "cap": c.name,
                "schema": schema,
            }))
        })
        .collect()
}

// ── Auth helpers ─────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct TokenQuery {
    #[serde(default)]
    token: Option<String>,
}

fn extract_token(headers: &HeaderMap, query: &TokenQuery) -> Option<String> {
    if let Some(ref t) = query.token {
        return Some(t.clone());
    }
    headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(|s| s.to_string())
}

fn check_auth(editor_token: &Option<String>, provided: Option<&str>) -> Result<(), StatusCode> {
    let expected = editor_token.as_deref().ok_or(StatusCode::FORBIDDEN)?;
    match provided {
        Some(t) if t == expected => Ok(()),
        _ => Err(StatusCode::FORBIDDEN),
    }
}

/// Helper: extract token and check auth, returning the token string on success.
async fn require_auth(state: &DashboardState, headers: &HeaderMap, query: &TokenQuery) -> Result<String, Response<Body>> {
    let token_str = extract_token(headers, query);
    let config = state.config.read().await;
    let (et, _) = news_config_from_toml(&config);
    check_auth(&et, token_str.as_deref()).map_err(|s| s.into_response())?;
    Ok(token_str.unwrap_or_default())
}

// ── Handlers ─────────────────────────────────────────────────────────────

async fn dashboard_page_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    let (et, title) = news_config_from_toml(&config);
    if check_auth(&et, token_str.as_deref()).is_err() {
        return (StatusCode::FORBIDDEN, "Invalid or missing editor token. Add ?token=YOUR_TOKEN to the URL.").into_response();
    }
    let token = token_str.unwrap_or_default();
    let config_json = match serde_json::to_string_pretty(&*config) {
        Ok(j) => j,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Serialization error: {e}")).into_response(),
    };

    let request_host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    let plugin_links = collect_plugin_links(&state.plugins, request_host);
    let plugin_links_json = serde_json::to_string(&plugin_links).unwrap_or_else(|_| "[]".into());

    let config_editors = collect_config_editors(&config, &state.plugins);
    let config_editors_json = serde_json::to_string(&config_editors).unwrap_or_else(|_| "[]".into());

    let template = DashboardTemplate {
        title: &title,
        token: &token,
        config_json: &config_json,
        plugin_links_json: &plugin_links_json,
        config_editors_json: &config_editors_json,
    };
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
    }
}

async fn status_handler(State(state): State<Arc<DashboardState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }
    let capabilities = state.tracker.snapshot().await;
    let metrics = state.tracker.system_metrics();
    let body = serde_json::json!({
        "capabilities": capabilities,
        "metrics": metrics,
    });
    axum::Json(body).into_response()
}

async fn run_now_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    if state.tracker.is_running(&name).await {
        return (StatusCode::CONFLICT, format!("Capability `{name}` is already running")).into_response();
    }

    match state.run_trigger.try_send(name.clone()) {
        Ok(()) => (StatusCode::ACCEPTED, format!("Capability `{name}` triggered")).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Run queue is full, try again later").into_response(),
    }
}

async fn sse_handler(State(state): State<Arc<DashboardState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    // Send initial snapshot, then stream incremental events
    let initial_capabilities = state.tracker.snapshot().await;
    let initial_metrics = state.tracker.system_metrics();
    let mut rx = state.tracker.subscribe();

    let stream = async_stream::stream! {
        // Send initial snapshot as individual events
        for cap in initial_capabilities {
            let event = DashboardEvent::CapabilityUpdate(cap);
            if let Ok(data) = serde_json::to_string(&event) {
                yield Ok::<_, Infallible>(Event::default().event("message").data(data));
            }
        }

        // Send initial metrics
        let metrics_event = DashboardEvent::SystemMetrics(initial_metrics);
        if let Ok(data) = serde_json::to_string(&metrics_event) {
            yield Ok::<_, Infallible>(Event::default().event("message").data(data));
        }

        // Stream incremental events
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(data) = serde_json::to_string(&event) {
                        yield Ok::<_, Infallible>(Event::default().event("message").data(data));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE client lagged, skipped {n} events");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(15))).into_response()
}

async fn historical_logs_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(date): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    // Validate date format: YYYY-MM-DD
    if date.len() != 10
        || date.as_bytes()[4] != b'-'
        || date.as_bytes()[7] != b'-'
        || !date[0..4].chars().all(|c| c.is_ascii_digit())
        || !date[5..7].chars().all(|c| c.is_ascii_digit())
        || !date[8..10].chars().all(|c| c.is_ascii_digit())
    {
        return (StatusCode::BAD_REQUEST, "Invalid date format, expected YYYY-MM-DD").into_response();
    }

    let log_path = state.config.read().await.data_dir().join("capabilities_logs").join(format!("capability.log.{date}"));

    let contents = match tokio::fs::read_to_string(&log_path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return axum::Json(serde_json::json!([])).into_response();
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read log file: {e}")).into_response();
        }
    };

    let entries: Vec<serde_json::Value> = contents
        .lines()
        .filter_map(|line| {
            let obj: serde_json::Value = serde_json::from_str(line).ok()?;
            let timestamp = obj.get("timestamp")?.as_str()?.to_string();
            let level = obj.get("level")?.as_str()?.to_string();
            let message = obj.get("fields").and_then(|f| f.get("message")).and_then(|m| m.as_str()).unwrap_or("").to_string();
            let target = obj.get("target").and_then(|t| t.as_str()).unwrap_or("unknown").to_string();
            Some(serde_json::json!({
                "capability": target,
                "entry": {
                    "timestamp": timestamp,
                    "level": level,
                    "message": message,
                }
            }))
        })
        .collect();

    axum::Json(entries).into_response()
}

async fn get_settings_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }
    let config = state.config.read().await;
    axum::Json(config.clone()).into_response()
}

async fn put_settings_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: String,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }
    let new_config: CorreConfig = match serde_json::from_str(&body) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid config JSON: {e}")).into_response(),
    };
    if let Err(e) = new_config.save(&state.config_path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save config: {e}")).into_response();
    }
    *state.config.write().await = new_config;
    (StatusCode::OK, "Settings saved.").into_response()
}

// =========================================================================
// Registry & MCP management endpoints
// =========================================================================

/// GET /api/registry/catalog — return the full cached manifest.
async fn registry_catalog_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    match state.registry_client.get_manifest().await {
        Ok(manifest) => axum::Json(manifest).into_response(),
        Err(e) => (StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize, Default)]
struct SearchQuery {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    q: String,
}

/// GET /api/registry/search?q=... — search entries by name/description/tags.
async fn registry_search_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Response<Body> {
    let tq = TokenQuery { token: query.token.clone() };
    if let Err(resp) = require_auth(&state, &headers, &tq).await {
        return resp;
    }

    match state.registry_client.search(&query.q).await {
        Ok(results) => axum::Json(results).into_response(),
        Err(e) => (StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), e.to_string()).into_response(),
    }
}

/// POST /api/registry/refresh — force-refresh the registry cache.
async fn registry_refresh_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    match state.registry_client.refresh().await {
        Ok(manifest) => axum::Json(serde_json::json!({
            "ok": true,
            "server_count": manifest.servers.len(),
            "updated_at": manifest.updated_at,
        }))
        .into_response(),
        Err(e) => (StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), e.to_string()).into_response(),
    }
}

/// GET /api/mcp/installed — list installed MCP servers from per-MCP config files.
async fn mcp_installed_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    drop(config);

    let file_configs = match corre_core::config::load_mcp_configs(&mcp_dir) {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load MCP configs: {e}")).into_response(),
    };

    let servers: Vec<serde_json::Value> = file_configs
        .iter()
        .filter(|(_, cfg)| cfg.installed)
        .map(|(name, cfg)| {
            serde_json::json!({
                "name": name,
                "command": cfg.command,
                "args": cfg.args,
                "env": cfg.env,
                "registry_id": cfg.registry_id,
                "source": if cfg.registry_id.is_some() { "registry" } else { "manual" },
            })
        })
        .collect();

    axum::Json(servers).into_response()
}

#[derive(serde::Deserialize)]
struct InstallRequest {
    id: String,
    #[serde(default)]
    env_values: std::collections::HashMap<String, String>,
}

/// POST /api/mcp/install — install an MCP server from the registry.
async fn mcp_install_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: String,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let req: InstallRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid request: {e}")).into_response(),
    };

    // Look up the registry entry
    let entry = match state.registry_client.get_entry(&req.id).await {
        Ok(Some(e)) => e,
        Ok(None) => return (StatusCode::NOT_FOUND, format!("Registry entry `{}` not found", req.id)).into_response(),
        Err(e) => {
            return (StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), e.to_string()).into_response();
        }
    };

    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    drop(config);

    // Check if already installed via per-MCP config file
    if let Ok(existing) = corre_core::config::load_mcp_config_raw(&mcp_dir, &entry.id) {
        if existing.installed {
            return (StatusCode::CONFLICT, format!("`{}` is already installed", entry.id)).into_response();
        }
    }

    // Run the installer
    let server_config = match state.installer.install(&entry, &req.env_values).await {
        Ok(cfg) => cfg,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Install failed: {e}")).into_response(),
    };

    // Save the config with installed = true
    let file_config = corre_core::config::McpServerConfig {
        registry_id: server_config.registry_id.clone(),
        command: server_config.command.clone(),
        args: server_config.args.clone(),
        env: server_config.env.clone(),
        installed: true,
    };
    if let Err(e) = corre_core::config::save_mcp_config(&mcp_dir, &entry.id, &file_config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save MCP config: {e}")).into_response();
    }

    axum::Json(serde_json::json!({ "ok": true, "id": entry.id, "name": entry.name })).into_response()
}

/// POST /api/mcp/uninstall/{name} — uninstall an MCP server and mark config as not installed.
async fn mcp_uninstall_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    drop(config);

    // Load existing config file
    let mut file_config = match corre_core::config::load_mcp_config_raw(&mcp_dir, &name) {
        Ok(c) => c,
        Err(_) => return (StatusCode::NOT_FOUND, format!("MCP server `{name}` not found")).into_response(),
    };

    if !file_config.installed {
        return (StatusCode::NOT_FOUND, format!("MCP server `{name}` is not installed")).into_response();
    }

    // Run uninstall (binary/npm/pip cleanup)
    let server_config = file_config.clone();
    if let Err(e) = state.installer.uninstall(&name, &server_config).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Uninstall failed: {e}")).into_response();
    }

    // Mark as not installed (keep the file so settings are preserved)
    file_config.installed = false;
    if let Err(e) = corre_core::config::save_mcp_config(&mcp_dir, &name, &file_config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update MCP config: {e}")).into_response();
    }

    axum::Json(serde_json::json!({ "ok": true, "removed": name })).into_response()
}

/// POST /api/mcp/test/{name} — start the MCP server, list tools, shut down.
async fn mcp_test_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    let bin_dir = config.data_dir().join("bin");
    drop(config);

    // Load per-MCP config file (with interpolation for resolved env values)
    let file_configs = match corre_core::config::load_mcp_configs(&mcp_dir) {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load MCP configs: {e}")).into_response(),
    };

    let server_config = match file_configs.get(&name) {
        Some(cfg) => cfg.with_resolved_command(Some(&bin_dir)),
        None => return (StatusCode::NOT_FOUND, format!("MCP server `{name}` not found")).into_response(),
    };

    match corre_registry::tester::test_mcp_server(&name, &server_config).await {
        Ok(tools) => axum::Json(serde_json::json!({ "ok": true, "tools": tools })).into_response(),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e })).into_response(),
    }
}

/// GET /api/mcp/config/{name} — return raw config (un-interpolated, shows `${VAR}` refs).
async fn mcp_config_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    drop(config);

    match corre_core::config::load_mcp_config_raw(&mcp_dir, &name) {
        Ok(cfg) => axum::Json(cfg).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, format!("MCP config `{name}` not found")).into_response(),
    }
}

/// PUT /api/mcp/configure/{name} — save updated config vars to per-MCP config file.
async fn mcp_configure_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
    body: String,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let updates: corre_core::config::McpServerConfig = match serde_json::from_str(&body) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid config JSON: {e}")).into_response(),
    };

    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    drop(config);

    // Preserve the installed flag from the existing file
    let installed = corre_core::config::load_mcp_config_raw(&mcp_dir, &name).map(|c| c.installed).unwrap_or(updates.installed);

    let file_config = corre_core::config::McpServerConfig { installed, ..updates };

    if let Err(e) = corre_core::config::save_mcp_config(&mcp_dir, &name, &file_config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save MCP config: {e}")).into_response();
    }

    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

// =========================================================================
// Capability install/uninstall endpoints
// =========================================================================

/// GET /api/capabilities/installed — list installed plugin capabilities.
async fn capabilities_installed_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let installed: Vec<serde_json::Value> = state
        .plugins
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.manifest.plugin.name,
                "version": p.manifest.plugin.version,
                "description": p.manifest.plugin.description,
                "dir": p.dir.to_string_lossy(),
            })
        })
        .collect();

    axum::Json(installed).into_response()
}

#[derive(serde::Deserialize)]
struct CapabilityInstallRequest {
    id: String,
}

/// POST /api/capabilities/install — install a capability from the registry.
async fn capability_install_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: String,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let req: CapabilityInstallRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid request: {e}")).into_response(),
    };

    // Look up the capability in the registry
    let entry = match state.registry_client.lookup_capability(&req.id).await {
        Ok(Some(e)) => e,
        Ok(None) => return (StatusCode::NOT_FOUND, format!("Capability `{}` not found in registry", req.id)).into_response(),
        Err(e) => {
            return (StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), e.to_string()).into_response();
        }
    };

    // Check if already installed
    let config = state.config.read().await;
    let data_dir = config.data_dir();
    drop(config);

    let plugin_dir = data_dir.join("plugins").join(&req.id);
    if plugin_dir.join("manifest.toml").exists() {
        return (StatusCode::CONFLICT, format!("Capability `{}` is already installed", req.id)).into_response();
    }

    // Install the capability binary + manifest
    let (_, mcp_deps) = match state.installer.install_capability(&entry).await {
        Ok(result) => result,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Install failed: {e}")).into_response(),
    };

    // Auto-install any MCP dependencies that aren't already present
    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    drop(config);

    let mut installed_deps = Vec::new();
    for dep_id in &mcp_deps {
        // Skip if already installed
        if corre_core::config::load_mcp_config_raw(&mcp_dir, dep_id).is_ok_and(|c| c.installed) {
            continue;
        }
        // Try to find and install from registry
        if let Ok(Some(mcp_entry)) = state.registry_client.get_entry(dep_id).await {
            // Use the env var names as default values (user sets actual values later)
            let env_values: std::collections::HashMap<String, String> =
                mcp_entry.config.iter().map(|spec| (spec.name.clone(), format!("${{{}}}", spec.name))).collect();
            if let Ok(mcp_config) = state.installer.install(&mcp_entry, &env_values).await {
                let file_config = corre_core::config::McpServerConfig {
                    registry_id: mcp_config.registry_id,
                    command: mcp_config.command,
                    args: mcp_config.args,
                    env: mcp_config.env,
                    installed: true,
                };
                let _ = corre_core::config::save_mcp_config(&mcp_dir, dep_id, &file_config);
                installed_deps.push(dep_id.clone());
            }
        }
    }

    // Add to tracker so it appears in the dashboard immediately
    let cap_config = corre_core::config::CapabilityConfig {
        name: entry.id.clone(),
        description: entry.description.clone(),
        schedule: entry.manifest.defaults.schedule.clone().unwrap_or_default(),
        mcp_servers: entry.manifest.permissions.mcp_servers.clone(),
        config_path: entry.manifest.defaults.config_path.clone(),
        enabled: true,
        llm: None,
        plugin: Some(plugin_dir.to_string_lossy().into_owned()),
    };
    state.tracker.add_capability(&cap_config).await;

    axum::Json(serde_json::json!({
        "ok": true,
        "id": entry.id,
        "name": entry.name,
        "installed_mcp_deps": installed_deps,
        "restart_required": true,
    }))
    .into_response()
}

/// POST /api/capabilities/uninstall/{name} — uninstall a capability plugin.
async fn capability_uninstall_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let orphaned_deps = match state.installer.uninstall_capability(&name).await {
        Ok(deps) => deps,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Uninstall failed: {e}")).into_response(),
    };

    // Remove from tracker
    state.tracker.remove_capability(&name).await;

    axum::Json(serde_json::json!({
        "ok": true,
        "removed": name,
        "orphaned_mcp_deps": orphaned_deps,
        "restart_required": true,
    }))
    .into_response()
}

// =========================================================================
// System management endpoints
// =========================================================================

/// POST /api/system/restart — trigger a graceful shutdown (Docker restart policy brings us back).
async fn system_restart_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let _ = state.shutdown_signal.send(true);
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

// =========================================================================
// Service management endpoints
// =========================================================================

/// GET /api/services — list all managed services with status.
async fn services_list_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let services = state.service_manager.list().await;
    let body: Vec<serde_json::Value> =
        services.into_iter().map(|(name, status)| serde_json::json!({"name": name, "status": status})).collect();
    axum::Json(body).into_response()
}

#[derive(serde::Deserialize)]
struct ServiceStartRequest {
    name: String,
    description: String,
    image: String,
    #[serde(default)]
    ports: Vec<String>,
    #[serde(default)]
    volumes: Vec<String>,
    #[serde(default)]
    env: std::collections::HashMap<String, String>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    health_check: Option<String>,
}

/// POST /api/services/start — start a service from a declaration.
async fn service_start_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: String,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let req: ServiceStartRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid request: {e}")).into_response(),
    };

    let decl = corre_sdk::ServiceDeclaration {
        name: req.name,
        description: req.description,
        image: req.image,
        ports: req.ports,
        volumes: req.volumes,
        env: req.env,
        optional: req.optional,
        health_check: req.health_check,
    };

    let config = state.config.read().await;
    let data_dir = config.data_dir();
    let docker_registry = config.registry.docker_registry.clone();
    drop(config);

    match state.service_manager.start_service(&decl, &data_dir, &docker_registry).await {
        Ok(service) => axum::Json(serde_json::json!({
            "ok": true,
            "name": service.name,
            "container_id": service.container_id,
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to start service: {e}")).into_response(),
    }
}

/// POST /api/services/stop/{name} — stop a running service.
async fn service_stop_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    match state.service_manager.stop_service(&name).await {
        Ok(()) => axum::Json(serde_json::json!({"ok": true, "stopped": name})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to stop service: {e}")).into_response(),
    }
}

// =========================================================================
// Per-capability config editor endpoints
// =========================================================================

/// Resolve the config file read path for a capability.
///
/// Checks `{data_dir}/{cap_name}/{config_path}` first, falls back to the legacy
/// root location `{data_dir}/{config_path}`. Returns the scoped path when neither
/// exists so that writes create the file in the right place.
fn resolve_config_read_path(config: &CorreConfig, cap_name: &str) -> Option<std::path::PathBuf> {
    let data_dir = config.data_dir();
    let cap = config.capabilities.iter().find(|c| c.name == cap_name)?;
    let config_path = cap.config_path.as_ref()?;
    let scoped = data_dir.join(&cap.name).join(config_path);
    if scoped.exists() {
        return Some(scoped);
    }
    let legacy = data_dir.join(config_path);
    if legacy.exists() { Some(legacy) } else { Some(scoped) }
}

/// Resolve the config file write path — always targets the scoped location.
fn resolve_config_write_path(config: &CorreConfig, cap_name: &str) -> Option<std::path::PathBuf> {
    let data_dir = config.data_dir();
    let cap = config.capabilities.iter().find(|c| c.name == cap_name)?;
    let config_path = cap.config_path.as_ref()?;
    Some(data_dir.join(&cap.name).join(config_path))
}

/// GET /api/config/{name} — return the raw config file content for a capability.
async fn get_config_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }
    let config = state.config.read().await;
    let Some(config_path) = resolve_config_read_path(&config, &name) else {
        return (StatusCode::NOT_FOUND, "Capability not found or has no config_path").into_response();
    };
    drop(config);
    match std::fs::read_to_string(&config_path) {
        Ok(content) => (StatusCode::OK, content).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (StatusCode::OK, String::new()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read config: {e}")).into_response(),
    }
}

/// PUT /api/config/{name} — write config file content to disk for a capability.
async fn put_config_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
    body: String,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }
    let config = state.config.read().await;
    let Some(config_path) = resolve_config_write_path(&config, &name) else {
        return (StatusCode::NOT_FOUND, "Capability not found or has no config_path").into_response();
    };
    drop(config);
    if let Some(parent) = config_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create directory: {e}")).into_response();
        }
    }
    match std::fs::write(&config_path, &body) {
        Ok(()) => (StatusCode::OK, "Config saved.").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write config: {e}")).into_response(),
    }
}

async fn static_handler(Path(path): Path<String>) -> impl IntoResponse {
    match DashboardAssets::get(&path) {
        Some(file) => {
            let mime = match path.rsplit_once('.').map(|(_, ext)| ext) {
                Some("css") => "text/css",
                Some("js") => "application/javascript",
                Some("html") => "text/html",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("ico") => "image/x-icon",
                _ => "application/octet-stream",
            };
            ([(header::CONTENT_TYPE, mime)], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Spawn a background task that broadcasts SystemMetrics every second while there are SSE subscribers.
pub fn spawn_metrics_broadcaster(tracker: Arc<ExecutionTracker>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let metrics = tracker.system_metrics();
            let event = DashboardEvent::SystemMetrics(metrics);
            // If nobody is subscribed, send returns Err — that's fine, just skip.
            let _ = tracker.event_sender().send(event);
        }
    });
}
