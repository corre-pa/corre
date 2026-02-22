use crate::cache::EditionCache;
use crate::render::{NewspaperTemplate, SettingsTemplate, TopicsTemplate};
use crate::search::SearchIndex;
use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use chrono::NaiveDate;
use corre_core::config::CorreConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;

pub struct AppState {
    pub cache: Arc<EditionCache>,
    pub search: SearchIndex,
    pub static_dir: PathBuf,
    pub config_path: PathBuf,
    pub config: Arc<RwLock<CorreConfig>>,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/edition/{date}", get(edition_handler))
        .route("/api/dates", get(dates_handler))
        .route("/search", get(search_handler))
        .route("/settings", get(settings_page_handler))
        .route("/settings/topics", get(topics_page_handler))
        .route("/api/settings", get(get_settings_handler).put(put_settings_handler))
        .route("/api/topics", get(get_topics_handler).put(put_topics_handler))
        .nest_service("/static", ServeDir::new(&state.static_dir))
        .with_state(state)
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

async fn index_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let title = state.config.read().await.news.title.clone();
    match state.cache.latest().await {
        Some(edition) => {
            let template = NewspaperTemplate { title: &title, edition: &edition, version: crate::render::VERSION };
            match template.render() {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
            }
        }
        None => Html(no_editions_page(&title)).into_response(),
    }
}

async fn edition_handler(State(state): State<Arc<AppState>>, Path(date_str): Path<String>) -> impl IntoResponse {
    let title = state.config.read().await.news.title.clone();
    let date = match NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid date format. Use YYYY-MM-DD".to_string()).into_response(),
    };

    match state.cache.load_date(date).await {
        Some(edition) => {
            let template = NewspaperTemplate { title: &title, edition: &edition, version: crate::render::VERSION };
            match template.render() {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
            }
        }
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
    match state.search.search(&params.q, params.limit) {
        Ok(results) => axum::Json(results).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Search error: {e}")).into_response(),
    }
}

// --- Settings handlers ---

async fn settings_page_handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    let title = config.news.title.clone();
    if let Err(e) = check_auth(&config.news.editor_token, token_str.as_deref()) {
        return match e {
            AuthError::NotConfigured => login_page(
                &title,
                "/settings",
                Some("Editor access not configured. Set <code>editor_token</code> in the <code>[news]</code> section of corre.toml."),
            ),
            AuthError::InvalidToken if token_str.is_some() => login_page(&title, "/settings", Some("Incorrect token.")),
            AuthError::InvalidToken => login_page(&title, "/settings", None),
        };
    }
    let token = token_str.unwrap_or_default();
    let config_json = match serde_json::to_string_pretty(&*config) {
        Ok(j) => j,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Serialization error: {e}")).into_response(),
    };
    let template = SettingsTemplate { title: &title, config_json: &config_json, token: &token };
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
    }
}

async fn get_settings_handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if let Err(e) = check_auth(&config.news.editor_token, token_str.as_deref()) {
        return auth_error_response(e);
    }
    axum::Json(config.clone()).into_response()
}

async fn put_settings_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: String,
) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    {
        let config = state.config.read().await;
        if let Err(e) = check_auth(&config.news.editor_token, token_str.as_deref()) {
            return auth_error_response(e);
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

// --- Topics handlers ---

async fn topics_page_handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    let title = config.news.title.clone();
    if let Err(e) = check_auth(&config.news.editor_token, token_str.as_deref()) {
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
    let topics_path = resolve_topics_path(&config, &state.config_path);
    let topics_yaml = std::fs::read_to_string(&topics_path).unwrap_or_default();
    let topics_value: serde_json::Value =
        serde_yaml_ng::from_str(&topics_yaml).unwrap_or_else(|_| serde_json::json!({"daily-briefing": {"sections": []}}));
    let topics_json = serde_json::to_string(&topics_value).unwrap_or_else(|_| r#"{"daily-briefing":{"sections":[]}}"#.to_string());
    let template = TopicsTemplate { title: &title, topics_json: &topics_json, token: &token };
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
    }
}

async fn get_topics_handler(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(query): Query<TokenQuery>) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if let Err(e) = check_auth(&config.news.editor_token, token_str.as_deref()) {
        return auth_error_response(e);
    }
    let topics_path = resolve_topics_path(&config, &state.config_path);
    match std::fs::read_to_string(&topics_path) {
        Ok(content) => (StatusCode::OK, content).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read topics: {e}")).into_response(),
    }
}

async fn put_topics_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
    body: String,
) -> Response<Body> {
    let token_str = extract_token(&headers, &query);
    let config = state.config.read().await;
    if let Err(e) = check_auth(&config.news.editor_token, token_str.as_deref()) {
        return auth_error_response(e);
    }
    let topics_path = resolve_topics_path(&config, &state.config_path);
    if let Err(e) = std::fs::write(&topics_path, &body) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write topics: {e}")).into_response();
    }
    (StatusCode::OK, "Topics saved.").into_response()
}

/// Resolve the topics file path from the first capability that has a config_path,
/// relative to the directory containing corre.toml.
fn resolve_topics_path(config: &CorreConfig, config_path: &std::path::Path) -> PathBuf {
    let base_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
    config
        .capabilities
        .iter()
        .find_map(|c| c.config_path.as_ref())
        .map(|p| base_dir.join(p))
        .unwrap_or_else(|| base_dir.join("config/topics.md"))
}

/// Start the web server, binding to the given address.
pub async fn serve(state: Arc<AppState>, addr: std::net::SocketAddr) -> anyhow::Result<()> {
    let router = build_router(state);
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
