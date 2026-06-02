//! Thin reqwest wrappers used by step definitions.

use anyhow::{Context as _, anyhow};
use reqwest::StatusCode;

use super::world::{ChatReply, GymWorld, RestTimerView};

/// POST `/api/chat` as the given user alias. Returns the parsed `{ "reply": "…" }`.
///
/// On non-2xx the call returns an `Err`; callers that care about the specific status
/// code should use [`send_chat_expect`] instead.
pub async fn send_chat(world: &GymWorld, alias: &str, message: &str) -> anyhow::Result<ChatReply> {
    let user = world.users.get(alias).ok_or_else(|| anyhow!("unknown user alias: {alias}"))?;
    let url = format!("{}/api/chat", world.server.base_url);
    tracing::debug!(alias, message, "POST /api/chat");
    let res = world
        .http
        .post(&url)
        .header("Cookie", &user.session_cookie)
        .json(&serde_json::json!({ "message": message }))
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.with_context(|| format!("decoding /api/chat body (status {status})"))?;
    if !status.is_success() {
        anyhow::bail!("POST /api/chat returned {status}: {body}");
    }
    let text =
        body.get("reply").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("/api/chat response missing `reply` string: {body}"))?.to_string();
    let rest_timer = body.get("rest_timer").and_then(|v| {
        let duration = v.get("duration_secs")?.as_u64()? as u32;
        let exercise_name = v.get("exercise_name")?.as_str()?.to_string();
        let is_superset = v.get("is_superset")?.as_bool().unwrap_or(false);
        Some(RestTimerView { duration_secs: duration, exercise_name, is_superset })
    });
    let cancel_rest_timer = body.get("cancel_rest_timer").and_then(|v| v.as_bool()).unwrap_or(false);
    tracing::debug!(reply_len = text.len(), rest_timer = ?rest_timer, cancel_rest_timer, "received /api/chat reply");
    Ok(ChatReply { text, rest_timer, cancel_rest_timer })
}

/// Like [`send_chat`] but returns the status code instead of bailing on non-2xx.
#[allow(dead_code)]
pub async fn send_chat_expect(world: &GymWorld, alias: &str, message: &str) -> anyhow::Result<(StatusCode, serde_json::Value)> {
    let user = world.users.get(alias).ok_or_else(|| anyhow!("unknown user alias: {alias}"))?;
    let url = format!("{}/api/chat", world.server.base_url);
    let res = world
        .http
        .post(&url)
        .header("Cookie", &user.session_cookie)
        .json(&serde_json::json!({ "message": message }))
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or_else(|_| serde_json::json!(null));
    Ok((status, body))
}
