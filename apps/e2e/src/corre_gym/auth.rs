//! Programmatic user provisioning + session-cookie minting.
//!
//! The corre-gym Telegram-Login HMAC dance only matters at the `/auth/telegram` HTTP
//! boundary; once a session cookie exists, every `/api/*` request accepts it. So tests
//! skip the dance entirely: they `INSERT` a user row directly, then call the same
//! `create_session_cookie` helper the production handler uses, signed with the
//! `session_secret` we baked into `AppState` at server startup.

use std::sync::Arc;

use anyhow::Context as _;
use corre_gym::db::{User, new_user};
use corre_gym::web::AppState;
use corre_gym::web::auth::{SESSION_COOKIE_NAME, create_session_cookie};
use tokio::sync::Mutex;

use super::world::RegisteredUser;

/// Insert a user row and mint a session cookie for them.
///
/// `display_name` is what gets stored as `users.name` (the `first_name + last_name`
/// concatenation the production callback would synthesise). `username` is informational
/// in tests — Telegram usernames are stored only in the live session payload, not the
/// users table — so it's accepted but unused beyond logging.
pub async fn register_user(
    db: &Arc<Mutex<corre_gym::db::Database>>,
    app_state: &Arc<AppState>,
    display_name: &str,
    username: Option<&str>,
    telegram_id: i64,
) -> anyhow::Result<RegisteredUser> {
    let draft = new_user(display_name, Some(&telegram_id.to_string()), "UTC");
    let user: User = {
        let db_guard = db.lock().await;
        let id = db_guard.insert_user(&draft).context("inserting test user")?;
        db_guard.get_user(id).context("re-reading test user")?.context("user vanished after insert")?
    };
    let raw_set_cookie = create_session_cookie(app_state, &user, telegram_id);
    let cookie_pair = raw_set_cookie.split(';').next().context("Set-Cookie had no '=' segment")?.trim().to_string();
    debug_assert!(
        cookie_pair.starts_with(&format!("{SESSION_COOKIE_NAME}=")),
        "minted cookie does not start with expected session name"
    );
    tracing::debug!(
        user_id = user.id,
        telegram_id,
        username = username.unwrap_or("<none>"),
        "registered test user and minted session cookie"
    );
    Ok(RegisteredUser { user, telegram_id, session_cookie: cookie_pair })
}
