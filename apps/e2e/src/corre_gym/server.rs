//! Spawns a real corre-gym HTTP server backed by an in-memory DB for a single scenario.

use std::sync::Arc;

use anyhow::Context as _;
use corre_core::app::LlmProvider;
use corre_core::config::CorreConfig;
use corre_gym::assistant::AssistantHandler;
use corre_gym::config::GymConfig;
use corre_gym::db::Database;
use corre_gym::web::{AppState, build_router};
use corre_llm::OpenAiCompatProvider;
use dashmap::DashMap;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Stable signing key for session cookies minted in tests. Cookies signed with this key
/// verify against the in-process `AppState` because `session_secret` is set to the same
/// value in `TestServer::spawn`.
const TEST_SESSION_SECRET: &str = "e2e-test-session-secret-do-not-use-in-prod-please-i-mean-it-32b";

/// A running corre-gym server tied to a single cucumber scenario.
pub struct TestServer {
    pub base_url: String,
    pub db: Arc<Mutex<Database>>,
    pub app_state: Arc<AppState>,
    /// Kept alive for the lifetime of the scenario; aborted on drop.
    task: JoinHandle<()>,
}

impl TestServer {
    /// Build the full corre-gym stack (DB, LLM, handler, router), bind a random
    /// loopback port, and spawn `axum::serve` on a tokio task.
    pub async fn spawn(corre_cfg: &CorreConfig) -> anyhow::Result<Self> {
        let db = Arc::new(Mutex::new(Database::open_in_memory().context("opening in-memory DB")?));

        let llm = build_llm_provider(corre_cfg).context("building LLM provider")?;

        let mut gym_cfg = GymConfig::from_toml_table(Some(&corre_cfg.gym)).context("parsing [gym] section from test corre.toml")?;
        gym_cfg.resolve_secrets().context("resolving gym secrets")?;
        gym_cfg.voice = None;

        let handler = Arc::new(AssistantHandler::new(db.clone(), llm, gym_cfg.clone()).await.context("constructing AssistantHandler")?);

        let app_state = Arc::new(AppState {
            db: db.clone(),
            handler: Some(handler),
            config: gym_cfg,
            bot_username: "e2e_test_bot".to_string(),
            session_secret: Some(TEST_SESSION_SECRET.to_string()),
            chat_rate_limiter: DashMap::new(),
        });

        let listener = TcpListener::bind("127.0.0.1:0").await.context("binding listener")?;
        let addr = listener.local_addr()?;

        let router = build_router(app_state.clone());
        let task = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                tracing::warn!("test axum::serve exited with error: {e:#}");
            }
        });

        let base_url = format!("http://{addr}");
        tracing::info!(base_url, "test corre-gym server spawned");
        Ok(Self { base_url, db, app_state, task })
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Mirror of `corre-gym/src/main.rs` LLM construction: real provider + optional safety wrap.
fn build_llm_provider(cfg: &CorreConfig) -> anyhow::Result<Box<dyn LlmProvider>> {
    let raw: Box<dyn LlmProvider> = Box::new(OpenAiCompatProvider::from_config(&cfg.llm)?);
    if cfg.safety.enabled { Ok(Box::new(corre_safety::SafeLlmProvider::new(raw, &cfg.safety))) } else { Ok(raw) }
}

/// Public so `auth.rs` can reuse the same value when minting cookies if needed for diagnostics.
pub fn test_session_secret() -> &'static str {
    TEST_SESSION_SECRET
}
