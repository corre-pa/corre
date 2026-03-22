# Gym Tracker / Personal Trainer -- Overall Plan

## What this is

A voice-driven gym tracker and personal trainer for Corre. Users interact via Telegram
voice/text messages. An LLM-powered assistant records workouts, suggests exercises, tracks
injuries, sends reminders, and provides coaching. A web dashboard shows progress, history,
and goals.

## Key architectural decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Messaging platform | Telegram first, Signal later | Telegram has a mature Bot API with voice message support. Signal has no official bot API. |
| Voice processing | Local whisper.cpp + Piper TTS | Privacy-centric, no external API calls. Aligns with Corre's philosophy. |
| Telegram integration | Direct Bot API in daemon | MCP is request/response, not suitable for real-time event streaming. The daemon long-polls Telegram directly. |
| Deployment model | Standalone binary (like corre-news) | Corre host doesn't support daemon mode yet. Standalone binary managed by Docker/systemd. |
| Database | SQLite with raw SQL | Consistent with project philosophy (no ORMs). rusqlite already in workspace. |
| LLM access | Direct via corre-llm crate | Standalone binary can depend on corre-llm directly (no CCPP needed). |

## Component architecture

```
crates/corre-gym/                 Single binary: Telegram bot + web dashboard
  src/
    main.rs                       Starts Telegram poller + Axum HTTP server concurrently
    config.rs                     Load [gym] section from corre.toml
    db/                           SQLite: schema, models, CRUD (13 files)
    telegram/                     Telegram Bot API client (long-polling, voice)
    voice/                        whisper.cpp STT + Piper TTS
    assistant/                    LLM conversation handler + action parser
    web/                          Axum routes, Telegram Login auth, chart APIs
    scheduler/                    tokio-cron-scheduler for reminders
  static/                         CSS, JS (Chart.js)
  templates/                      Askama HTML templates
  Dockerfile
```

### Dependencies

```
corre-gym
  |-- corre-core          (config, types)
  |-- corre-llm           (LLM provider)
  |-- rusqlite             (SQLite)
  |-- axum                 (web server)
  |-- reqwest              (Telegram API, file downloads)
  |-- askama               (HTML templates)
  |-- tokio-cron-scheduler (reminder scheduling)
  |-- chrono, serde, uuid, tracing, anyhow  (standard)
```

Does NOT depend on corre-sdk, corre-mcp, or any CCPP protocol machinery.

### Config (corre.toml)

```toml
[gym]
bind = "127.0.0.1:5520"
telegram_bot_token = "${TELEGRAM_GYM_BOT_TOKEN}"
default_timezone = "Europe/London"
conversation_history_limit = 20
db_path = "gym-tracker.db"

[gym.voice]
stt_enabled = true
whisper_binary = "whisper-cli"
whisper_model = "base.en"
tts_enabled = true
piper_binary = "piper"
piper_model = "en_US-lessac-medium"
response_mode = "both"               # "voice", "text", or "both"
```

### Docker compose

```yaml
corre-gym:
  image: ghcr.io/corre-pa/corre-gym:latest
  build:
    context: .
    dockerfile: crates/corre-gym/Dockerfile
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

## Milestones

| # | Title | Depends on | What it delivers |
|---|-------|------------|------------------|
| 0 | Database + Data Model | -- | SQLite schema, all domain types, CRUD, access control, seed data, tests |
| 1 | Telegram Text Chat | M0 | Telegram bot, LLM assistant, exercise logging from text, auto-registration |
| 2 | Voice Pipeline | M1 | whisper.cpp STT, Piper TTS, voice messages in/out via Telegram |
| 3 | Web Dashboard | M0, M1 | Axum web server, Telegram Login auth, progress charts, history, chat |
| 4 | Schedules & Reminders | M1 | Cron-based reminders, programme creation via chat, escalation logic |
| 5 | Health Tracking | M1 | Injury/illness logging, adaptive programmes, recovery check-ins |
| 6 | Signal Integration | M1, M2 | signal-cli sidecar, mcp-signal, messaging abstraction trait |

Each milestone produces a working, testable increment. Milestones 3-5 can be developed
in parallel after M1 is complete.

## Existing code to reuse

| What | Where | How |
|------|-------|-----|
| SQLite Database wrapper | `apps/rolodex/src/db/db.rs` | Same open/migrate pattern with `CREATE TABLE IF NOT EXISTS` |
| Domain model enums | `apps/rolodex/src/db/models.rs` | Same serde enum pattern with `as_str()` / `from_str_loose()` |
| Standalone Axum server | `crates/corre-news/src/main.rs` | Same binary structure, config loading, static asset serving |
| Config ${VAR} resolution | `crates/corre-core/src/config.rs` | Reuse env var resolution for bot tokens |
| LLM chat completions | `crates/corre-llm/` | `OpenAiCompatProvider` for all LLM calls |
| Telegram HTTP calls | `crates/mcp-telegram/src/main.rs` | Same reqwest pattern for Bot API |
| JSON extraction from LLM | `crates/corre-sdk/src/tools.rs` | `extract_json()` for structured output parsing |
| HTML sanitization | `crates/corre-sdk/src/html.rs` | `sanitize_html()` for user-provided content in dashboard |

## Open items for later milestones

- **Daemon mode in corre-host**: When `ExecutionMode::Daemon` is implemented in the host,
  the gym tracker could optionally migrate to a CCPP daemon app. Not blocking.
- **Signal**: Requires signal-cli (Java) as a Docker sidecar. Fragile, deferred.
- **WhatsApp**: Requires WhatsApp Business API (paid, complex). Not planned.
- **Corre dashboard integration**: A link to `https://host:5520` from the operator dashboard.
  Can be done via a manifest `[[plugin.links]]` entry once host integration is added.
