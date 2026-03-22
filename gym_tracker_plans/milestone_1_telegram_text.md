# Milestone 1: Telegram Text Chat

## Goal

Users text a Telegram bot. The LLM-powered assistant understands gym-related commands,
records exercises, manages sessions, and responds conversationally. Text only -- no voice yet.

## Prerequisites

- Milestone 0 complete (database + CRUD layer)
- A Telegram bot created via @BotFather (provides the bot token)
- LLM provider configured in `corre.toml [llm]` section

## File structure

```
crates/corre-gym/src/
    main.rs                 Start Telegram polling loop
    config.rs               Load [gym] section from corre.toml
    telegram/
      mod.rs                Re-exports
      client.rs             Telegram Bot API client (long-polling)
      types.rs              Telegram API response/request types
    assistant/
      mod.rs                Re-exports
      handler.rs            Message orchestrator (context -> LLM -> actions -> reply)
      prompts.rs            System prompt templates
      actions.rs            Structured action types + execution
      parser.rs             Extract JSON actions from LLM response
```

## Config (config.rs)

Add a `[gym]` section to `corre.toml`:

```toml
[gym]
bind = "127.0.0.1:5520"              # web dashboard (M3, unused here)
telegram_bot_token = "${TELEGRAM_GYM_BOT_TOKEN}"
default_timezone = "Europe/London"
conversation_history_limit = 20       # messages to include in LLM context
db_path = "gym-tracker.db"            # relative to data_dir
```

Config struct:

```rust
#[derive(Debug, Deserialize)]
pub struct GymConfig {
    pub bind: String,
    pub telegram_bot_token: String,
    pub default_timezone: String,
    #[serde(default = "default_history_limit")]
    pub conversation_history_limit: usize,
    #[serde(default = "default_db_path")]
    pub db_path: String,
}
```

Reuse `corre-core`'s config loading with `${VAR}` env resolution.

## Telegram Bot API client (telegram/client.rs)

Lightweight HTTP client using `reqwest`. No external Telegram crate.

```rust
pub struct TelegramClient {
    token: String,
    client: reqwest::Client,
}

impl TelegramClient {
    pub fn new(token: &str) -> Self;

    /// Long-poll for updates. Returns Vec<Update>.
    /// Blocks for up to `timeout` seconds if no updates.
    pub async fn get_updates(&self, offset: i64, timeout: u32) -> Result<Vec<Update>>;

    /// Send a text message. Returns the sent Message.
    pub async fn send_message(&self, chat_id: i64, text: &str, parse_mode: Option<&str>) -> Result<Message>;

    /// Send a "typing..." indicator.
    pub async fn send_chat_action(&self, chat_id: i64, action: &str) -> Result<()>;

    /// Get bot info (for startup verification).
    pub async fn get_me(&self) -> Result<BotUser>;
}
```

All methods call `https://api.telegram.org/bot{token}/{method}` via POST with JSON body.

### Telegram types (telegram/types.rs)

Minimal serde structs for the Bot API responses we need:

```rust
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

pub struct Message {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: Chat,
    pub date: i64,
    pub text: Option<String>,
    pub voice: Option<Voice>,        // for M2
}

pub struct TelegramUser {
    pub id: i64,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

pub struct Chat {
    pub id: i64,
}

pub struct Voice {
    pub file_id: String,
    pub duration: i32,
}

pub struct BotUser {
    pub id: i64,
    pub username: Option<String>,
}
```

## Main loop (main.rs)

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Load config
    let config = load_config()?;

    // 2. Open database
    let db = Database::open(&data_dir.join(&config.gym.db_path))?;
    db.seed_exercises()?;

    // 3. Initialize LLM provider
    let llm = OpenAiCompatProvider::from_config(&config.llm)?;

    // 4. Create Telegram client
    let telegram = TelegramClient::new(&config.gym.telegram_bot_token);
    let me = telegram.get_me().await?;
    tracing::info!("Telegram bot @{} connected", me.username.unwrap_or_default());

    // 5. Create assistant handler
    let handler = AssistantHandler::new(db, llm, config.gym);

    // 6. Long-polling loop
    let mut offset = 0i64;
    loop {
        let updates = telegram.get_updates(offset, 30).await?;
        for update in updates {
            offset = update.update_id + 1;
            if let Some(message) = update.message {
                if let Some(text) = &message.text {
                    // Send typing indicator
                    telegram.send_chat_action(message.chat.id, "typing").await.ok();
                    // Process message
                    let reply = handler.handle_text_message(&message, text).await;
                    telegram.send_message(message.chat.id, &reply, Some("Markdown")).await?;
                }
            }
        }
    }
}
```

## Assistant handler (assistant/handler.rs)

The core orchestration logic:

```rust
pub struct AssistantHandler {
    db: Database,
    llm: OpenAiCompatProvider,
    config: GymConfig,
}

impl AssistantHandler {
    /// Process an incoming text message and return a reply string.
    pub async fn handle_text_message(&self, message: &Message, text: &str) -> String {
        // 1. Identify user (auto-register if new)
        let user = self.ensure_user(message).unwrap();

        // 2. Check for slash commands (/start, /help, /status)
        if let Some(reply) = self.handle_command(text, &user) {
            return reply;
        }

        // 3. Build LLM context
        let context = self.build_context(&user).unwrap();

        // 4. Call LLM
        let llm_response = self.call_llm(&context, text).await;

        // 5. Parse response into message + actions
        let parsed = parse_assistant_response(&llm_response);

        // 6. Execute actions (DB mutations)
        for action in &parsed.actions {
            self.execute_action(action, &user).unwrap_or_else(|e| {
                tracing::warn!("Action failed: {e:#}");
            });
        }

        // 7. Store conversation turn
        self.store_conversation(&user.id, text, &parsed.message).unwrap();

        // 8. Return reply
        parsed.message
    }
}
```

### User auto-registration

On first contact, create a User record:
- `id`: UUID v4
- `name`: Telegram `first_name` (+ `last_name` if present)
- `telegram_id`: Telegram user ID (as string)
- `timezone`: default from config

Send a welcome message explaining the bot's capabilities.

### Slash commands

| Command | Action |
|---------|--------|
| `/start` | Welcome message + auto-register |
| `/help` | List available commands and example phrases |
| `/status` | Current session state, today's stats, active health issues |
| `/history` | Last 5 workouts summary |
| `/exercises` | List available exercises by muscle group |

## LLM integration (assistant/prompts.rs)

### System prompt template

```
You are a personal gym trainer assistant. You help users track their workouts,
suggest exercises, and provide coaching advice.

You MUST respond with a JSON object in this exact format:
{
  "message": "Your conversational response to the user",
  "actions": [<list of actions to execute, or empty array>]
}

Available action types:
- {"type": "log_exercise", "exercise": "<name>", "sets": N, "reps": N, "weight_kg": N.N, "difficulty": "easy|medium|hard|failure"}
- {"type": "log_exercise_timed", "exercise": "<name>", "duration_secs": N, "difficulty": "..."}
- {"type": "start_session", "notes": "<optional>"}
- {"type": "end_session"}
- {"type": "log_health", "entry_type": "injury|illness|wellbeing", "body_part": "<optional>", "severity": "mild|moderate|severe", "description": "..."}
- {"type": "set_target", "exercise": "<name>", "target_value": "...", "end_date": "<optional YYYY-MM-DD>"}
- {"type": "none"}

Current user: {user_name}
Current time: {current_time} ({timezone})
Active session: {session_status}

Active health issues:
{health_entries}

Recent workout history (last 7 days):
{recent_history}

Today's scheduled workout:
{today_schedule}

Available exercises:
{exercise_list}

Guidelines:
- When the user reports an exercise, always log it with an action
- Start a session automatically if the user logs an exercise without one active
- Be encouraging but not patronizing
- If the user mentions pain or injury, log it as a health entry and suggest modifications
- Use exercise names exactly as they appear in the available exercises list
- If an exercise isn't in the list, use the closest match and mention it
- Keep responses concise -- this is a chat interface, not an essay
```

### Context building

```rust
fn build_context(&self, user: &User) -> Result<String> {
    let recent_messages = self.db.get_recent_messages(&user.id, self.config.conversation_history_limit)?;
    let active_session = self.db.get_active_session(&user.id)?;
    let health_entries = self.db.list_active_health_entries(&user.id)?;
    let recent_logs = self.db.get_recent_logs(&user.id, 7)?;
    let schedules = self.db.list_schedules(&user.id)?;
    let exercises = self.db.list_exercises()?;

    // Format into system prompt template
    // ...
}
```

### LLM call

Use `corre-llm::OpenAiCompatProvider::complete()` with:
- System message: filled template above
- Conversation history: last N messages as user/assistant pairs
- Current user message

Temperature: 0.3 (structured output needs low temp).
Use `json_mode: true` if the provider supports it.

## Action types and execution (assistant/actions.rs)

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantAction {
    LogExercise {
        exercise: String,
        sets: Option<i32>,
        reps: Option<i32>,
        weight_kg: Option<f64>,
        difficulty: Option<String>,
    },
    LogExerciseTimed {
        exercise: String,
        duration_secs: i32,
        difficulty: Option<String>,
    },
    StartSession {
        notes: Option<String>,
    },
    EndSession,
    LogHealth {
        entry_type: String,
        body_part: Option<String>,
        severity: Option<String>,
        description: String,
    },
    SetTarget {
        exercise: String,
        target_value: String,
        end_date: Option<String>,
    },
    None,
}

#[derive(Debug, Deserialize)]
pub struct AssistantResponse {
    pub message: String,
    pub actions: Vec<AssistantAction>,
}
```

Action execution:

```rust
fn execute_action(&self, action: &AssistantAction, user: &User) -> Result<()> {
    match action {
        AssistantAction::LogExercise { exercise, sets, reps, weight_kg, difficulty } => {
            // Look up exercise by name (fuzzy match)
            let ex = self.find_exercise(exercise)?;
            // Auto-start session if none active
            let session = self.ensure_session(user)?;
            // Insert exercise log
            self.db.insert_log(&ExerciseLog { ... })?;
        }
        AssistantAction::StartSession { notes } => {
            self.db.start_session(&user.id, notes.as_deref())?;
        }
        AssistantAction::EndSession => {
            if let Some(session) = self.db.get_active_session(&user.id)? {
                self.db.end_session(&session.id)?;
            }
        }
        // ... other actions
    }
}
```

### Fuzzy exercise matching

When the LLM returns an exercise name that doesn't exactly match the catalogue:
1. Try exact match (case-insensitive)
2. Try contains match
3. Try Levenshtein distance (use `strsim` crate or simple impl)
4. If no match, log a warning and ask the user to clarify

## Response parsing (assistant/parser.rs)

```rust
pub fn parse_assistant_response(raw: &str) -> AssistantResponse {
    // 1. Try extract_json() to strip markdown fences
    let json_str = corre_sdk::tools::extract_json(raw);

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

## Dependencies to add

```toml
# In crates/corre-gym/Cargo.toml
[dependencies]
corre-llm = { workspace = true }
reqwest = { workspace = true }
tracing-subscriber = { workspace = true }
dotenvy = { workspace = true }
```

And add `corre-sdk` as a dev or regular dependency for `extract_json()`.
(Or copy the small `extract_json` function to avoid the dep -- it's ~20 lines.)

## Tests

### Action parsing tests
- Parse well-formed JSON response
- Parse response with markdown fences (```json ... ```)
- Parse response with multiple actions
- Fallback on malformed JSON (returns raw text as message)
- Parse each action type individually

### Prompt construction tests
- Verify system prompt includes user's health entries
- Verify recent history is correctly formatted
- Verify exercise list is included
- Verify conversation history is in correct order

### Handler tests (with mock LLM)
- User auto-registration on first message
- Exercise logging creates correct DB records
- Session auto-start when logging without active session
- `/start` command sends welcome
- `/help` command lists capabilities
- `/status` shows current state

### Integration test with mock Telegram
- Spin up a local HTTP server that mimics Telegram API
- Verify the polling loop processes messages correctly
- Verify responses are sent back

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
```
