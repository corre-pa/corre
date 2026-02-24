//! Axum HTTP server: route definitions, request handlers, and server startup.
//!
//! Builds the `Router` that serves the newspaper UI, edition API, full-text search,
//! token-gated settings pages, and embedded/plugin static assets.

use crate::cache::EditionCache;
use crate::config::NewsConfig;
use crate::render::{NewspaperTemplate, TopicsTemplate};
use crate::search::SearchIndex;
use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use chrono::NaiveDate;
use corre_core::config::CorreConfig;
use rust_embed::RustEmbed;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(RustEmbed)]
#[folder = "../../static/"]
struct Assets;

pub struct AppState {
    pub cache: Arc<EditionCache>,
    pub search: Option<SearchIndex>,
    pub config_path: PathBuf,
    pub config: Arc<RwLock<CorreConfig>>,
    /// Data directory for resolving plugin static assets.
    pub data_dir: PathBuf,
}

impl AppState {
    /// Parse the `[news]` section from the config into a `NewsConfig`.
    async fn news_config(&self) -> NewsConfig {
        let config = self.config.read().await;
        NewsConfig::from_toml_table(Some(&config.news))
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/edition/{date}", get(edition_handler))
        .route("/api/dates", get(dates_handler))
        .route("/search", get(search_handler))
        .route("/settings/topics", get(topics_page_handler))
        .route("/api/topics", get(get_topics_handler).put(put_topics_handler))
        .route("/plugin/{name}/static/{*path}", get(plugin_static_handler))
        .route("/static/{*path}", get(static_handler))
        .with_state(state)
}

async fn static_handler(Path(path): Path<String>) -> impl IntoResponse {
    match Assets::get(&path) {
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

/// Serve a static asset from a plugin's `static/` directory.
async fn plugin_static_handler(State(state): State<Arc<AppState>>, Path((name, path)): Path<(String, String)>) -> impl IntoResponse {
    // Prevent path traversal
    if name.contains("..") || path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let file_path = state.data_dir.join("plugins").join(&name).join("static").join(&path);
    if !file_path.exists() || !file_path.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mime = match file_path.extension().and_then(|e| e.to_str()) {
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("html") => "text/html",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    };
    match std::fs::read(&file_path) {
        Ok(data) => ([(header::CONTENT_TYPE, mime)], data).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Extract a bearer token from either `?token=` query param or `Authorization: Bearer` header.
fn extract_token(headers: &HeaderMap, query: &TokenQuery) -> Option<String> {
    if let Some(ref t) = query.token {
        return Some(t.clone());
    }
    headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(|s| s.to_string())
}

enum AuthError {
    /// editor_token is not set in config at all
    NotConfigured,
    /// Token was missing or wrong
    InvalidToken,
}

/// Check that the provided token matches the configured editor_token.
fn check_auth(editor_token: &Option<String>, provided: Option<&str>) -> Result<(), AuthError> {
    let expected = editor_token.as_deref().ok_or(AuthError::NotConfigured)?;
    match provided {
        Some(t) if t == expected => Ok(()),
        _ => Err(AuthError::InvalidToken),
    }
}

/// Return a 403 JSON-ish response for API endpoints.
fn auth_error_response(err: AuthError) -> Response<Body> {
    match err {
        AuthError::NotConfigured => {
            (StatusCode::FORBIDDEN, "Editor access not configured. Set editor_token in [news] section of corre.toml.").into_response()
        }
        AuthError::InvalidToken => (StatusCode::FORBIDDEN, "Invalid or missing editor token.").into_response(),
    }
}

/// Return a login page for browser-facing endpoints.
fn login_page(title: &str, path: &str, error_msg: Option<&str>) -> Response<Body> {
    let error_html = error_msg.map(|msg| format!(r#"<p class="login-error">{msg}</p>"#)).unwrap_or_default();
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title} — Sign in</title>
    <link rel="stylesheet" href="/static/style.css">
    <link rel="stylesheet" href="/static/settings.css">
    <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Manufacturing+Consent&display=swap">
</head>
<body>
    <header class="masthead">
        <div class="masthead-rule"></div>
        <h1 class="masthead-title"><a href="/">{title}</a></h1>
        <div class="masthead-meta"><span>Settings</span></div>
        <div class="masthead-rule"></div>
    </header>
    <main class="settings-main">
        <form class="login-form" method="get" action="{path}">
            <fieldset>
                <legend>Editor access</legend>
                {error_html}
                <div class="field">
                    <label for="token">Token</label>
                    <input type="password" id="token" name="token" autocomplete="off" autofocus required>
                </div>
                <button type="submit" class="btn btn-primary">Sign in</button>
            </fieldset>
        </form>
    </main>
</body>
</html>"#
    );
    Html(html).into_response()
}

#[derive(serde::Deserialize, Default)]
struct TokenQuery {
    #[serde(default)]
    token: Option<String>,
}

// --- Existing handlers ---

/// Render an edition as HTML, handling both newspaper and custom content types.
fn render_edition(title: &str, edition: &crate::edition::Edition) -> Response<Body> {
    use crate::edition::ContentType;

    match edition.content_type {
        ContentType::Custom => {
            if let Some(ref custom) = edition.custom_content {
                let sanitized_html = crate::edition::sanitize_custom_html(&custom.html);
                let css_block = custom.css.as_deref().map(|css| format!("<style>{css}</style>")).unwrap_or_default();
                let page = format!(
                    r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="/static/style.css">
    {css_block}
</head>
<body>
    <header class="masthead">
        <div class="masthead-rule"></div>
        <h1 class="masthead-title"><a href="/">{title}</a></h1>
        <div class="masthead-meta">
            <span>{date}</span>
        </div>
        <div class="masthead-rule"></div>
    </header>
    <main>
        <div class="plugin-content">
            {sanitized_html}
        </div>
    </main>
    <footer class="newspaper-footer">
        <p>{title} &mdash; v{version}</p>
    </footer>
</body>
</html>"#,
                    date = edition.date,
                    version = crate::render::VERSION,
                );
                Html(page).into_response()
            } else {
                // Custom content type but no custom_content — fall back to newspaper
                render_newspaper(title, edition)
            }
        }
        ContentType::Newspaper => render_newspaper(title, edition),
    }
}

fn render_newspaper(title: &str, edition: &crate::edition::Edition) -> Response<Body> {
    let template = NewspaperTemplate { title, edition, version: crate::render::VERSION };
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
    }
}

async fn index_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let news = state.news_config().await;
    match state.cache.latest().await {
        Some(edition) => render_edition(&news.title, &edition),
        None => Html(no_editions_page(&news.title)).into_response(),
    }
}

async fn edition_handler(State(state): State<Arc<AppState>>, Path(date_str): Path<String>) -> impl IntoResponse {
    let title = state.news_config().await.title;
    let date = match NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid date format. Use YYYY-MM-DD".to_string()).into_response(),
    };

    match state.cache.load_date(date).await {
        Some(edition) => render_edition(&title, &edition),
        None => (StatusCode::NOT_FOUND, format!("No edition for {date}")).into_response(),
    }
}

async fn dates_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let dates = state.cache.list_dates().await;
    let date_strings: Vec<String> = dates.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect();
    axum::Json(date_strings).into_response()
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

async fn search_handler(State(state): State<Arc<AppState>>, Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let Some(ref search) = state.search else {
        return axum::Json(Vec::<crate::search::SearchResult>::new()).into_response();
    };
    match search.search(&params.q, params.limit) {
        Ok(results) => axum::Json(results).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Search error: {e}")).into_response(),
    }
}

// --- Topics handlers ---

#[derive(serde::Deserialize, Default)]
struct TopicsQuery {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    cap: Option<String>,
}

async fn topics_page_handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(query): Query<TopicsQuery>) -> Response<Body> {
    let token_query = TokenQuery { token: query.token.clone() };
    let token_str = extract_token(&headers, &token_query);
    let config = state.config.read().await;
    let news = NewsConfig::from_toml_table(Some(&config.news));
    let title = news.title.clone();
    if let Err(e) = check_auth(&news.editor_token, token_str.as_deref()) {
        return match e {
            AuthError::NotConfigured => login_page(
                &title,
                "/settings/topics",
                Some("Editor access not configured. Set <code>editor_token</code> in the <code>[news]</code> section of corre.toml."),
            ),
            AuthError::InvalidToken if token_str.is_some() => login_page(&title, "/settings/topics", Some("Incorrect token.")),
            AuthError::InvalidToken => login_page(&title, "/settings/topics", None),
        };
    }
    let token = token_str.unwrap_or_default();
    let topics_path = resolve_topics_path(&config, query.cap.as_deref());
    let topics_yaml = std::fs::read_to_string(&topics_path).unwrap_or_default();
    let empty_default = || serde_json::json!({"daily-briefing": {"sections": []}});
    let topics_value: serde_json::Value =
        serde_yaml_ng::from_str(&topics_yaml).ok().filter(|v: &serde_json::Value| !v.is_null()).unwrap_or_else(empty_default);
    let topics_json = serde_json::to_string(&topics_value).unwrap_or_else(|_| r#"{"daily-briefing":{"sections":[]}}"#.to_string());
    let template = TopicsTemplate { title: &title, topics_json: &topics_json, token: &token };
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
    }
}

async fn get_topics_handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(query): Query<TopicsQuery>) -> Response<Body> {
    let token_query = TokenQuery { token: query.token.clone() };
    let token_str = extract_token(&headers, &token_query);
    let config = state.config.read().await;
    let news = NewsConfig::from_toml_table(Some(&config.news));
    if let Err(e) = check_auth(&news.editor_token, token_str.as_deref()) {
        return auth_error_response(e);
    }
    let topics_path = resolve_topics_path(&config, query.cap.as_deref());
    tracing::info!("Reading topics from {}", topics_path.display());
    match std::fs::read_to_string(&topics_path) {
        Ok(content) => {
            tracing::info!("Successfully read topics ({} bytes)", content.len());
            (StatusCode::OK, content).into_response()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("No topics file at {}, returning empty", topics_path.display());
            (StatusCode::OK, String::new()).into_response()
        }
        Err(e) => {
            tracing::info!("Failed to read topics from {}: {e}", topics_path.display());
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read topics: {e}")).into_response()
        }
    }
}

async fn put_topics_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TopicsQuery>,
    body: String,
) -> Response<Body> {
    let token_query = TokenQuery { token: query.token.clone() };
    let token_str = extract_token(&headers, &token_query);
    let config = state.config.read().await;
    let news = NewsConfig::from_toml_table(Some(&config.news));
    if let Err(e) = check_auth(&news.editor_token, token_str.as_deref()) {
        return auth_error_response(e);
    }
    let topics_path = resolve_topics_write_path(&config, query.cap.as_deref());
    if let Some(parent) = topics_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create directory: {e}")).into_response();
        }
    }
    if let Err(e) = std::fs::write(&topics_path, &body) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write topics: {e}")).into_response();
    }
    (StatusCode::OK, "Topics saved.").into_response()
}

/// Resolve the topics file path for a capability under the plugin data directory.
///
/// If `cap_name` is given, looks up that capability; otherwise picks the first
/// capability that declares a `config_path`. The primary path is
/// `{data_dir}/{capability.name}/{config_path}`. If that doesn't exist yet,
/// falls back to the legacy root location `{data_dir}/{config_path}` (from
/// before the plugin architecture). Writes always target the primary path.
fn resolve_topics_path(config: &CorreConfig, cap_name: Option<&str>) -> PathBuf {
    let data_dir = config.data_dir();
    let cap = match cap_name {
        Some(name) => config.capabilities.iter().find(|c| c.name == name),
        None => config.capabilities.iter().find(|c| c.config_path.is_some()),
    };
    match cap.and_then(|c| c.config_path.as_ref().map(|p| (c, p))) {
        Some((c, p)) => {
            let scoped = data_dir.join(&c.name).join(p);
            if scoped.exists() {
                return scoped;
            }
            // Fall back to legacy root location for reading
            let legacy = data_dir.join(p);
            if legacy.exists() { legacy } else { scoped }
        }
        None => data_dir.join("config/topics.yml"),
    }
}

/// Resolve the topics path for writing — always uses the scoped location.
fn resolve_topics_write_path(config: &CorreConfig, cap_name: Option<&str>) -> PathBuf {
    let data_dir = config.data_dir();
    let cap = match cap_name {
        Some(name) => config.capabilities.iter().find(|c| c.name == name),
        None => config.capabilities.iter().find(|c| c.config_path.is_some()),
    };
    match cap.and_then(|c| c.config_path.as_ref().map(|p| (c, p))) {
        Some((c, p)) => data_dir.join(&c.name).join(p),
        None => data_dir.join("config/topics.yml"),
    }
}

/// Start the web server, binding to the given address.
pub async fn serve(state: Arc<AppState>, addr: std::net::SocketAddr) -> anyhow::Result<()> {
    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

/// Start the web server with additional routes merged in (e.g. dashboard).
pub async fn serve_with_extra_routes(state: Arc<AppState>, extra: Router, addr: std::net::SocketAddr) -> anyhow::Result<()> {
    let router = build_router(state).merge(extra);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

fn no_editions_page(title: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head><title>{title}</title><link rel="stylesheet" href="/static/style.css"></head>
<body>
<header><h1>{title}</h1></header>
<main><p class="no-editions">No editions yet. Run a capability to generate your first edition.</p></main>
</body>
</html>"#
    )
}
