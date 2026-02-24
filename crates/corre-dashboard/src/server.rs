use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::Event;
use axum::response::{Html, IntoResponse, Response, Sse};
use axum::routing::{get, post};
use corre_core::config::CorreConfig;
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
}

pub struct DashboardState {
    pub tracker: Arc<ExecutionTracker>,
    pub config: Arc<RwLock<CorreConfig>>,
    pub config_path: std::path::PathBuf,
    pub run_trigger: mpsc::Sender<String>,
    pub registry_client: Arc<RegistryClient>,
    pub installer: Arc<McpInstaller>,
}

pub fn build_router(state: Arc<DashboardState>) -> Router {
    Router::new()
        .route("/dashboard", get(dashboard_page_handler))
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
        .route("/api/mcp/deps/{id}", get(mcp_deps_handler))
        .route("/dashboard/static/{*path}", get(static_handler))
        .with_state(state)
}

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
    check_auth(&config.news.editor_token, token_str.as_deref()).map_err(|s| s.into_response())?;
    Ok(token_str.unwrap_or_default())
}

async fn dashboard_page_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    let title = config.news.title.clone();
    if check_auth(&config.news.editor_token, token_str.as_deref()).is_err() {
        return (StatusCode::FORBIDDEN, "Invalid or missing editor token. Add ?token=YOUR_TOKEN to the URL.").into_response();
    }
    let token = token_str.unwrap_or_default();
    let config_json = match serde_json::to_string_pretty(&*config) {
        Ok(j) => j,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Serialization error: {e}")).into_response(),
    };
    let template = DashboardTemplate { title: &title, token: &token, config_json: &config_json };
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
    }
}

async fn status_handler(State(state): State<Arc<DashboardState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if check_auth(&config.news.editor_token, token_str.as_deref()).is_err() {
        return StatusCode::FORBIDDEN.into_response();
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
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if check_auth(&config.news.editor_token, token_str.as_deref()).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    drop(config);

    if state.tracker.is_running(&name).await {
        return (StatusCode::CONFLICT, format!("Capability `{name}` is already running")).into_response();
    }

    match state.run_trigger.try_send(name.clone()) {
        Ok(()) => (StatusCode::ACCEPTED, format!("Capability `{name}` triggered")).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Run queue is full, try again later").into_response(),
    }
}

async fn sse_handler(State(state): State<Arc<DashboardState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if check_auth(&config.news.editor_token, token_str.as_deref()).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    drop(config);

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
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if check_auth(&config.news.editor_token, token_str.as_deref()).is_err() {
        return StatusCode::FORBIDDEN.into_response();
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

    let log_path = config.data_dir().join("capabilities_logs").join(format!("capability.log.{date}"));
    drop(config);

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
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if check_auth(&config.news.editor_token, token_str.as_deref()).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    axum::Json(config.clone()).into_response()
}

async fn put_settings_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: String,
) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    {
        let config = state.config.read().await;
        if check_auth(&config.news.editor_token, token_str.as_deref()).is_err() {
            return StatusCode::FORBIDDEN.into_response();
        }
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

    // Save per-MCP config file with installed = true
    let file_config = corre_core::config::McpServerFileConfig {
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
    let server_config = file_config.to_server_config();
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
        Some(cfg) => cfg.to_server_config_with_bin_dir(Some(&bin_dir)),
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

    let updates: corre_core::config::McpServerFileConfig = match serde_json::from_str(&body) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("Invalid config JSON: {e}")).into_response(),
    };

    let config = state.config.read().await;
    let mcp_dir = config.mcp_dir();
    drop(config);

    // Preserve the installed flag from the existing file
    let installed = corre_core::config::load_mcp_config_raw(&mcp_dir, &name).map(|c| c.installed).unwrap_or(updates.installed);

    let file_config = corre_core::config::McpServerFileConfig { installed, ..updates };

    if let Err(e) = corre_core::config::save_mcp_config(&mcp_dir, &name, &file_config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save MCP config: {e}")).into_response();
    }

    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

/// GET /api/mcp/deps/{id} — check dependencies for a registry entry.
async fn mcp_deps_handler(
    State(state): State<Arc<DashboardState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    Path(id): Path<String>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&state, &headers, &query).await {
        return resp;
    }

    let entry = match state.registry_client.get_entry(&id).await {
        Ok(Some(e)) => e,
        Ok(None) => return (StatusCode::NOT_FOUND, format!("Registry entry `{id}` not found")).into_response(),
        Err(e) => {
            return (StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), e.to_string()).into_response();
        }
    };

    let results = corre_registry::deps::check_deps(&entry.dependencies).await;
    axum::Json(results).into_response()
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
