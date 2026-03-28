# Milestone 3: Web Dashboard

Status: Implemented

## Goal

A standalone Axum web server running alongside the Telegram polling loop in the same
`corre-gym` binary. Shows personalised workout history, progress charts, goals, health
status, and a web chat interface. Authenticated via Telegram Login Widget (no passwords).

## Prerequisites

- Milestone 0 (database) + Milestone 1 (Telegram text chat) complete
- Telegram Login Widget configured via @BotFather (Settings > Domain > set the domain
  where the dashboard will be served)
- The gym-tracker binary already runs; this milestone adds the HTTP server as a second
  concurrent task

## Architecture

The `corre-gym` binary runs two concurrent tasks via `tokio::select!`:

1. **Telegram long-polling loop** (from M1) -- unchanged
2. **Axum HTTP server** on `config.gym.bind` (default `127.0.0.1:5520`) -- new in M3

Both share the same `Arc<RwLock<Database>>` (SQLite in WAL mode handles concurrent readers;
`RwLock` allows parallel read queries from the web and Telegram while serializing writes)
and the same `Arc<AssistantHandler>`. The handler is currently not `Arc`-wrapped -- this
milestone changes it.

```rust
// main.rs (simplified)
let handler = Arc::new(AssistantHandler::new(db.clone(), llm, config.clone()).await?);

tokio::select! {
    result = run_polling_loop(&telegram, &handler, &allowed_ids, voice_pipeline.as_ref()) => { ... }
    result = web::serve(&config.bind, db.clone(), handler.clone(), config.clone(), bot_username) => { ... }
    _ = tokio::signal::ctrl_c() => {
        tracing::info!("Ctrl+C received, shutting down");
        Ok(())
    }
}
```

The Ctrl+C handler lives in the outer `tokio::select!` so both tasks shut down cleanly on
signal. The inner polling loop's Ctrl+C handler is removed to avoid ambiguity about which
branch receives the signal.

If either the polling loop or the web server errors out, `tokio::select!` cancels the other
branch. This is intentional -- treat either failure as fatal.

The `run_polling_loop` signature changes to take `&Arc<AssistantHandler>` instead of
`&AssistantHandler`. Since `Arc<T>: Deref<Target = T>`, all `handler.method()` calls
continue to work unchanged.

## File structure

```
apps/corre-gym/
  askama.toml                 Template directory config (dirs = ["templates"])
  src/
    web/
      mod.rs                  Router, AppState, RustEmbed, serve()
      auth.rs                 Telegram Login verification, session cookies, AuthUser extractor
      handlers.rs             HTML page handlers (dashboard, history, progress, chat, login)
      api.rs                  JSON API handlers (logs, charts, chat, user profile)
    db/
      dashboard.rs            New aggregate queries (volume, records, streak, week summary)
    assistant/
      handler.rs              Refactored: extract handle_message_for_user()
    main.rs                   Arc<AssistantHandler>, tokio::select! for both loops
    lib.rs                    Add `pub mod web;`
  templates/
    base.html                 Shared layout (nav, head, CSS/JS includes)
    login.html                Telegram Login Widget (standalone, no base)
    dashboard.html            Overview page (extends base)
    history.html              Exercise log table with filters (extends base)
    progress.html             Charts page with Chart.js (extends base)
    chat.html                 Web chat interface (extends base)
  static/
    style.css                 Dashboard CSS (dark mode, mobile-first, card layout)
    dashboard.js              Chart init, API fetch, chat message handling
    chart.min.js              Vendored Chart.js v4 (~200KB, MIT)
```

## Dependencies

### Workspace Cargo.toml additions

```toml
hmac = "0.12"
hex = "0.4"
dashmap = "6"
```

### App Cargo.toml additions

```toml
axum = { workspace = true }
tower-http = { workspace = true }
askama = { workspace = true }
rust-embed = { workspace = true }
sha2 = { workspace = true }
hmac = { workspace = true }
hex = { workspace = true }
base64 = { workspace = true }
dashmap = { workspace = true }
```

Note: `hex` is needed for decoding the incoming hash in `verify_telegram_login` (via
`hex::decode`). The `hmac` crate's `Mac::verify_slice()` handles constant-time comparison
internally, so no `subtle` or `constant_time` crate is needed. `dashmap` is used for the
per-user chat rate limiter.

## Authentication (`web/auth.rs`)

### Telegram Login Widget flow

1. User visits `/login` -- page renders the Telegram Login Widget JS snippet
2. User clicks widget, authenticates with Telegram
3. Telegram redirects to `/auth/telegram` with query params:
   `id`, `first_name`, `last_name`, `username`, `photo_url`, `auth_date`, `hash`
4. Server verifies the HMAC hash, finds or creates the user, sets a session cookie,
   redirects to `/`

### Hash verification

```rust
fn verify_telegram_login(params: &TelegramLoginParams, bot_token: &str) -> bool {
    // Reject stale auth data (>5 minutes old)
    let now = chrono::Utc::now().timestamp();
    if now - params.auth_date > 300 {
        return false;
    }

    // Build data-check-string: alphabetically sorted key=value pairs, excluding "hash"
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("auth_date={}", params.auth_date));
    parts.push(format!("first_name={}", params.first_name));
    parts.push(format!("id={}", params.id));
    if let Some(ref v) = params.last_name { parts.push(format!("last_name={v}")); }
    if let Some(ref v) = params.photo_url { parts.push(format!("photo_url={v}")); }
    if let Some(ref v) = params.username { parts.push(format!("username={v}")); }
    parts.sort();
    let data_check_string = parts.join("\n");

    // secret_key = SHA256(bot_token)
    let secret_key = Sha256::digest(bot_token.as_bytes());

    // Verify HMAC-SHA256(data_check_string, secret_key) against the provided hash.
    // Mac::verify_slice performs constant-time comparison internally, avoiding the need
    // for a separate constant_time_eq function or the `subtle` crate.
    let mut mac = HmacSha256::new_from_slice(&secret_key).expect("HMAC accepts any key size");
    mac.update(data_check_string.as_bytes());
    mac.verify_slice(&hex::decode(&params.hash).unwrap_or_default()).is_ok()
}
```

Security properties:
- `id` and `first_name` are NOT optional -- they are always present in Telegram's Login Widget response
- `auth_date` staleness check (5-minute window) prevents replay attacks
- `hmac::Mac::verify_slice()` performs constant-time comparison internally, preventing timing side-channels

### TelegramLoginParams

```rust
#[derive(Debug, Deserialize)]
pub struct TelegramLoginParams {
    pub id: i64,                      // always present
    pub first_name: String,           // always present per Telegram API
    pub last_name: Option<String>,
    pub username: Option<String>,
    pub photo_url: Option<String>,
    pub auth_date: i64,               // Unix timestamp
    pub hash: String,
}
```

### Session cookies

Signed JSON payload: `base64url(json_payload) + "." + hex(hmac_sha256(base64_part, sha256(bot_token)))`

```rust
#[derive(Serialize, Deserialize)]
struct SessionPayload {
    user_id: String,
    telegram_id: i64,
    name: String,
    created_at: i64,  // Unix timestamp
}
```

Cookie attributes: `HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000` (30 days).
The `Secure` flag is inferred from the bind address: if the host is `127.0.0.1` or
`localhost`, `Secure` is omitted (local dev over HTTP); otherwise it is set (assumes TLS
termination via reverse proxy). This avoids a manual config flag that is easy to forget.

Session cookies are signed with `SHA256(session_secret)` if `config.gym.session_secret` is
set, otherwise `SHA256(bot_token)`. The `session_secret` config field allows rotating the
signing key independently of the bot token, which is useful for revoking all sessions
without disrupting the Telegram bot.

### AuthUser extractor

An Axum `FromRequestParts` extractor that:
1. Reads the `corre_gym_session` cookie
2. Splits on `.`, verifies HMAC signature with constant-time comparison
3. Decodes base64 payload, deserializes `SessionPayload`
4. Checks `created_at` is within 30 days
5. Looks up user by ID in the database (handles deleted users)
6. Returns `AuthUser { user: User }` or rejects:
   - HTML paths (`/`, `/history`, etc.) -> redirect to `/login`
   - API paths (`/api/*`) -> 401 Unauthorized

```rust
impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = axum::response::Response;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
        // Extract cookie, verify, lookup user...
        let path = parts.uri.path();
        if path.starts_with("/api/") {
            Err(StatusCode::UNAUTHORIZED.into_response())
        } else {
            Err(Redirect::to("/login").into_response())
        }
    }
}
```

### Login callback handler (`/auth/telegram`)

On successful verification:
1. Look up user by `telegram_id` in the DB
2. If not found, auto-register (same logic as Telegram bot registration)
3. Create signed session cookie
4. Redirect to `/`

## AssistantHandler refactoring (`assistant/handler.rs`)

The current `handle_text_message` takes `&TgMessage` for user resolution and hardcodes
`"telegram"` as the conversation platform. For web chat, the user is already authenticated
and the platform is `"web"`.

### Extract `handle_message_for_user`

```rust
/// Process a message for a known, authenticated user on any platform.
pub async fn handle_message_for_user(
    &self,
    user: &User,
    text: &str,
    platform: &str,
) -> anyhow::Result<Reply> {
    // Steps 2-13 from the existing handle_text_message, but:
    // - No ensure_user (caller already has the User)
    // - Conversation history loaded for `platform` instead of hardcoded "telegram"
    // - Conversation stored with `platform` instead of hardcoded "telegram"
    // - /clear command clears messages for the given `platform`
}
```

### Refactor `handle_text_message` to delegate

```rust
pub async fn handle_text_message(&self, message: &TgMessage, text: &str) -> anyhow::Result<Reply> {
    let (user, is_new) = self.ensure_user(message).await?;
    if is_new {
        return Ok(Reply::new(self.welcome_message(&user)));
    }
    self.handle_message_for_user(&user, text, "telegram").await
}
```

### Platform-parametric conversation storage

The existing private methods `store_conversation` and `store_excluded_conversation` hardcode
`"telegram"`. Refactor these to accept a `platform` parameter:

```rust
async fn store_conversation_on_platform(
    &self, user_id: &str, platform: &str, user_text: &str, assistant_text: &str,
) -> anyhow::Result<()>

async fn store_excluded_conversation_on_platform(
    &self, user_id: &str, platform: &str, user_text: &str, assistant_text: &str,
) -> anyhow::Result<()>
```

### `handle_command` platform awareness

The `/clear` command currently calls `exclude_all_messages_for_platform(user_id, "telegram")`.
Add a `platform` parameter to `handle_command` so it clears the correct platform's context.

```rust
async fn handle_command(
    &self, user: &User, command: &str, platform: &str,
) -> anyhow::Result<Option<Reply>>
```

`handle_message_for_user` passes its `platform` argument through to `handle_command`.
The existing `handle_text_message` delegates to `handle_message_for_user` with
`platform = "telegram"`, so `/clear` in Telegram continues to clear Telegram context,
while `/clear` in web chat clears web context.

## Web module (`web/mod.rs`)

### AppState

```rust
pub struct AppState {
    pub db: Arc<RwLock<Database>>,
    pub handler: Arc<AssistantHandler>,
    pub config: GymConfig,
    pub bot_username: String,  // for Telegram Login Widget (without @ prefix)
    pub chat_rate_limiter: DashMap<String, Vec<Instant>>,  // per-user sliding window
}
```

The `bot_username` is obtained from `telegram.get_me()` during startup (already called in
`main.rs`). The Login Widget requires the username **without** the `@` prefix. The existing
code extracts `me.username.as_deref()` which does not include `@` -- verify this with a
debug assertion at startup.

### Static assets via RustEmbed

```rust
#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;
```

Follows the same pattern as corre-news. Assets embedded in the binary at compile time.
Served via a handler that maps file extensions to MIME types.

### Router

```rust
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // HTML pages
        .route("/", get(handlers::dashboard))
        .route("/login", get(handlers::login))
        .route("/logout", get(handlers::logout))
        .route("/history", get(handlers::history))
        .route("/progress", get(handlers::progress))
        .route("/chat", get(handlers::chat))
        // Auth callback
        .route("/auth/telegram", get(auth::telegram_login_callback))
        // JSON API
        .route("/api/logs", get(api::logs))
        .route("/api/progress/exercise", get(api::progress_exercise))
        .route("/api/progress/volume", get(api::progress_volume))
        .route("/api/progress/frequency", get(api::progress_frequency))
        .route("/api/progress/records", get(api::progress_records))
        .route("/api/goals", get(api::goals))
        .route("/api/health", get(api::health))
        .route("/api/schedule", get(api::schedule))
        .route("/api/chat", post(api::chat_send))
        .route("/api/chat/history", get(api::chat_history))
        .route("/api/user", get(api::user_profile))
        .route("/api/group/{id}/members", get(api::group_members))
        // Liveness probe (unauthenticated)
        .route("/api/ping", get(|| async { "ok" }))
        // Static assets
        .route("/static/{*path}", get(static_handler))
        .with_state(state)
}
```

## Routes

### HTML pages

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/` | Yes | Dashboard overview |
| GET | `/login` | No | Telegram Login Widget page |
| GET | `/logout` | Yes | Set cookie `Max-Age=0; Path=/` to delete it, redirect to /login |
| GET | `/history` | Yes | Exercise log table with date/exercise/muscle group filters |
| GET | `/progress` | Yes | Progress charts (weight, volume, frequency, PRs) |
| GET | `/chat` | Yes | Web-based chat with the assistant |
| GET | `/static/{*path}` | No | Embedded static assets (CSS, JS) |

### JSON API

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/auth/telegram` | No | Verify Telegram Login hash, set session cookie |
| GET | `/api/logs` | Yes | Exercise logs (query: from, to, exercise_id, user_id, limit, offset) |
| GET | `/api/progress/exercise` | Yes | Time series for exercise (uses existing `exercise_time_series`) |
| GET | `/api/progress/volume` | Yes | Volume per muscle group per week (new query) |
| GET | `/api/progress/frequency` | Yes | Sessions per week (uses existing `session_count_by_week`) |
| GET | `/api/progress/records` | Yes | Personal records per exercise (new query) |
| GET | `/api/goals` | Yes | Active goals with progress (uses existing `goal_progress_report`) |
| GET | `/api/health` | Yes | Active health entries (uses existing `list_active_health_entries`) |
| GET | `/api/schedule` | Yes | User's schedules (uses existing `list_schedules`) |
| POST | `/api/chat` | Yes | Send message, get assistant reply (JSON). Rate-limited: 10 req/min per user |
| GET | `/api/chat/history` | Yes | Recent web chat messages (uses `get_recent_messages_for_platform`) |
| GET | `/api/user` | Yes | Current user profile |
| GET | `/api/group/{id}/members` | Yes | Group members (requires membership in that specific group) |
| GET | `/api/ping` | No | Liveness probe for monitoring (returns `"ok"`) |

### Access control in API

All API endpoints accept an optional `user_id` query parameter. When provided and
different from the authenticated user:
- Read endpoints check `db.can_read(auth.user.id, target_id)`
- Return 403 if denied
- Write access (for personal trainers editing clients' data) checked via `db.can_write()`

Helper function to avoid repetition:

```rust
async fn resolve_target_user(
    auth: &AuthUser,
    state: &AppState,
    requested_user_id: Option<&str>,
) -> Result<String, axum::response::Response> {
    match requested_user_id {
        Some(target_id) if target_id != auth.user.id => {
            let db = state.db.lock().await;
            if !db.can_read(&auth.user.id, target_id).unwrap_or(false) {
                return Err(StatusCode::FORBIDDEN.into_response());
            }
            Ok(target_id.to_string())
        }
        _ => Ok(auth.user.id.clone()),
    }
}
```

### Pagination

A modular `Paginated<T>` wrapper that any endpoint can opt into by adding `Query<PaginationParams>`
to its extractor list:

```rust
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub limit: Option<i64>,   // default 100, max 500
    pub offset: Option<i64>,  // default 0
}

#[derive(Debug, Serialize)]
pub struct Paginated<T: Serialize> {
    pub data: Vec<T>,
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
}
```

Database query functions that support pagination accept `(limit, offset)` and return
`(Vec<T>, total_count)`. The `total_count` comes from a `COUNT(*)` over the same WHERE
clause (without LIMIT/OFFSET). The `/api/logs` endpoint uses this; other endpoints can
adopt it by wrapping their return type in `Paginated<T>`.

### Rate limiting on `/api/chat`

The `POST /api/chat` endpoint is rate-limited to 10 requests per minute per user. This
prevents runaway browser tabs or misuse from hammering the LLM endpoint.

Implementation: a `DashMap<String, Vec<Instant>>` in `AppState` keyed by user ID. The chat
handler checks the sliding window before processing. Returns `429 Too Many Requests` with
a `Retry-After` header when exceeded.

### Group membership check

`GET /api/group/{id}/members` verifies that the requesting user is a member of that specific
group (any membership level), not just that they share a group with any target user. This
requires a dedicated query `is_group_member(user_id, group_id)` rather than reusing `can_read`.

## New database queries (`db/dashboard.rs`)

### Existing queries reused

| Query | File | Purpose on dashboard |
|-------|------|---------------------|
| `exercise_time_series` | `progress.rs` | Weight Progression line chart |
| `goal_progress_report` | `progress.rs` | Goal progress bars on dashboard |
| `session_count_by_week` | `logs.rs` | Workout Frequency bar chart |
| `list_active_health_entries` | `health.rs` | Health warnings on dashboard |
| `list_session_summaries` | `logs.rs` | Recent sessions on dashboard |
| `list_schedules` | `schedules.rs` | Schedule display |
| `get_logs_for_user` | `logs.rs` | History page log table |
| `list_full_exercises` | `exercises.rs` | Exercise dropdowns in filters |

### New queries

#### `volume_by_muscle_group_weekly`

Total volume (sets x reps x weight_kg) per muscle group per ISO week.
For the stacked bar chart on the progress page.

```sql
SELECT strftime('%G-W%V', el.logged_at) AS week,
       mg.name AS muscle_group,
       SUM(el.sets * el.reps * el.weight_kg) AS total_volume
FROM exercise_logs el
JOIN exercises e ON el.exercise_id = e.id
JOIN muscle_groups mg ON e.muscle_group_id = mg.id
WHERE el.user_id = ?1
  AND el.logged_at >= datetime('now', ?2)
  AND el.sets IS NOT NULL AND el.reps IS NOT NULL AND el.weight_kg IS NOT NULL
GROUP BY week, mg.name
ORDER BY week, mg.name
```

Returns `Vec<MuscleGroupWeeklyVolume>` (new struct: week, muscle_group, total_volume).

#### `personal_records`

All-time best per exercise. Uses `ROW_NUMBER()` window function to get the correct
`logged_at` for the PR (a simple `GROUP BY` would give an arbitrary date).

```sql
SELECT e.id, e.name, mg.name, mt.name, pr.best_value, pr.logged_at
FROM (
    SELECT el.exercise_id,
           CASE mt.name
               WHEN 'weight_reps' THEN el.weight_kg
               WHEN 'time_based' THEN CAST(el.duration_secs AS REAL)
               WHEN 'distance_based' THEN el.distance_m
               ELSE CAST(el.level AS REAL)
           END AS best_value,
           el.logged_at,
           ROW_NUMBER() OVER (
               PARTITION BY el.exercise_id
               ORDER BY CASE mt.name
                   WHEN 'weight_reps' THEN el.weight_kg
                   WHEN 'time_based' THEN CAST(el.duration_secs AS REAL)
                   WHEN 'distance_based' THEN el.distance_m
                   ELSE CAST(el.level AS REAL)
               END DESC
           ) AS rn
    FROM exercise_logs el
    JOIN exercises e ON el.exercise_id = e.id
    JOIN measurement_types mt ON e.measurement_type_id = mt.id
    WHERE el.user_id = ?1
      AND CASE mt.name
              WHEN 'weight_reps' THEN el.weight_kg IS NOT NULL
              WHEN 'time_based' THEN el.duration_secs IS NOT NULL
              WHEN 'distance_based' THEN el.distance_m IS NOT NULL
              ELSE el.level IS NOT NULL
          END
) pr
JOIN exercises e ON pr.exercise_id = e.id
JOIN muscle_groups mg ON e.muscle_group_id = mg.id
JOIN measurement_types mt ON e.measurement_type_id = mt.id
WHERE pr.rn = 1
ORDER BY mg.name, e.name
```

The subquery now includes `JOIN measurement_types mt` (previously missing, which would have
caused a SQL error) and filters out rows where the relevant measurement column is NULL,
so the window function only processes meaningful values. The outer `WHERE pr.best_value IS NOT NULL`
is no longer needed since NULLs are excluded in the subquery.

Returns `Vec<PersonalRecord>` (new struct: exercise_id, exercise_name, muscle_group,
measurement_type, value, achieved_at).

#### `workout_streak`

Consecutive days with at least one completed session, counting backwards from today.
Allows "yesterday" as the start if today has no session yet.

```rust
pub fn workout_streak(&self, user_id: &str) -> anyhow::Result<i32> {
    let dates: Vec<String> = /* SELECT DISTINCT date(started_at) FROM sessions
        WHERE user_id = ?1 AND ended_at IS NOT NULL ORDER BY date(started_at) DESC
        LIMIT 400 */;  // No realistic streak exceeds a year; bounds the result set

    // Walk backwards from today, counting consecutive days
    let today = chrono::Utc::now().date_naive();
    let mut streak = 0;
    let mut expected = today;

    for date_str in &dates {
        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")?;
        if date == expected {
            streak += 1;
            expected -= chrono::Duration::days(1);
        } else if streak == 0 && date == today - chrono::Duration::days(1) {
            // Allow starting from yesterday
            streak += 1;
            expected = date - chrono::Duration::days(1);
        } else {
            break;
        }
    }
    Ok(streak)
}
```

#### `week_summary`

Quick stats for the current ISO week: session count and total volume.

```sql
SELECT COUNT(DISTINCT s.id),
       COALESCE(SUM(
           CASE WHEN el.sets IS NOT NULL AND el.reps IS NOT NULL AND el.weight_kg IS NOT NULL
                THEN el.sets * el.reps * el.weight_kg ELSE 0 END
       ), 0)
FROM sessions s
LEFT JOIN exercise_logs el ON el.session_id = s.id
WHERE s.user_id = ?1
  AND s.started_at >= date('now', 'weekday 1', '-7 days')
  AND s.started_at < date('now', 'weekday 1')
```

Returns `(i32, f64)` (session_count, total_volume).

## Web chat endpoint (`POST /api/chat`)

Reuses the `AssistantHandler` from M1 via the new `handle_message_for_user` method:

```rust
async fn chat_send(
    auth: AuthUser,
    State(state): State<Arc<AppState>>,
    Json(body): Json<ChatSendBody>,
) -> impl IntoResponse {
    match state.handler.handle_message_for_user(&auth.user, &body.message, "web").await {
        Ok(reply) => Json(serde_json::json!({ "reply": reply.text })).into_response(),
        Err(e) => {
            tracing::error!("Chat error: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR,
             Json(serde_json::json!({ "error": "Processing failed" }))
            ).into_response()
        }
    }
}
```

Web chat uses `"web"` as the platform, so conversation history is separate from Telegram.
The `get_recent_messages_for_platform(user_id, "web", limit)` query already supports this.
The `conversation_history` table's CHECK constraint already allows `"web"` as a platform value.

**Future enhancement:** SSE streaming for chat responses. The current request-response model
means users see no feedback until the full LLM response is generated. SSE streaming would
let the frontend render tokens incrementally. This is acceptable for MVP but should be
revisited once the dashboard is in use.

## Templates

Use Askama template inheritance. The gym dashboard has 5+ pages that share nav, head,
and CSS includes -- template inheritance avoids duplication.

### `askama.toml`

```toml
[general]
dirs = ["templates"]
```

Located at `apps/corre-gym/askama.toml`. Templates resolve relative to the crate root.

### `base.html` -- shared layout

Provides: HTML head (viewport, CSS link), sidebar navigation (Dashboard, History,
Progress, Chat, Logout), and content/scripts blocks.

The sidebar highlights the active page. Navigation is mobile-responsive (collapses to a
bottom tab bar on small screens).

### Individual page templates

Each extends `base.html`:
- **`login.html`** -- standalone (no nav), renders the Telegram Login Widget JS snippet
  with `{{ bot_username }}` for the `data-telegram-login` attribute
- **`dashboard.html`** -- overview cards: week stats (sessions, volume, streak), active
  goals with progress bars, recent sessions list, active health alerts
- **`history.html`** -- filter controls (date range, exercise dropdown, muscle group) +
  a table that loads data from `/api/logs` via JS. Client-side CSV export
- **`progress.html`** -- Chart.js charts loaded from API endpoints. Includes chart.min.js
  and dashboard.js. Chart controls: exercise dropdown, time range selector
- **`chat.html`** -- chat bubble UI. Loads history from `/api/chat/history` on page load.
  Sends messages via `POST /api/chat`. Auto-scrolls on new messages

### Chart.js charts on the progress page

**1. Weight Progression** (line chart)
- User selects an exercise from a dropdown
- Fetches from `/api/progress/exercise?exercise_id=X&from=Y&to=Z`
- X axis: date, Y axis: max value per day
- Overlays goal target line if a goal exists for that exercise

**2. Volume per Muscle Group** (stacked bar chart)
- Fetches from `/api/progress/volume?weeks=N`
- X axis: ISO week, Y axis: total volume
- Each bar segment = one muscle group (color-coded)

**3. Workout Frequency** (bar chart)
- Fetches from `/api/progress/frequency?weeks=N`
- X axis: ISO week, Y axis: session count

**4. Personal Records** (table, not a chart)
- Fetches from `/api/progress/records`
- Columns: Exercise, Muscle Group, PR Value, Date Achieved

## Static assets

### `style.css`

Clean, responsive design. Dark mode via `prefers-color-scheme: dark`. Mobile-first layout
(the dashboard is primarily accessed from phones after a workout).

Key design elements:
- Sidebar navigation (desktop) / bottom tab bar (mobile)
- Card-based layout for dashboard widgets
- Responsive table for history
- Full-width charts on progress page
- Chat bubbles with user/assistant distinction

### `dashboard.js`

Client-side logic:
- Fetch chart data from API endpoints and initialize Chart.js instances
- Handle filter changes (date range, exercise dropdown) -- re-fetch and re-render
- Chat page: message sending via fetch, history loading, auto-scroll
- Exercise filter dropdown population from `list_full_exercises` data embedded in template

### `chart.min.js`

Vendored Chart.js v4 (MIT license, ~200KB minified). Embedded in the binary via RustEmbed
to avoid external CDN requests (privacy-centric, consistent with Corre philosophy).

## `main.rs` changes

### Structural changes

1. `AssistantHandler` wrapped in `Arc<AssistantHandler>`
2. `Database` changed from `Arc<Mutex<Database>>` to `Arc<RwLock<Database>>` -- allows parallel
   reads from web and Telegram while serializing writes
3. `setup()` returns additional values: `GymConfig`, `Arc<RwLock<Database>>`, bot_username
4. Main function uses `tokio::select!` with three branches: polling loop, web server, and
   Ctrl+C handler. The inner polling loop's Ctrl+C handler is removed
5. Bot username obtained from `telegram.get_me()` (already called, just store the result).
   Debug-assert that the value does not start with `@`

### `run_polling_loop` signature change

```rust
// Before:
async fn run_polling_loop(telegram: &TelegramClient, handler: &AssistantHandler, ...) -> anyhow::Result<()>

// After:
async fn run_polling_loop(telegram: &TelegramClient, handler: &Arc<AssistantHandler>, ...) -> anyhow::Result<()>
```

All interior `handler.method()` calls are unchanged thanks to `Deref`.

## Implementation sequence

Each step produces a compilable codebase:

### Step 1: Dependencies
Add `hmac = "0.12"`, `hex = "0.4"`, and `dashmap = "6"` to workspace `Cargo.toml`.
Add `axum`, `tower-http`, `askama`, `rust-embed`, `sha2`, `hmac`, `hex`, `base64`, `dashmap`
to `apps/corre-gym/Cargo.toml`.
Verify: `cargo check -p corre-gym`

### Step 2: Database queries (`db/dashboard.rs`)
New file with: `volume_by_muscle_group_weekly`, `personal_records`, `workout_streak`,
`week_summary`. New model structs: `MuscleGroupWeeklyVolume`, `PersonalRecord`.
Update `db/mod.rs` to include the module and re-export types.
Verify: `cargo test -p corre-gym -- dashboard`

### Step 3: Handler refactoring
Extract `handle_message_for_user`. Make conversation storage platform-parametric.
Change `handle_command` signature to `handle_command(&self, user, cmd, platform)` and
thread the platform parameter from `handle_message_for_user` through to `cmd_clear`.
This is a pure refactoring -- existing Telegram tests must pass.
Verify: `cargo test -p corre-gym`

### Step 4: Auth module (`web/auth.rs`)
Implement `verify_telegram_login`, session cookie create/verify, `AuthUser` extractor,
login callback handler. Write unit tests for hash verification, cookie round-trips, expiry.
Verify: `cargo test -p corre-gym -- auth`

### Step 5: Web module skeleton
Create `web/mod.rs` (router, AppState, RustEmbed, serve), `web/handlers.rs` (page handlers
with minimal templates), `web/api.rs` (JSON endpoints with `Paginated<T>` wrapper, per-user
chat rate limiter, `/api/ping` liveness endpoint). Create `askama.toml` and minimal
template stubs. Create minimal `static/style.css`.
Verify: `cargo check -p corre-gym`

### Step 6: `main.rs` integration
Wrap `AssistantHandler` in `Arc`. Change `Database` from `Mutex` to `RwLock`. Add
`web::serve()` as concurrent task via outer `tokio::select!` with Ctrl+C handler.
Remove inner polling loop's Ctrl+C handler. Store bot_username (verify no `@` prefix).
Update `run_polling_loop` signature.
Verify: `cargo run -p corre-gym -- -c path/to/corre.toml` (both loops start, Ctrl+C shuts down both)

### Step 7: Templates and static assets
Build full HTML templates with Askama inheritance. Write `style.css` with dark mode and
responsive layout. Write `dashboard.js` with Chart.js integration and API calls.
Vendor `chart.min.js`.
Verify: manual browser testing

### Step 8: Integration testing
Web-specific tests: auth flow, API access control, handler responses.
Full manual flow: `/login` -> Telegram auth -> `/` -> `/history` -> `/progress` -> `/chat`

## Pitfalls and mitigations

| Risk | Mitigation |
|------|------------|
| `Arc<RwLock<Database>>` contention between Telegram + web | `RwLock` allows parallel reads; write locks are short-lived. Keep lock scopes tight |
| `Box<dyn LlmProvider>` not Clone | Wrapping handler in `Arc` solves this -- both tasks share one instance |
| Askama template dir resolution | `askama.toml` with `dirs = ["templates"]` in crate root |
| RustEmbed folder path | `#[folder = "static/"]` is relative to crate's `Cargo.toml` |
| Telegram Login Widget requires HTTPS in prod | Document reverse proxy (nginx/caddy) for TLS termination |
| Session cookie `Secure` flag prevents local dev | Inferred from bind address: omit for `127.0.0.1`/`localhost`, set otherwise |
| `/clear` hardcodes "telegram" platform | Refactored: `handle_command(user, cmd, platform)` passes platform through |
| Ctrl+C only shuts down one task | Moved to outer `tokio::select!` so both tasks receive the signal |
| Chat endpoint abuse / LLM cost | Per-user rate limit (10 req/min) via `DashMap` sliding window |
| Session compromise with no revocation path | Optional `session_secret` config field rotatable independently of bot token |
| `/api/logs` returning unbounded rows | Modular `Paginated<T>` wrapper with default limit=100, max=500 |

## Tests

### Auth tests (`web/auth.rs`)
- `verify_telegram_login_valid` -- known-good test vector verifies via `Mac::verify_slice`
- `verify_telegram_login_tampered` -- modified id/name fails
- `verify_telegram_login_expired` -- old auth_date rejected
- `verify_telegram_login_bad_hex_hash` -- malformed hex in hash field returns false (not panic)
- `session_cookie_round_trip` -- create -> verify -> extract user_id
- `session_cookie_wrong_key` -- different bot_token/session_secret fails verification
- `session_cookie_expired` -- old created_at rejected

### DB query tests (`db/dashboard.rs`)
- `volume_by_muscle_group_weekly` with mixed exercises returns correct aggregation
- `personal_records` returns one PR per exercise with correct date
- `workout_streak` with consecutive days returns correct count
- `workout_streak` with gap resets count
- `workout_streak_starting_yesterday` works when today has no session
- `week_summary` returns correct session count and volume

### Handler refactoring tests
- All existing `handle_text_message` tests pass unchanged
- `handle_message_for_user` with platform `"web"` stores messages with platform `"web"`
- Web and Telegram conversation histories are isolated

### API tests
- Unauthenticated request to `/` redirects to `/login`
- Unauthenticated request to `/api/logs` returns 401
- Authenticated request to `/api/logs` returns own data with pagination metadata
- `/api/logs` respects `limit` and `offset` query parameters
- `/api/logs` clamps `limit` to max 500
- Request with `user_id=other` without group access returns 403
- Request with `user_id=other` with group read access returns data
- `POST /api/chat` returns assistant reply JSON
- `POST /api/chat` returns 429 after exceeding rate limit
- `GET /api/chat/history` returns web-platform messages only
- `GET /api/group/{id}/members` returns 403 for non-members of that group
- `GET /api/ping` returns 200 without authentication

## Verification

```sh
# Unit + integration tests
cargo test -p corre-gym -- web auth dashboard

# Start the binary
cargo run -p corre-gym -- -c ~/.local/share/corre/corre.toml

# Manual verification:
# 1. Open http://localhost:5520/login
# 2. Click Telegram Login Widget, authenticate
# 3. Verify redirect to / with session cookie set
# 4. Dashboard shows week stats, goals, recent sessions
# 5. /history shows exercise log table, filters work
# 6. /progress shows Chart.js charts with data from API
# 7. /chat -- type a message, verify assistant responds
# 8. /logout clears cookie, redirects to /login
# 9. Direct access to /api/logs without cookie returns 401
```
