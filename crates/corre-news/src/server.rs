use crate::cache::EditionCache;
use crate::render::NewspaperTemplate;
use crate::search::SearchIndex;
use askama::Template;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use chrono::NaiveDate;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;

pub struct AppState {
    pub cache: Arc<EditionCache>,
    pub search: SearchIndex,
    pub title: String,
    pub static_dir: PathBuf,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/edition/{date}", get(edition_handler))
        .route("/api/dates", get(dates_handler))
        .route("/search", get(search_handler))
        .nest_service("/static", ServeDir::new(&state.static_dir))
        .with_state(state)
}

async fn index_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.cache.latest().await {
        Some(edition) => {
            let template = NewspaperTemplate { title: &state.title, edition: &edition };
            match template.render() {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Template error: {e}")).into_response(),
            }
        }
        None => Html(no_editions_page(&state.title)).into_response(),
    }
}

async fn edition_handler(State(state): State<Arc<AppState>>, Path(date_str): Path<String>) -> impl IntoResponse {
    let date = match NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid date format. Use YYYY-MM-DD".to_string()).into_response(),
    };

    match state.cache.load_date(date).await {
        Some(edition) => {
            let template = NewspaperTemplate { title: &state.title, edition: &edition };
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
