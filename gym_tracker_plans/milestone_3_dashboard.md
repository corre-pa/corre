# Milestone 3: Web Dashboard

## Goal

A standalone Axum web server showing personalised workout history, progress charts, goals,
and a web chat interface. Authenticated via Telegram Login Widget (no passwords).

## Prerequisites

- Milestone 0 + 1 complete (database with data, Telegram bot working)
- Telegram Login Widget configured via @BotFather (set domain)
- The gym-tracker binary already runs; this milestone adds the HTTP server alongside the
  Telegram polling loop

## Architecture

The `corre-gym` binary runs two concurrent tasks:
1. Telegram long-polling loop (from M1)
2. Axum HTTP server on `config.gym.bind` (new in M3)

Both share the same `Database` (SQLite in WAL mode handles concurrent readers).

```rust
// main.rs
tokio::select! {
    result = telegram_loop(handler.clone(), telegram.clone()) => { ... }
    result = web::serve(config.gym.bind, db.clone(), llm.clone(), config.gym.clone()) => { ... }
}
```

The database needs to be wrapped in `Arc<Mutex<Database>>` or (better) use a connection pool
since rusqlite `Connection` is not `Send`. Options:
- `Arc<Mutex<Database>>` -- simple, works for moderate load
- `r2d2` + `r2d2_sqlite` -- connection pool, better concurrency
- `tokio::task::spawn_blocking` -- offload SQLite calls to blocking threadpool

Recommended: `Arc<Mutex<Database>>` initially (simplest). The dashboard is low-traffic.

## File structure

```
crates/corre-gym/src/
    web/
      mod.rs              Re-exports, router construction
      server.rs           Axum app setup, middleware, state
      auth.rs             Telegram Login Widget verification + session cookies
      handlers.rs         HTML page handlers (dashboard, history, progress)
      api.rs              JSON API handlers
      charts.rs           Data aggregation for chart endpoints
    main.rs               Spawn both Telegram loop and HTTP server
  static/
    style.css             Dashboard styling
    dashboard.js          Client-side interactivity (fetch, chart init)
    chart.min.js          Chart.js v4 (vendored, ~200KB minified)
  templates/
    base.html             Askama base layout (nav, head, body shell)
    login.html            Telegram Login Widget page
    dashboard.html        Overview page
    history.html          Exercise log with filters
    progress.html         Charts page
    chat.html             Web chat interface
```

## Routes

### HTML pages

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/` | Yes | Dashboard overview |
| GET | `/login` | No | Login page with Telegram widget |
| GET | `/logout` | Yes | Clear session cookie, redirect to /login |
| GET | `/history` | Yes | Exercise log table with date/exercise filters |
| GET | `/progress` | Yes | Progress charts (weight, volume, frequency) |
| GET | `/chat` | Yes | Web-based chat with the assistant |
| GET | `/static/{*path}` | No | Static assets (CSS, JS) |

### JSON API

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/auth/telegram` | No | Verify Telegram Login Widget hash, set cookie |
| GET | `/api/logs` | Yes | Exercise logs (query: from, to, exercise_id) |
| GET | `/api/sessions` | Yes | Workout sessions (query: from, to) |
| GET | `/api/progress/weight` | Yes | Weight progression (query: exercise_id, weeks) |
| GET | `/api/progress/volume` | Yes | Volume per muscle group per week (query: weeks) |
| GET | `/api/progress/frequency` | Yes | Sessions per week (query: weeks) |
| GET | `/api/progress/records` | Yes | Personal records per exercise |
| GET | `/api/targets` | Yes | Active targets |
| GET | `/api/health` | Yes | Active and recent health entries |
| GET | `/api/schedule` | Yes | User's schedules with exercises |
| POST | `/api/chat` | Yes | Send message, get assistant response (JSON) |
| GET | `/api/user` | Yes | Current user profile |
| GET | `/api/group/{id}/members` | Yes | Group members (if user has read access) |

## Authentication (web/auth.rs)

### Telegram Login Widget

The Telegram Login Widget is a JavaScript snippet that lets users authenticate on your
website using their Telegram account. The flow:

1. User visits `/login`
2. Page includes the Telegram Login Widget script:
   ```html
   <script async src="https://telegram.org/js/telegram-widget.js?22"
     data-telegram-login="YourBotUsername"
     data-size="large"
     data-auth-url="/auth/telegram"
     data-request-access="write">
   </script>
   ```
3. User clicks the widget, authenticates with Telegram
4. Telegram redirects to `/auth/telegram` with query params:
   `id`, `first_name`, `last_name`, `username`, `photo_url`, `auth_date`, `hash`
5. Server verifies the hash

### Hash verification

```rust
use sha2::{Sha256, Digest};
use hmac::{Hmac, Mac};

pub fn verify_telegram_login(
    params: &TelegramLoginParams,
    bot_token: &str,
) -> bool {
    // 1. Build the data-check-string (sorted key=value pairs, excluding hash)
    let mut fields = vec![];
    fields.push(format!("auth_date={}", params.auth_date));
    fields.push(format!("first_name={}", params.first_name));
    if let Some(ref id) = params.id { fields.push(format!("id={id}")); }
    if let Some(ref last_name) = params.last_name { fields.push(format!("last_name={last_name}")); }
    if let Some(ref photo_url) = params.photo_url { fields.push(format!("photo_url={photo_url}")); }
    if let Some(ref username) = params.username { fields.push(format!("username={username}")); }
    fields.sort();
    let data_check_string = fields.join("\n");

    // 2. Compute secret_key = SHA256(bot_token)
    let secret_key = Sha256::digest(bot_token.as_bytes());

    // 3. Compute HMAC-SHA256(data_check_string, secret_key)
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret_key).unwrap();
    mac.update(data_check_string.as_bytes());
    let result = hex::encode(mac.finalize().into_bytes());

    // 4. Compare with provided hash
    result == params.hash
}
```

### Session cookies

After successful verification:

```rust
// Create signed session token
let session = SessionToken {
    user_id: user.id.clone(),
    telegram_id: params.id.clone(),
    issued_at: chrono::Utc::now().timestamp(),
};

// Sign with HMAC-SHA256 using bot_token as key
let token_json = serde_json::to_string(&session)?;
let signature = hmac_sha256(bot_token, token_json.as_bytes());
let cookie_value = format!("{}.{}", base64_encode(&token_json), hex_encode(&signature));

// Set cookie
headers.insert(SET_COOKIE, format!(
    "gym_session={}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=2592000",
    cookie_value
));
```

### Auth middleware

An Axum middleware (or extractor) that:
1. Reads the `gym_session` cookie
2. Verifies the HMAC signature
3. Checks `auth_date` is not too old (configurable, e.g. 30 days)
4. Looks up the user by ID
5. Returns the User or redirects to `/login`

```rust
pub struct AuthUser(pub User);

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = Redirect;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract and verify cookie...
    }
}
```

## Axum app setup (web/server.rs)

```rust
pub async fn serve(bind: &str, db: Arc<Mutex<Database>>, llm: Arc<LlmProvider>, config: GymConfig) -> Result<()> {
    let state = AppState { db, llm, config };

    let app = Router::new()
        // HTML pages
        .route("/", get(handlers::dashboard))
        .route("/login", get(handlers::login))
        .route("/logout", get(handlers::logout))
        .route("/history", get(handlers::history))
        .route("/progress", get(handlers::progress))
        .route("/chat", get(handlers::chat_page))
        // Auth
        .route("/auth/telegram", get(auth::telegram_callback))
        // API
        .route("/api/logs", get(api::get_logs))
        .route("/api/sessions", get(api::get_sessions))
        .route("/api/progress/weight", get(api::weight_progress))
        .route("/api/progress/volume", get(api::volume_progress))
        .route("/api/progress/frequency", get(api::frequency_progress))
        .route("/api/progress/records", get(api::personal_records))
        .route("/api/targets", get(api::get_targets))
        .route("/api/health", get(api::get_health))
        .route("/api/schedule", get(api::get_schedule))
        .route("/api/chat", post(api::chat))
        .route("/api/user", get(api::get_user))
        .route("/api/group/{id}/members", get(api::get_group_members))
        // Static assets
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("Dashboard listening on {bind}");
    axum::serve(listener, app).await?;
    Ok(())
}
```

## Dashboard page (templates/dashboard.html)

Overview showing:
- **Today's workout**: scheduled exercises, completed so far
- **This week**: number of sessions, total volume
- **Active targets**: progress bars
- **Active health issues**: warnings/alerts
- **Recent sessions**: last 5 sessions with summary
- **Quick stats**: streak (consecutive days with sessions), most-trained muscle group

## History page (templates/history.html)

- Date range filter (from/to)
- Exercise filter (dropdown)
- Muscle group filter
- Table showing: Date, Exercise, Sets x Reps @ Weight, Difficulty, Notes
- Pagination
- Export to CSV button (client-side)

## Progress page (templates/progress.html)

### Charts (rendered with Chart.js)

**1. Weight Progression** (line chart)
- Select exercise from dropdown
- X axis: date, Y axis: max weight lifted that day
- Overlay target line if a target exists
- Time range: 4w, 12w, 6m, 1y

**2. Volume per Muscle Group** (stacked bar chart)
- X axis: week, Y axis: total volume (sets * reps * weight)
- Each bar segment = one muscle group (color-coded)
- Time range: 4w, 12w, 6m

**3. Workout Frequency** (bar chart)
- X axis: week, Y axis: number of sessions
- Overlay target frequency line if configured
- Time range: 4w, 12w, 6m, 1y

**4. Personal Records** (table)
- Exercise | PR Weight | PR Date | Recent Max | Trend (up/down/flat)

### Chart data aggregation (web/charts.rs)

```rust
impl Database {
    /// Weight progression: max weight per day for a given exercise.
    pub fn weight_progression(
        &self, user_id: &str, exercise_id: &str, weeks: i32,
    ) -> Result<Vec<(NaiveDate, f64)>>;

    /// Volume per muscle group per week.
    pub fn volume_by_muscle_group(
        &self, user_id: &str, weeks: i32,
    ) -> Result<Vec<(String, String, f64)>>;  // (week, muscle_group, volume)

    /// Sessions per week.
    pub fn sessions_per_week(
        &self, user_id: &str, weeks: i32,
    ) -> Result<Vec<(String, i32)>>;  // (week_label, count)

    /// Personal records per exercise.
    pub fn personal_records(
        &self, user_id: &str,
    ) -> Result<Vec<PersonalRecord>>;
}
```

SQL example for weight progression:
```sql
SELECT DATE(logged_at) as day, MAX(weight_kg) as max_weight
FROM exercise_logs
WHERE user_id = ? AND exercise_id = ?
  AND logged_at >= datetime('now', ? || ' days')
  AND weight_kg IS NOT NULL
GROUP BY DATE(logged_at)
ORDER BY day
```

## Chat page (templates/chat.html)

- Chat bubble UI (similar to messaging apps)
- Text input + send button
- Messages displayed as user/assistant pairs
- Loads recent conversation history on page load
- POST to `/api/chat` with message text, receive JSON response
- Auto-scroll to latest message

### API chat endpoint

```rust
#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub reply: String,
    pub actions_taken: Vec<String>,  // human-readable list of actions executed
}

async fn chat(
    AuthUser(user): AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Json<ChatResponse> {
    // Reuse the same AssistantHandler from M1
    let reply = state.handler.handle_text_message_for_user(&user, &req.message).await;
    // ...
}
```

## Access control in web

All API endpoints enforce access control:

```rust
async fn get_logs(
    AuthUser(user): AuthUser,
    Query(params): Query<LogQuery>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ExerciseLog>>, StatusCode> {
    let db = state.db.lock().unwrap();

    // If viewing another user's data (via group access)
    let target_user = params.user_id.as_deref().unwrap_or(&user.id);
    if target_user != user.id && !db.can_read(&user.id, target_user)? {
        return Err(StatusCode::FORBIDDEN);
    }

    let logs = db.get_logs_for_user(target_user, params.from.as_deref(), params.to.as_deref())?;
    Ok(Json(logs))
}
```

Group member viewing:
- If user belongs to a group with `read` access, the dashboard shows a user-switcher dropdown
- Selecting another user shows their data (read-only unless user has `write` access)
- Write access enables editing/deleting another user's logs (for personal trainers)

## Static assets

### style.css

Clean, responsive design. Dark mode support via `prefers-color-scheme`. Mobile-friendly
(the dashboard is primarily accessed from phones after a workout).

Key design elements:
- Card-based layout for dashboard widgets
- Responsive table for history
- Full-width charts on progress page
- Chat bubbles with user/assistant distinction

### dashboard.js

- Fetch chart data from API and initialize Chart.js instances
- Handle date range filter changes (re-fetch and re-render)
- Chat page: message sending, history loading, auto-scroll
- Exercise filter dropdown population

### chart.min.js

Vendor Chart.js v4 (MIT license). Include as a static file rather than CDN
to avoid external network requests (privacy-centric).

## Docker compose addition

```yaml
# In docker-compose.yml
corre-gym:
  image: ghcr.io/corre-pa/corre-gym:latest
  build:
    context: .
    dockerfile: crates/corre-gym/Dockerfile
  command: ["/app/corre-gym", "-c", "/data/corre.toml"]
  ports:
    - "5520:5520"
  volumes:
    - ${CORRE_DATA_DIR:-/var/corre}:/data
  env_file: [.env]
  environment:
    CORRE_DATA_DIR: /data
  restart: unless-stopped
  networks: [corre-internal]
```

## Dependencies to add

```toml
# Additional deps for M3
axum = { workspace = true }
tower-http = { workspace = true }    # ServeDir for static files
askama = { workspace = true }
rust-embed = { workspace = true }    # embed static assets in binary
hmac = "0.12"                        # HMAC-SHA256 for Telegram Login + cookies
hex = "0.4"                          # hex encoding for HMAC comparison
```

Add `hmac` and `hex` to workspace deps if not already present.

## Tests

### Auth tests
- `verify_telegram_login_valid` -- known-good test data verifies correctly
- `verify_telegram_login_tampered` -- modified data fails verification
- `session_cookie_round_trip` -- create, sign, verify, extract user_id
- `expired_session_rejected` -- old auth_date is rejected

### API tests
- `get_logs_returns_own_data` -- user sees their logs
- `get_logs_denies_other_user` -- user cannot see another user's logs
- `get_logs_allows_group_read` -- group member with read access can view
- `chart_data_aggregation` -- verify weight progression, volume, frequency SQL

### Page handler tests
- `dashboard_redirects_unauthenticated` -- no cookie -> 302 to /login
- `dashboard_renders_with_valid_session` -- valid cookie -> 200

## Verification

```sh
# Tests
cargo test -p corre-gym -- web

# Manual
cargo run -p corre-gym -- -c ~/.local/share/corre/corre.toml

# 1. Open http://localhost:5520/login
# 2. Click Telegram Login Widget, authenticate
# 3. Verify redirect to dashboard with session cookie
# 4. Navigate to /history -- see logged exercises
# 5. Navigate to /progress -- see charts
# 6. Navigate to /chat -- type a message, verify assistant responds
```
