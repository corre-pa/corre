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
}

pub fn build_router(state: Arc<DashboardState>) -> Router {
    Router::new()
        .route("/dashboard", get(dashboard_page_handler))
        .route("/api/dashboard/status", get(status_handler))
        .route("/api/dashboard/run/{name}", post(run_now_handler))
        .route("/api/dashboard/events", get(sse_handler))
        .route("/api/settings", get(get_settings_handler).put(put_settings_handler))
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
