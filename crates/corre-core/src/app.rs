//! Core traits and execution context for the Corre app system.
//!
//! Defines `App`, `McpCaller`, `LlmProvider`, `AppContext`, and `ProgressTracker`.
//! These abstractions decouple app implementations from the concrete MCP and LLM crates,
//! allowing the safety layer to wrap both without changes to app code.

// Re-export types from corre-sdk so downstream crates keep their imports.
pub use corre_sdk::llm::{LlmMessage, LlmRequest, LlmResponse, LlmRole};
pub use corre_sdk::types::{AppManifest, AppOutput, ContentType, CustomContent};

use chrono::{DateTime, Utc};
use corre_sdk::types::{Article, Section};
use std::path::PathBuf;

/// Error type for MCP tool calls, distinguishing tool-level errors from protocol failures.
#[derive(Debug, thiserror::Error)]
pub enum McpCallError {
    /// The MCP tool reported an error via `is_error: true` in CallToolResult.
    #[error("tool `{tool}` on `{server}` returned an error: {message}")]
    ToolError { server: String, tool: String, message: String },

    /// Protocol-level or transport failure (connection lost, JSON-RPC error, etc.)
    #[error(transparent)]
    Protocol(#[from] anyhow::Error),
}

/// Trait for calling tools on MCP servers, decoupling corre-core from corre-mcp.
#[async_trait::async_trait]
pub trait McpCaller: Send + Sync {
    async fn call_tool(&self, server_name: &str, tool_name: &str, args: serde_json::Value) -> Result<serde_json::Value, McpCallError>;
    async fn list_tools(&self, server_name: &str) -> Result<Vec<String>, McpCallError>;
}

/// Trait for LLM completions, decoupling corre-core from corre-llm.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse>;
}

/// A progress or log event emitted by an app during execution.
///
/// Sent through `AppContext::progress_tx` so the orchestrator can
/// forward real-time updates to the dashboard.
pub enum ProgressEvent {
    Progress { pct: Option<u8>, phase: String },
    Log { level: String, message: String },
}

/// Runtime context provided to an app during execution.
pub struct AppContext {
    pub mcp: Box<dyn McpCaller>,
    pub llm: Box<dyn LlmProvider>,
    pub config_dir: PathBuf,
    /// Maximum concurrent LLM requests (from `llm.max_concurrent` in config).
    pub max_concurrent_llm: usize,
    /// Source URLs from all previously published editions, for cross-edition deduplication.
    pub seen_urls: std::collections::HashSet<String>,
    /// Optional channel for forwarding progress/log events to the dashboard.
    pub progress_tx: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
}

/// Result of polling an app after its initial timeout elapses.
pub enum ProgressStatus {
    /// Still working. Optional hint: percentage complete (0-100).
    StillBusy(Option<u8>),
    /// Finished (or has enough partial data). Contains the output to publish.
    Done(AppOutput),
    /// Stuck with no useful output. Kill the app.
    Stuck,
}

/// Thread-safe progress tracker that apps update as they work.
///
/// The orchestrator calls [`ProgressTracker::evaluate`] to decide whether an
/// app that has exceeded its timeout is still making progress, has
/// partial results worth publishing, or is stuck.
pub struct ProgressTracker {
    inner: std::sync::Mutex<ProgressState>,
}

struct ProgressState {
    last_activity: DateTime<Utc>,
    phase: &'static str,
    completed_articles: Vec<(String, Article)>,
    total_expected: usize,
    app_name: String,
}

impl ProgressTracker {
    pub fn new(app_name: &str) -> Self {
        Self {
            inner: std::sync::Mutex::new(ProgressState {
                last_activity: Utc::now(),
                phase: "init",
                completed_articles: Vec::new(),
                total_expected: 0,
                app_name: app_name.to_string(),
            }),
        }
    }

    /// Reset all state (call at the start of each execution).
    pub fn reset(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.last_activity = Utc::now();
        state.phase = "init";
        state.completed_articles.clear();
        state.total_expected = 0;
    }

    /// Record activity in the given phase.
    pub fn touch(&self, phase: &'static str) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.last_activity = Utc::now();
        state.phase = phase;
    }

    /// Set the total number of articles expected (after scoring).
    pub fn set_expected(&self, n: usize) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.total_expected = n;
        state.last_activity = Utc::now();
    }

    /// Record a completed article.
    pub fn add_article(&self, section: String, article: Article) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.completed_articles.push((section, article));
        state.last_activity = Utc::now();
    }

    /// Evaluate whether the app is still making progress.
    ///
    /// - If `last_activity` is within `staleness_threshold` → [`ProgressStatus::StillBusy`]
    /// - If stale but has articles → [`ProgressStatus::Done`] with partial output
    /// - If stale with no articles → [`ProgressStatus::Stuck`]
    pub fn evaluate(&self, staleness_threshold: std::time::Duration) -> ProgressStatus {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let elapsed = Utc::now().signed_duration_since(state.last_activity);
        let is_stale = elapsed > chrono::Duration::from_std(staleness_threshold).unwrap_or(chrono::Duration::MAX);

        if !is_stale {
            let hint = if state.total_expected > 0 {
                Some((state.completed_articles.len() * 100 / state.total_expected).min(99) as u8)
            } else {
                None
            };
            tracing::info!(
                "ProgressTracker[{}]: phase={}, {}/{} articles, last activity {elapsed} ago — still busy",
                state.app_name,
                state.phase,
                state.completed_articles.len(),
                state.total_expected,
            );
            return ProgressStatus::StillBusy(hint);
        }

        if !state.completed_articles.is_empty() {
            tracing::info!(
                "ProgressTracker[{}]: stale ({elapsed} ago) but has {} articles — returning partial results",
                state.app_name,
                state.completed_articles.len(),
            );
            let mut article_map: std::collections::HashMap<String, Vec<Article>> = std::collections::HashMap::new();
            for (section, article) in &state.completed_articles {
                article_map.entry(section.clone()).or_default().push(article.clone());
            }
            let sections: Vec<Section> = article_map.into_iter().map(|(title, articles)| Section { title, articles }).collect();
            return ProgressStatus::Done(AppOutput {
                app_name: state.app_name.clone(),
                produced_at: Utc::now(),
                sections,
                content_type: ContentType::default(),
                custom_content: None,
            });
        }

        tracing::warn!("ProgressTracker[{}]: stale ({elapsed} ago) with no articles — stuck", state.app_name,);
        ProgressStatus::Stuck
    }
}

impl From<&crate::config::AppConfig> for AppManifest {
    fn from(c: &crate::config::AppConfig) -> Self {
        Self {
            name: c.name.clone(),
            description: c.description.clone(),
            schedule: c.schedule.clone(),
            mcp_servers: c.mcp_servers.clone(),
            config_path: c.config_path.clone(),
        }
    }
}

/// Trait implemented by each app (daily brief, rolodex, etc.).
#[async_trait::async_trait]
pub trait App: Send + Sync {
    fn manifest(&self) -> &AppManifest;
    async fn execute(&self, ctx: &AppContext) -> anyhow::Result<AppOutput>;

    /// Called by the orchestrator after the initial timeout elapses.
    /// The default returns `StillBusy(None)`, granting another full timeout period.
    async fn in_progress(&self) -> ProgressStatus {
        ProgressStatus::StillBusy(None)
    }
}
