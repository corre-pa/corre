# Milestone 1: Telegram Text Chat

## Goal

Users text a Telegram bot. The LLM-powered assistant understands gym-related commands,
records exercises, manages sessions, and responds conversationally. Text only -- no voice yet.

## Prerequisites

- Milestone 0 complete (database + CRUD layer)
- A Telegram bot created via @BotFather (provides the bot token)
- LLM provider configured in `corre.toml [llm]` section

## Design decisions

### LLM-to-database interface

The LLM needs to trigger database mutations (log exercise, start session, etc.) based on
user messages. Four approaches were considered:

**Option A: Structured JSON actions (chosen)**

The LLM returns a JSON object with `message` (reply text) and `actions` (array of typed
operations). The handler deserializes actions into an `AssistantAction` enum and calls DB
functions directly.

| Pro | Con |
|-----|-----|
| Fast -- no IPC or process overhead | Adding new action types requires code changes |
| Type-safe -- serde deserializes into enum | Not reusable outside corre-gym |
| Domain-level actions, not raw SQL | |
| Proven pattern (daily-brief structured scoring) | |
| Easy to debug (log the JSON, inspect actions) | |

**Option B: MCP server wrapping the DB**

An MCP server exposes DB operations as tools. The LLM discovers and calls them through MCP.

| Pro | Con |
|-----|-----|
| Standard MCP pattern | corre-gym is standalone -- no MCP pool |
| Potentially reusable by other agents | Would need embedded MCP client or sidecar process |
| | JSON-RPC round-trip per DB operation |
| | Over-engineered for a single consumer with direct DB access |

**Option C: CLI tool**

A CLI binary the LLM invokes via shell. Self-documenting via `--help`.

| Pro | Con |
|-----|-----|
| Self-documenting via `--help` | ~50-100ms process spawn overhead per operation |
| Testable independently | Fragile stdout parsing |
| | Shell injection risk from unsanitized LLM output |
| | Only suits agentic LLMs with shell access, not chat-completion |

**Option D: LLM native tool/function calling**

Use the OpenAI function_call API to define callable tools.

| Pro | Con |
|-----|-----|
| Providers optimize for tool-call accuracy | Not all OpenAI-compatible providers support it |
| Clean separation of conversation vs actions | Requires extending corre-llm wire types significantly |
| | Provider compatibility matrix grows |

Option A was chosen for its simplicity and directness. The domain actions are stable and
well-defined. If provider-native function calling is needed later, it's a natural evolution
that doesn't require rearchitecting the handler.

### Telegram client vs mcp-telegram reuse

The existing `mcp-telegram` crate (84 lines) is an rmcp MCP tool wrapper that only implements
`send_message` and `draft_message`. It reads the bot token from the environment per-call, has
no response types, no update polling, and no `get_me`. The gym tracker needs a full interactive
bot client: `get_updates` (long-polling), `send_message` (with parse_mode, reply_to),
`send_chat_action`, `get_me`, plus voice/file methods for M2.

The overlap between the two use cases is just "POST JSON to api.telegram.org" -- the API
surface, lifecycle, and error handling are entirely different. Building the client in corre-gym
is the right approach. When M6 (Signal integration) introduces a messaging abstraction trait,
we can revisit whether to extract a shared Telegram client crate that both corre-gym and
mcp-telegram depend on.

## File structure

```
apps/corre-gym/src/
    main.rs                     Telegram polling loop + graceful shutdown
    config.rs                   GymConfig from [gym] in corre.toml
    telegram/
        mod.rs                  Re-exports
        client.rs               Telegram Bot API HTTP client
        types.rs                Telegram API request/response serde types
    assistant/
        mod.rs                  Re-exports
        handler.rs              Message orchestrator (context -> LLM -> actions -> reply)
        prompts.rs              System prompt template + context formatting
        actions.rs              Action enum + execution logic
        parser.rs               Extract JSON from LLM output, parse into AssistantResponse
        matching.rs             Multi-stage fuzzy exercise name matching
```

## corre-core change

Add `gym: toml::Value` to `CorreConfig` in `crates/corre-core/src/config.rs`, following the
exact pattern used by `news`:

```rust
#[serde(default = "default_empty_table")]
pub gym: toml::Value,
```

The `default_empty_table()` function already exists. Add env var override in `CorreConfig::load`:

```rust
if let Ok(bind) = std::env::var("CORRE_GYM_BIND") {
    if let Some(table) = config.gym.as_table_mut() {
        table.insert("bind".into(), toml::Value::String(bind));
    }
}
```

This is the only change outside the `corre-gym` crate.

## Config (config.rs)

Add a `[gym]` section to `corre.toml`:

```toml
[gym]
bind = "127.0.0.1:5520"              # web dashboard (M3, unused here)
telegram_bot_token = "${TELEGRAM_GYM_BOT_TOKEN}"
telegram_allowed_ids = []             # empty = allow all (dev mode); set to restrict access
default_timezone = "Europe/London"
conversation_history_limit = 20       # messages to include in LLM context
db_path = "gym-tracker.db"            # relative to data_dir
max_message_length = 2000             # truncate user messages beyond this before sending to LLM
session_timeout_hours = 4             # auto-close stale sessions older than this

[gym.llm]                             # optional per-service LLM overrides
# model = "gpt-4o"
# temperature = 0.3
```

Config struct:

```rust
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GymConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    pub telegram_bot_token: String,
    /// Telegram user IDs allowed to use the bot. Empty = allow all (dev mode).
    #[serde(default)]
    pub telegram_allowed_ids: Vec<i64>,
    #[serde(default = "default_timezone")]
    pub default_timezone: String,
    #[serde(default = "default_history_limit")]
    pub conversation_history_limit: usize,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_max_message_length")]
    pub max_message_length: usize,
    #[serde(default = "default_session_timeout_hours")]
    pub session_timeout_hours: u32,
    #[serde(default)]
    pub llm: Option<AppLlmConfig>,
}
```

Defaults: `bind` = `"127.0.0.1:5520"`, `default_timezone` = `"Europe/London"`,
`conversation_history_limit` = `20`, `db_path` = `"gym-tracker.db"`,
`max_message_length` = `2000`, `session_timeout_hours` = `4`.

Follow the `corre-news::NewsConfig` pattern:

```rust
impl GymConfig {
    pub fn from_toml_table(table: Option<&toml::Value>) -> anyhow::Result<Self> {
        table
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing [gym] section in corre.toml"))
            .and_then(|v| v.try_into().map_err(Into::into))
    }

    /// Resolve ${VAR} references in secret fields.
    pub fn resolve_secrets(&mut self) -> anyhow::Result<()> {
        self.telegram_bot_token = corre_core::secret::resolve_value(&self.telegram_bot_token)
            .context("resolving TELEGRAM_GYM_BOT_TOKEN")?;
        Ok(())
    }
}
```

Unlike `NewsConfig` which silently defaults on parse failure, `GymConfig` returns an error if
the `[gym]` section or `telegram_bot_token` is missing -- these are required for operation.

## Telegram Bot API client (telegram/client.rs)

Lightweight HTTP client using `reqwest`. No external Telegram crate.

```rust
pub struct TelegramClient {
    token: String,
    client: reqwest::Client,
    base_url: String,  // "https://api.telegram.org/bot{token}"
}

impl TelegramClient {
    pub fn new(token: &str) -> Self;

    /// Get bot info (startup verification).
    pub async fn get_me(&self) -> Result<BotUser>;

    /// Long-poll for updates. Blocks server-side for up to `timeout` seconds.
    pub async fn get_updates(&self, offset: i64, timeout: u32) -> Result<Vec<Update>>;

    /// Send a text message. Returns the sent Message.
    pub async fn send_message(
        &self, chat_id: i64, text: &str, parse_mode: Option<&str>, reply_to: Option<i64>,
    ) -> Result<Message>;

    /// Send a "typing..." indicator.
    pub async fn send_chat_action(&self, chat_id: i64, action: &str) -> Result<()>;
}
```

All methods POST JSON to `{base_url}/{method}` and parse a `TelegramResponse<T>` wrapper.

Implementation details:
- `new()` validates the token format (`{digits}:{alphanumeric}`) and returns `Result`, failing
  early with a clear error rather than a cryptic Telegram 401
- `reqwest::Client` with 10s connect timeout
- Regular calls use 60s total timeout
- `get_updates` uses `(timeout + 10)s` total timeout to account for network overhead beyond
  the Telegram server-side long-poll window
- Telegram API errors (`ok == false`) produce `anyhow::bail!("Telegram API {error_code}: {description}")`
- 429 rate limiting: detect and sleep for `retry_after` seconds before retrying

### Telegram types (telegram/types.rs)

Minimal serde structs for the Bot API responses we need:

```rust
#[derive(Debug, Deserialize)]
pub struct TelegramResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub error_code: Option<i32>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: Chat,
    pub date: i64,
    pub text: Option<String>,
    pub voice: Option<Voice>,        // for M2
}

#[derive(Debug, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
}

#[derive(Debug, Deserialize)]
pub struct Voice {
    pub file_id: String,
    pub duration: i32,
}

#[derive(Debug, Deserialize)]
pub struct BotUser {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: String,
}
```

## Main loop (main.rs)

Keep `main()` short (~20 lines) by extracting setup and polling into helpers:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (telegram, handler, allowed_ids) = setup().await?;
    run_polling_loop(&telegram, &handler, &allowed_ids).await
}
```

### setup()

```rust
async fn setup() -> anyhow::Result<(TelegramClient, AssistantHandler, Vec<i64>)> {
    // 1. Load .env (same pattern as corre-news)
    let default_data_dir = dirs::data_dir()
        .map(|d| d.join("corre"))
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = dotenvy::from_filename_override(default_data_dir.join(".env")).ok();

    // 2. Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
        )
        .with_writer(std::io::stderr)
        .init();

    // 3. Parse CLI args (-c/--config only)
    let cli = Cli::parse();

    // 4. Load config
    let config = CorreConfig::load(&cli.config).context("loading config")?;
    let data_dir = config.data_dir();
    let mut gym_config = GymConfig::from_toml_table(Some(&config.gym))?;
    gym_config.resolve_secrets()?;

    // 5. Build LLM provider (with optional [gym.llm] overrides)
    let effective_llm = match gym_config.llm.as_ref() {
        Some(overrides) => config.llm.with_overrides(overrides),
        None => config.llm.clone(),
    };
    let llm = OpenAiCompatProvider::from_config(&effective_llm)?;

    // 6. Open database (exercise seeding happens in the migration, not here)
    let db = Database::open(&data_dir.join(&gym_config.db_path))?;
    let db = Arc::new(tokio::sync::Mutex::new(db));

    // 7. Create Telegram client, verify connection
    let telegram = TelegramClient::new(&gym_config.telegram_bot_token)?;
    let me = telegram.get_me().await?;
    tracing::info!(
        "Bot @{} connected (id: {})",
        me.username.as_deref().unwrap_or("?"),
        me.id
    );

    let allowed_ids = gym_config.telegram_allowed_ids.clone();

    // 8. Create handler
    let handler = AssistantHandler::new(db, Box::new(llm), gym_config)?;

    Ok((telegram, handler, allowed_ids))
}
```

### run_polling_loop()

```rust
async fn run_polling_loop(
    telegram: &TelegramClient, handler: &AssistantHandler, allowed_ids: &[i64],
) -> anyhow::Result<()> {
    let mut offset = 0i64;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("Shutting down");
                break Ok(());
            }
            result = telegram.get_updates(offset, 30) => {
                match result {
                    Ok(updates) => {
                        for update in updates {
                            offset = update.update_id + 1;
                            if let Some(ref message) = update.message {
                                process_message(telegram, handler, message, allowed_ids).await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("get_updates failed: {e:#}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}
```

### process_message()

```rust
async fn process_message(
    telegram: &TelegramClient, handler: &AssistantHandler,
    message: &Message, allowed_ids: &[i64],
) {
    // Skip non-private chats, messages without sender, messages without text
    if message.chat.chat_type != "private" { return; }
    let Some(ref from) = message.from else { return; };
    let Some(ref text) = message.text else { return; };

    // Authorization: reject users not in the allow-list (if non-empty)
    if !allowed_ids.is_empty() && !allowed_ids.contains(&from.id) {
        tracing::debug!("Ignoring message from unauthorized user {}", from.id);
        return;
    }

    let _ = telegram.send_chat_action(message.chat.id, "typing").await;

    let reply = match handler.handle_text_message(message, text).await {
        Ok(text) => text,
        Err(e) => {
            tracing::error!("Handler error: {e:#}");
            "I had trouble processing that -- could you try again?".to_string()
        }
    };

    if let Err(e) = send_long_message(telegram, message.chat.id, &reply).await {
        tracing::error!("Failed to send reply: {e:#}");
    }
}
```

### send_long_message()

Splits messages exceeding Telegram's 4096 character limit. Splitting strategy:
1. Try splitting at the last newline before the 4096 limit
2. If no newline, split at the last space
3. If no space, hard-split at 4096

### Exercise seeding

Exercise seeding is a database migration step, not part of the application startup.
The `seed_exercises()` call was moved into `Database::open()` as part of the migration
sequence in M0. The main function does not call it.

## Assistant handler (assistant/handler.rs)

The core orchestration logic:

```rust
pub struct AssistantHandler {
    db: Arc<tokio::sync::Mutex<Database>>,
    llm: Box<dyn LlmProvider>,
    config: GymConfig,
    exercises: Vec<FullExercise>,  // cached at startup; M3/M5 may need cache invalidation
}
```

The handler takes `Box<dyn LlmProvider>` (not `OpenAiCompatProvider` directly) so that tests
can inject a mock. The `LlmProvider` trait is defined in `corre_core::app`.

`Database` is wrapped in `Arc<tokio::sync::Mutex<Database>>` because `rusqlite::Connection` is
`Send` but not `Sync`. In M1 the polling loop is sequential so the mutex is uncontended. For M3
(web dashboard), WAL mode handles concurrent readers and the mutex serializes writes.

```rust
impl AssistantHandler {
    pub fn new(
        db: Arc<tokio::sync::Mutex<Database>>,
        llm: Box<dyn LlmProvider>,
        config: GymConfig,
    ) -> anyhow::Result<Self>;

    /// Process an incoming text message and return a reply string.
    /// Returns `Result<String>` -- the caller (`process_message`) catches errors
    /// and produces a generic fallback message for the user.
    pub async fn handle_text_message(
        &self, message: &Message, text: &str,
    ) -> anyhow::Result<String> {
        // 1. Identify user (auto-register if new)
        let (user, is_new) = self.ensure_user(message).await?;
        if is_new { return Ok(self.welcome_message(&user)); }

        // 2. Check for slash commands
        if let Some(reply) = self.handle_command(text, &user).await? {
            return Ok(reply);
        }

        // 3. Auto-close stale sessions (older than session_timeout_hours with no activity)
        self.close_stale_session(&user).await?;

        // 4. Truncate message to max_message_length to bound LLM token usage
        let text = if text.len() > self.config.max_message_length {
            &text[..self.config.max_message_length]
        } else {
            text
        };

        // 5. Build system prompt with current context
        let system_prompt = self.build_context(&user).await?;

        // 6. Load conversation history
        let history = self.db.lock().await.get_recent_messages_for_platform(
            &user.id, "telegram", self.config.conversation_history_limit,
        )?;

        // 7. Call LLM
        let llm_response = self.call_llm(&system_prompt, &history, text).await?;

        // 8. Parse response into message + actions
        let parsed = parse_assistant_response(&llm_response);

        // 9. Execute actions, track failures
        let mut failures: Vec<String> = Vec::new();
        for action in &parsed.actions {
            if let Err(e) = self.execute_action(action, &user).await {
                tracing::warn!("Action execution failed: {e:#}");
                failures.push(format!("{e:#}"));
            }
        }

        // 10. Build final reply, appending failure notes if any
        let reply = if failures.is_empty() {
            parsed.message.clone()
        } else {
            format!(
                "{}\n\n(Note: some actions failed: {})",
                parsed.message,
                failures.join("; ")
            )
        };

        // 11. Store conversation turn (only the extracted message text, not raw JSON)
        self.store_conversation(&user.id, text, &parsed.message).await?;

        // 12. Prune old messages to prevent unbounded growth
        self.db.lock().await.prune_old_messages(
            &user.id, self.config.conversation_history_limit * 2,
        )?;

        Ok(reply)
    }
}
```

### Session staleness

`close_stale_session` checks for an active session whose last activity (most recent exercise
log timestamp, or session start time if no logs) is older than `session_timeout_hours`. If
found, it auto-closes the session. This prevents yesterday's forgotten session from
accumulating today's exercises.

### User auto-registration

On first contact, create a User record:
- `id`: UUID v4
- `name`: Telegram `first_name` (+ ` last_name` if present)
- `telegram_id`: Telegram user ID (as string)
- `timezone`: default from config

Send a welcome message explaining the bot's capabilities.

### Slash commands

| Command | Action |
|---------|--------|
| `/start` | Welcome message + auto-register. For existing users, show a brief "already registered" note with a summary of capabilities |
| `/help` | List available commands and example phrases |
| `/status` | Current session state, today's stats, active health issues |
| `/history` | Last 5 workout summaries |
| `/exercises` | List available exercises by muscle group |

Commands return static or computed strings without going through the LLM. This keeps them
fast and deterministic.

### Edge cases

- Group chats: ignore (check `chat.chat_type == "private"`)
- Unauthorized users: silently ignore if `telegram_allowed_ids` is non-empty and user not listed
- Messages without `from`: skip
- Messages without text: skip in M1 (voice handled in M2)
- Oversized messages: truncate to `max_message_length` before sending to LLM
- LLM timeout or error: `process_message` catches and returns fallback "I had trouble..."
- DB write error during action execution: log warning, append failure note to reply, continue
- Multiple actions in one response: execute sequentially, collect all failure notes
- Stale sessions: auto-close sessions older than `session_timeout_hours` before processing

## LLM integration (assistant/prompts.rs)

### Context building

```rust
pub struct PromptContext {
    pub user_name: String,
    pub timezone: String,
    pub current_time: String,
    pub active_session: Option<Session>,
    pub session_logs: Vec<(ExerciseLog, String)>,  // (log, exercise_name)
    pub health_entries: Vec<HealthEntry>,
    pub recent_summaries: Vec<SessionSummary>,
    pub recent_logs: Vec<ExerciseLog>,
    pub exercises: Vec<FullExercise>,
    pub active_goals: Vec<GoalProgress>,
    pub schedules: Vec<Schedule>,
}

pub fn build_system_prompt(ctx: &PromptContext) -> String;
pub fn format_exercise_list(exercises: &[FullExercise]) -> String;
pub fn format_health_entries(entries: &[HealthEntry]) -> String;
pub fn format_recent_history(
    summaries: &[SessionSummary], logs: &[ExerciseLog], exercises: &[FullExercise],
) -> String;
pub fn format_active_goals(goals: &[GoalProgress]) -> String;
```

### System prompt template

```
You are a personal gym trainer assistant. You help users track workouts, log exercises,
manage health issues, and provide coaching.

RESPONSE FORMAT: You MUST respond with ONLY a JSON object. No text before or after.
{
  "message": "Your conversational response to the user",
  "actions": []
}

ACTION TYPES:
- {"type": "log_exercise", "exercise": "<EXACT NAME>", "sets": N, "reps": N,
   "weight_kg": N.N, "difficulty": "easy|medium|hard|failure"}
- {"type": "log_exercise_timed", "exercise": "<EXACT NAME>", "duration_secs": N,
   "difficulty": "easy|medium|hard|failure"}
- {"type": "log_exercise_distance", "exercise": "<EXACT NAME>", "distance_m": N.N,
   "duration_secs": N, "difficulty": "easy|medium|hard|failure"}
- {"type": "start_session", "notes": "<optional>"}
- {"type": "end_session"}
- {"type": "log_health", "entry_type": "injury|illness|wellbeing",
   "body_part": "<optional>", "severity": "mild|moderate|severe", "description": "..."}
- {"type": "resolve_health", "description": "match by description substring"}
- {"type": "set_goal", "exercise": "<EXACT NAME>", "target_value": N.N,
   "end_date": "<optional YYYY-MM-DD>"}

EXERCISE NAME RULE: You MUST use exercise names EXACTLY as they appear in double quotes
in the Available Exercises list below. Do not abbreviate, paraphrase, or invent names.
If the user mentions an exercise not in the list, use the closest match and note the
substitution in your message.

GUIDELINES:
- When the user reports an exercise, always include a log_exercise action
- Auto-start a session (start_session action) before logging if no session is active
- If the user mentions pain, injury, or illness, log it with log_health
- Keep responses concise -- this is a chat interface
- Be encouraging but not patronizing
- If details are ambiguous, ask for clarification rather than guessing
- All action fields use metric units (weight_kg, distance_m). If the user specifies
  imperial, convert to metric in the action and mention the conversion in your message

CURRENT STATE:
User: {user_name}
Time: {current_time} ({timezone})
Active session: {session_status}

{health_entries_section}

{recent_history_section}

{active_goals_section}

AVAILABLE EXERCISES:
{exercise_list}
```

### Exercise list format

Exercises are grouped by muscle group with aliases and measurement type shown so the LLM
can recognize informal names and knows which fields to populate:

```
## Chest
- "Barbell Bench Press" (aliases: flat bench, bench, bench press) [weight_reps]
- "Incline Dumbbell Press" (aliases: incline press, incline db press) [weight_reps]

## Back
- "Conventional Deadlift" (aliases: deadlift, dl) [weight_reps]
...
```

### Conversation history storage contract

Only the extracted `message` text from the LLM response is stored in `conversation_history`,
not the raw JSON. This keeps the history clean and saves tokens when re-injecting into context.

When building the LLM messages array, conversation history is included as plain-text
user/assistant pairs. The system prompt contains the JSON format instruction, and the current
user message is the only new input. The LLM sees its own previous replies as plain text but
is instructed to respond in JSON -- this is handled by placing the format instruction clearly
in the system prompt (which is always present) rather than relying on the LLM inferring
format from its own history.

### LLM call

Use `corre_llm::OpenAiCompatProvider::complete()` (via `Box<dyn LlmProvider>`) with:
- System message: filled template above
- Conversation history: last N messages as user/assistant pairs (plain text)
- Current user message
- Temperature: 0.3 (low for structured output reliability)
- `max_completion_tokens`: 1024 (the response is structured JSON with a short message and
  small actions array; capping this prevents runaway responses and controls cost)

### Token budget

The exercise list (48 exercises) is ~2000 tokens. The full system prompt with context is
~2500-3500 tokens. With 20 history messages (~1000 tokens), total input is ~4000 tokens.
This fits comfortably in any modern model's context window.

## Action types and execution (assistant/actions.rs)

```rust
#[derive(Debug, Deserialize)]
pub struct AssistantResponse {
    pub message: String,
    #[serde(default)]
    pub actions: Vec<AssistantAction>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantAction {
    LogExercise {
        exercise: String,
        sets: Option<i32>,
        reps: Option<i32>,
        weight_kg: Option<f64>,
        #[serde(default)]
        difficulty: Option<Difficulty>,
    },
    LogExerciseTimed {
        exercise: String,
        duration_secs: i32,
        #[serde(default)]
        difficulty: Option<Difficulty>,
    },
    LogExerciseDistance {
        exercise: String,
        distance_m: Option<f64>,
        duration_secs: Option<i32>,
        #[serde(default)]
        difficulty: Option<Difficulty>,
    },
    StartSession {
        notes: Option<String>,
    },
    EndSession,
    LogHealth {
        entry_type: HealthEntryType,
        body_part: Option<String>,
        severity: Option<String>,
        description: String,
    },
    ResolveHealth {
        description: String,
    },
    SetGoal {
        exercise: String,
        target_value: f64,
        end_date: Option<String>,
    },
    #[serde(other)]
    Unknown,
}
```

Uses domain enums (`Difficulty`, `HealthEntryType`) from the M0 models directly. These
already implement serde deserialization with `from_str_loose()`, so malformed values from the
LLM are caught at parse time. If the enum value is unrecognized, the whole action falls to
`Unknown` via `#[serde(other)]`.

The `#[serde(default)]` on `actions` means if the LLM omits the field entirely, we get an
empty vec rather than a parse error.

### Imperial unit conversion

The system prompt instructs the LLM to always convert imperial units to metric before
populating action fields. The DB columns are `weight_kg` and `distance_m`. When a user says
"I benched 185 lbs", the LLM converts to ~83.9 kg in the `weight_kg` field and mentions the
conversion in its `message` reply. No conversion logic is needed in the handler.

### Action execution

```rust
fn execute_action(&self, action: &AssistantAction, user: &User) -> Result<()> {
    match action {
        AssistantAction::LogExercise { exercise, sets, reps, weight_kg, difficulty } => {
            // 1. Look up exercise via matching pipeline
            let ex = find_exercise(&self.exercises, exercise)
                .ok_or_else(|| anyhow!("Unknown exercise: {exercise}"))?;
            // 2. Get or auto-start session
            let session = self.ensure_session(user)?;
            // 3. Build ExerciseLog, insert
            let log = new_exercise_log(&user.id, &ex.exercise.id, &session.id);
            // Set sets, reps, weight_kg, difficulty
            self.db.insert_log(&log)?;
        }
        AssistantAction::StartSession { notes } => {
            // Only start if no active session
            if self.db.get_active_session(&user.id)?.is_none() {
                self.db.start_session(&user.id, notes.as_deref())?;
            }
        }
        AssistantAction::EndSession => {
            if let Some(session) = self.db.get_active_session(&user.id)? {
                self.db.end_session(&session.id)?;
            }
        }
        AssistantAction::LogHealth { entry_type, body_part, severity, description } => {
            let entry = new_health_entry(&user.id, entry_type, description);
            // Set body_part, severity
            self.db.insert_health_entry(&entry)?;
        }
        AssistantAction::ResolveHealth { description } => {
            // Find active entry by description substring, resolve it
            let entries = self.db.list_active_health_entries(&user.id)?;
            if let Some(entry) = entries.iter().find(|e|
                e.description.to_lowercase().contains(&description.to_lowercase())
            ) {
                self.db.resolve_health_entry(&entry.id)?;
            }
        }
        AssistantAction::SetGoal { exercise, target_value, end_date } => {
            let ex = find_exercise(&self.exercises, exercise)
                .ok_or_else(|| anyhow!("Unknown exercise: {exercise}"))?;
            let goal = new_exercise_goal(&user.id, &ex.exercise.id, *target_value);
            // Set end_date if provided
            self.db.insert_goal(&goal)?;
        }
        AssistantAction::Unknown => {
            tracing::debug!("Ignoring unknown action type from LLM");
        }
        // Timed and distance variants follow the same pattern as LogExercise
    }
}
```

### Fuzzy exercise matching (assistant/matching.rs)

The system prompt forces the LLM to use exact exercise names. The fuzzy matching pipeline is
a safety net for when the LLM deviates:

```rust
pub fn find_exercise<'a>(exercises: &'a [FullExercise], name: &str) -> Option<&'a FullExercise>
```

Multi-stage pipeline (stops at first match):

1. **Exact match (case-insensitive)**: `name.eq_ignore_ascii_case(&exercise.name)`
2. **Alias match (case-insensitive)**: Split each exercise's `aliases` by comma, trim, compare
3. **Contains match**: `exercise.name.to_lowercase().contains(name_lower)` -- only if
   exactly one exercise matches (skip if ambiguous)
4. **Levenshtein distance**: Return best match if distance <= 3 AND distance < half name
   length. This prevents matching "run" to "Romanian Deadlift"
5. **No match**: Return `None`. The handler logs a warning

Implement Levenshtein as an inline two-row DP function (~15 lines) rather than adding a crate
dependency. The exercise catalogue is 48 entries; performance is irrelevant.

## Response parsing (assistant/parser.rs)

```rust
pub fn parse_assistant_response(raw: &str) -> AssistantResponse {
    // 1. Try extract_json() to strip markdown fences and find JSON substring
    let json_str = extract_json(raw);

    // 2. Try parsing as AssistantResponse
    match serde_json::from_str::<AssistantResponse>(json_str) {
        Ok(parsed) => parsed,
        Err(e) => {
            tracing::warn!("Failed to parse LLM response as JSON: {e}");
            // Fallback: treat entire response as plain text message, no actions
            AssistantResponse {
                message: raw.to_string(),
                actions: vec![],
            }
        }
    }
}
```

Don't copy the `extract_json` function. A dependency on corre-sdk is acceptable.

## Dependencies to add

```toml
# In apps/corre-gym/Cargo.toml [dependencies]
corre-llm = { workspace = true }
corre-sdk = { workspace = true }
reqwest = { workspace = true }
tracing-subscriber = { workspace = true }
dotenvy = { workspace = true }
clap = { workspace = true }
dirs = { workspace = true }
toml = { workspace = true }
async-trait = { workspace = true }
```

All crates are already workspace dependencies. No new workspace-level additions needed.

Order crates alphabetically.

## Tests

### Action parsing tests
- Parse well-formed JSON response
- Parse response with markdown fences (```json ... ```)
- Parse response with multiple actions
- Fallback on malformed JSON (returns raw text as message)
- Parse each action type individually
- Unknown action type deserializes as `Unknown` (doesn't break other actions)
- Missing optional fields (sets, reps, difficulty) parse without error
- `"actions": null` defaults to empty vec
- `"actions"` field absent defaults to empty vec

### Exercise matching tests
- Exact match case-insensitive: "barbell bench press" → "Barbell Bench Press"
- Alias match: "bench" → "Barbell Bench Press"
- Alias match: "dl" → "Conventional Deadlift"
- Contains single match: "Cable Fly" matches
- Contains ambiguous: "curl" matches multiple, falls through to edit distance
- Levenshtein close match: "Barbel Bench Press" (typo) → "Barbell Bench Press"
- Levenshtein too distant: "yoga" → None
- No match: "Underwater Basket Weaving" → None
- Levenshtein function correctness (direct unit tests)

### Prompt construction tests
- System prompt includes user's active health entries
- System prompt shows "No active session" when appropriate
- System prompt includes active session + current exercise logs
- Exercise list is grouped by muscle group with correct format
- Recent history is correctly formatted

### Handler tests (with mock LLM)

Create a `MockLlm` that implements `LlmProvider` and returns predetermined responses.
Use in-memory SQLite database for isolation.

- User auto-registration on first message
- Exercise logging creates correct DB records
- Session auto-start when logging without active session
- Stale session auto-close (session from 5 hours ago gets closed before new exercise)
- `/start` command sends welcome for new user
- `/start` command for existing user shows "already registered" note
- `/help` command lists capabilities
- `/status` shows current state
- Multiple actions in one response all execute
- Partial action failure appends note to reply
- Message truncation at max_message_length boundary

### Authorization tests
- Message from allowed ID is processed
- Message from disallowed ID is silently dropped
- Empty allowed_ids list permits all users

### Integration test with mock Telegram
- Spin up a local HTTP server that mimics Telegram API (getMe, getUpdates, sendMessage)
- Queue updates, verify the bot sends correct replies
- Verify typing indicator is sent before the reply
- A variety of human-like messages (imprecise but representative) with a mock LLM returning
  correct structured JSON results in the right data being recorded or retrieved

## Verification

```sh
# Unit tests
cargo test -p corre-gym

# Manual testing
export TELEGRAM_GYM_BOT_TOKEN="your-token-here"
cargo run -p corre-gym -- -c ~/.local/share/corre/corre.toml

# Then in Telegram:
# 1. Send /start to your bot
# 2. "I just did 3 sets of bench press at 80kg, 8 reps, felt medium"
# 3. "What did I do today?"
# 4. /status
# 5. "My left shoulder is a bit sore"
# 6. "End my session"
# 7. /history
```

## Implementation sequence

1. corre-core change: add `gym: toml::Value` field to `CorreConfig`
2. `config.rs`: GymConfig struct, from_toml_table, resolve_secrets
3. `telegram/types.rs`: all Telegram serde types
4. `telegram/client.rs`: TelegramClient with get_me, get_updates, send_message, send_chat_action
5. `assistant/matching.rs`: find_exercise + levenshtein (pure functions, easy to test first)
6. `assistant/parser.rs`: parse_assistant_response with extract_json (pure functions)
7. `assistant/actions.rs`: AssistantAction enum, AssistantResponse struct
8. `assistant/prompts.rs`: system prompt builder, context formatting
9. `assistant/handler.rs`: AssistantHandler with full flow
10. `main.rs`: wire everything together, polling loop, graceful shutdown
11. Tests: unit tests (matching, parser, prompts), handler integration, mock Telegram

Steps 2-8 have no inter-dependencies beyond types and can be developed in parallel.
Step 9 integrates everything. Step 10 is thin glue code.
