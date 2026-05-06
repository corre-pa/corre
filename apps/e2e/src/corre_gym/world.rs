//! The cucumber `World` for the corre-gym e2e suite.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use cucumber::World;
use reqwest::StatusCode;
use tokio::sync::Mutex;

use corre_gym::db::{Database, User};
use corre_gym::web::AppState;

use super::fixtures;
use super::server::TestServer;

/// A user that has been provisioned in the DB and signed-in via a minted session cookie.
#[derive(Clone)]
pub struct RegisteredUser {
    pub user: User,
    pub telegram_id: i64,
    /// Raw `corre_gym_session=…` pair, ready for the `Cookie` request header.
    pub session_cookie: String,
}

impl fmt::Debug for RegisteredUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredUser")
            .field("user_id", &self.user.id)
            .field("name", &self.user.name)
            .field("telegram_id", &self.telegram_id)
            .field("session_cookie", &"<redacted>")
            .finish()
    }
}

/// Reply from `POST /api/chat`. Just the assistant's text — actions live in the DB.
#[derive(Debug, Clone)]
pub struct ChatReply {
    pub text: String,
}

/// Per-scenario state. A fresh `GymWorld` is created for every scenario, which means a
/// fresh in-memory DB and a fresh axum task on a fresh port.
#[derive(World)]
#[world(init = Self::new)]
pub struct GymWorld {
    pub server: TestServer,
    pub http: reqwest::Client,
    pub users: HashMap<String, RegisteredUser>,
    pub last_reply: Option<ChatReply>,
    pub last_status: Option<StatusCode>,
    /// Alias of the user who last spoke; used by Then steps that want to assert
    /// against "the user from the last When step".
    pub current_user: Option<String>,
}

impl fmt::Debug for GymWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GymWorld")
            .field("base_url", &self.server.base_url)
            .field("user_count", &self.users.len())
            .field("current_user", &self.current_user)
            .field("last_reply_len", &self.last_reply.as_ref().map(|r| r.text.len()))
            .field("last_status", &self.last_status)
            .finish()
    }
}

impl GymWorld {
    pub async fn new() -> anyhow::Result<Self> {
        let cfg = fixtures::load_test_config()?;
        let server = TestServer::spawn(&cfg).await?;
        // Default reqwest client has no cookie jar (the `cookies` feature is not enabled),
        // so we set the Cookie header explicitly per request — exactly what we want.
        let http = reqwest::Client::builder().build()?;
        Ok(Self { server, http, users: HashMap::new(), last_reply: None, last_status: None, current_user: None })
    }

    /// Convenience: shared DB handle for assertions.
    pub fn db(&self) -> &Arc<Mutex<Database>> {
        &self.server.db
    }

    /// Convenience: shared `AppState` for cookie minting.
    pub fn app_state(&self) -> &Arc<AppState> {
        &self.server.app_state
    }

    /// Get a registered user by alias, returning a clear error if missing.
    pub fn user(&self, alias: &str) -> anyhow::Result<&RegisteredUser> {
        self.users.get(alias).ok_or_else(|| anyhow::anyhow!("no user registered with alias `{alias}`"))
    }
}
