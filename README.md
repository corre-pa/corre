# Corre

> Last updated at v0.19.0

A simple, safe, easy-to-use personal AI task scheduler that runs modular *apps* on cron schedules.

## Goals

- **Privacy-centric**: All data stays local. Use whichever LLM provider you like, but Corre defaults to privacy-centric services that 
  use open-source models and don't log your sessions (Venice.ai, Brave Search, Kagi etc.).
- **Modular apps**: Each task (daily news brief, assistant, birthday reminders, ...) is a self-contained app that can be
  installed, configured, and removed independently.
- **MCP-native**: Apps interact with the outside world through
  [Model Context Protocol](https://modelcontextprotocol.io/) servers. We've curated a pool of MCP servers for web search, calendar 
  access, email sending, and more, and you can add your own.
- **Deterministic orchestration**: Scheduling is done with cron expressions, not open-ended LLM agent loops. We don't use LLMs to solve 
  problems that Unix solved 60 years ago.
- **Accessible anywhere**: CorreNews binds to `127.0.0.1` by default and is designed to sit behind a NAT-punching solution (Headscale, 
  WireGuard, Tor hidden service) so you can read your personal newspaper from any device without exposing it to the public internet.
- **Security-minded**: MCP servers and apps each run in their own sandbox. Apps have to provide a manifest of every MCP
  server they use, and you _can_ configure fine-grained permissions for file access, network access and more. However, the app
  registry has pre-configured permissions to the absolute minimum so that you don't have to fiddle with manifests.

### Included app: Daily Research Brief

The first end-to-end app ships with the MVP. Each morning it:

1. Reads topics from `config/topics.yml`
2. Searches the web for each topic via the Brave Search MCP server
3. Deduplicates results and asks an LLM to score them for newsworthiness
4. Summarises the top stories
5. Publishes a new CorreNews edition

## Setup

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.85+ | Edition 2024 is used. Install via [rustup](https://rustup.rs) |
| Node.js / npm | 18+ | Only needed for `npx`-based MCP servers (e.g. Brave Search) |

### 1. Clone and build

```sh
git clone <repo-url> corre && cd corre
cargo build --release
```

The binary is at `target/release/corre`.

### 2. Set up API keys

Corre never stores secrets in config files. Instead, `corre.toml` uses `${VAR}` references that
are resolved from the environment at runtime. Set the actual values in your shell or `.env` file.

```sh
# LLM provider (Venice.ai by default, or any OpenAI-compatible API)
export VENICE_API_KEY="your-venice-api-key"

# Brave Search (used by the daily-brief app)
export BRAVE_API_KEY="your-brave-api-key"
```

### 3. Configure

Configuration lives in the data directory at `~/.local/share/corre/` (Linux) or
`~/Library/Application Support/corre/` (macOS). Running `corre setup` creates these files
automatically. You can override the config path with `corre -c /path/to/corre.toml`.

Edit `~/.local/share/corre/corre.toml` to point at your preferred LLM provider:

```toml
[llm]
provider = "openai-compatible"
base_url = "https://api.venice.ai/api/v1"   # or http://localhost:11434/v1 for Ollama
model = "nvidia-nemotron-3-nano-30b-a3b"
api_key = "${VENICE_API_KEY}"                # ${VAR} references are resolved from the environment
temperature = 0.3
max_concurrent = 10                          # parallel LLM requests; tune to your provider's rate limit
```

Edit `~/.local/share/corre/config/topics.yml` to choose your daily brief topics:

```markdown
## Technology
- Rust programming language news
- AI and machine learning developments

## World News
- Geopolitics and international relations
```

### 4. Run an app manually

```sh
# One-shot run -- executes the daily brief immediately and exits
corre run-now daily-brief
```

The edition is written to `~/.local/share/corre/editions/YYYY-MM-DD/edition.json`.

### 5. Start the full daemon

```sh
corre run
```

This starts the cron scheduler and the operator dashboard. Apps fire on their configured
schedules (the daily brief defaults to `0 0 5 * * *` -- 05:00 every day). The CorreNews web
server runs as a separate service (see `corre-news`).

### CLI reference

```
corre [OPTIONS] <COMMAND>

Commands:
  run           Start the full daemon (scheduler + dashboard)
  run-now       Run a single app immediately and exit
  setup         Interactive setup wizard — configure LLM, API keys, topics, and systemd
  install-deps  Check and install required external dependencies
  health        Health check: verify data dir and config are accessible (exit 0/1)

Options:
  -c, --config <CONFIG>  Path to config file [default: ~/.local/share/corre/corre.toml]
  -h, --help             Print help
```

Running `corre` with no subcommand launches the setup wizard if no config exists, or prints help
otherwise.

## Remote access via Tailscale

Corre binds to `127.0.0.1` by default, so it is only reachable from the host machine. To read
CorreNews from your phone, laptop, or any other device without exposing it to the public
internet, you can enable the built-in [Tailscale](https://tailscale.com/) integration. Tailscale
creates an encrypted WireGuard mesh between your devices and gives each node a stable DNS name
with automatic HTTPS certificates.

### 1. Prepare your Tailscale account

1. Sign up at [login.tailscale.com](https://login.tailscale.com/) (free for personal use).
2. In the admin console, enable **MagicDNS** (Settings > DNS).
3. Enable **HTTPS Certificates** (Settings > DNS > HTTPS Certificates).
4. Generate an auth key (Settings > Keys > Generate auth key). Reusable keys are convenient
   for containers that may restart. Copy the key -- it looks like `tskey-auth-...`.

### 2. Configure the Corre host

Add these variables to your `.env` file:

```sh
TAILSCALE_ENABLED=true
TAILSCALE_AUTHKEY="tskey-auth-..."
TS_HOSTNAME=corre                   # the MagicDNS name for this node
```

If you use a self-hosted [Headscale](https://github.com/juanfont/headscale) coordination
server instead of Tailscale's hosted control plane, also set:

```sh
TAILSCALE_LOGIN_SERVER="https://headscale.example.com"
```

Start (or restart) the stack:

```sh
docker compose up -d
```

Check the logs for confirmation:

```sh
docker compose logs corre-core | grep "Tailscale is up"
# Expected: Tailscale is up: 100.x.x.x
```

Once running, the services are available over the tailnet with automatic HTTPS:

| Service | Port | URL |
|---------|------|-----|
| Dashboard | 5500 | `https://corre.<tailnet>.ts.net:5500/` |
| CorreNews | 5510 | `https://corre.<tailnet>.ts.net:5510/` |
| Registry  | 5580 | `https://corre.<tailnet>.ts.net:5580/` |

Replace `corre` with whatever you set `TS_HOSTNAME` to, and `<tailnet>` with your tailnet name
(visible in the admin console, e.g. `tail1234a.ts.net`).

### 3. Install Tailscale on your devices

Every device that needs to reach Corre must be on the same tailnet. Install Tailscale, sign in
with the same account, and you're connected.

**Linux**

```sh
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

**macOS**

Install from the [Mac App Store](https://apps.apple.com/app/tailscale/id1475387142) or via
Homebrew:

```sh
brew install --cask tailscale
```

Open Tailscale from the menu bar and sign in.

**Windows**

Download the installer from [tailscale.com/download/windows](https://tailscale.com/download/windows),
run it, and sign in from the system tray icon.

**iOS**

Install [Tailscale](https://apps.apple.com/app/tailscale/id1470499037) from the App Store,
open it, and sign in.

**Android**

Install [Tailscale](https://play.google.com/store/apps/details?id=com.tailscale.ipn) from
Google Play, open it, and sign in.

### 4. Access Corre from any device

Once Tailscale is running on both the server and your device, open a browser and navigate to:

- **Dashboard:** `https://corre.<tailnet>.ts.net:5500/`
- **CorreNews:** `https://corre.<tailnet>.ts.net:5510/`

The HTTPS certificates are provisioned automatically by Tailscale via Let's Encrypt -- no
manual certificate setup is needed. The connection is end-to-end encrypted over WireGuard and
never traverses the public internet.

### Troubleshooting

| Symptom | Fix |
|---------|-----|
| `Tailscale is up` never appears in logs | Check that `TAILSCALE_AUTHKEY` is valid and not expired. Generate a new key in the admin console if needed. |
| Browser shows certificate error | Ensure HTTPS Certificates are enabled in the Tailscale admin DNS settings. |
| Connection times out from a device | Verify the device is signed into the same Tailscale account and shows as "Connected" in the client. |
| `corre-news` unreachable on 5510 | Confirm both containers are on the `corre-internal` Docker network (`docker network inspect corre-internal`). |
| Dashboard loads but CorreNews doesn't | `corre-news` depends on `corre-core` being healthy. Check `docker compose ps` and `docker compose logs corre-news`. |

## Architecture

### Workspace layout

```
corre/                              # source repository
  templates/
    newspaper.html                  # CorreNews edition template
    topics.html                     # topics editor page
  static/
    style.css                       # newspaper CSS
    settings.css                    # settings/topics page CSS
    topics.js                       # topics editor JS
  crates/
    corre-sdk/                      # types + protocol for subprocess apps
    corre-core/                     # shared types, traits, config, scheduler
    corre-host/                     # subprocess app host (CCPP protocol)
    corre-mcp/                      # MCP server pool
    corre-llm/                      # LLM provider (OpenAI-compatible)
    corre-news/                     # CorreNews web server + archive + search
    corre-dashboard/                # operator dashboard web UI + API
    corre-plugin/                   # built-in app implementations
    corre-safety/                   # prompt injection defense middleware
    corre-registry/                 # MCP server + app registry client
    corre-cli/                      # binary entry point
    mcp-smtp/                       # SMTP email MCP server
    mcp-telegram/                   # Telegram messaging MCP server
  apps/
    daily-brief/                    # daily research brief (subprocess app)
    rolodex/                        # personal contact engagement (subprocess app)

~/.local/share/corre/               # runtime data directory
  corre.toml                        # main config file
  .env                              # API keys (created by `corre setup`)
  config/
    topics.yml                      # user-editable per-app config
    mcp/                            # per-MCP server config files (*.toml)
  editions/
    YYYY-MM-DD/edition.json         # archived editions
  plugins/                          # installed app plugins
  bin/                              # locally installed MCP server binaries
  search_index/                     # Tantivy full-text search index
  app_logs/                         # daily-rotating app log files
```

### Crate dependency graph

```
corre-cli
  |-- corre-core       --> corre-sdk
  |-- corre-host       --> corre-core, corre-sdk
  |-- corre-mcp        --> corre-core
  |-- corre-llm        --> corre-core
  |-- corre-safety     --> corre-core
  |-- corre-plugin      --> corre-core, corre-mcp, corre-llm
  |-- corre-dashboard  --> corre-core, corre-sdk, corre-registry
  |-- corre-registry

corre-news (standalone binary)
  |-- corre-core

daily-brief (subprocess app)
  |-- corre-sdk

rolodex (subprocess app)
  |-- corre-sdk
```

`corre-sdk` sits at the very bottom with zero internal dependencies — it defines the types and
protocol that subprocess apps use to communicate with the host. `corre-core` depends
only on `corre-sdk` and defines the trait abstractions that the host-side crates implement.

### Key abstractions (`corre-core`)

**`App`** -- the unit of work. Each app declares a manifest (name, cron schedule,
required MCP servers) and an `execute` method that receives a context and returns articles.
Apps can be built-in (compiled into the host binary) or subprocess plugins that
communicate over stdin/stdout using the CCPP (Corre Capability Plugin Protocol) JSON-RPC
protocol — see `APP_GUIDE.md` for details.

```rust
#[async_trait]
pub trait App: Send + Sync {
    fn manifest(&self) -> &AppManifest;
    async fn execute(&self, ctx: &AppContext) -> anyhow::Result<AppOutput>;
}
```

**`LlmProvider`** -- abstracts over any chat-completion API. The single implementation
(`OpenAiCompatProvider` in `corre-llm`) speaks the OpenAI wire format, which Venice.ai, Ollama,
and many others support.

**`McpCaller`** -- abstracts over MCP tool invocation, allowing apps to call MCP tools
without depending on `corre-mcp` directly.

**Publishing types** -- `Edition` > `Section` > `Article`. An edition is a dated collection of
sections, each containing scored articles with sources. The highest-scoring article title becomes
the edition headline.


### MCP server pool (`corre-mcp`)

Each MCP server has its own config file at `~/.local/share/corre/config/mcp/{name}.toml`
(e.g. `brave-search.toml`). These are created automatically when you install an MCP server
through the dashboard or the registry. A typical MCP config file:

```toml
registry_id = "brave-search"
command = "npx"
args = ["-y", "@brave/brave-search-mcp-server"]
installed = true

[env]
BRAVE_API_KEY = "${BRAVE_API_KEY}"
```

MCP servers are started lazily as stdio child processes on first use, cached for the duration
of an app run, and shut down afterward. Environment variable references (`${VAR}` syntax)
in the `env` table are resolved at spawn time from the host environment.

### CorreNews web server (`corre-news`)

Standalone binary (port 5510 by default). Axum HTTP server with askama-rendered newspaper
templates, a filesystem edition archive, and a Tantivy full-text search index.

Routes:

| Path | Description |
|------|-------------|
| `GET /` | Latest edition |
| `GET /edition/{date}` | Specific edition with archive navigation |
| `GET /api/dates` | List available edition dates (JSON) |
| `GET /search?q=...&limit=N` | Full-text search (returns JSON) |
| `GET /settings/topics` | Topics editor page (requires `editor_token`) |
| `GET /api/topics` | Read topics config (JSON) |
| `PUT /api/topics` | Update topics config |
| `GET /plugin/{name}/static/{*path}` | Plugin static assets |
| `GET /static/*` | CSS and static assets |

Set `editor_token` in the `[news]` section to enable the `/settings/topics` page — requests
must include `?token=<value>` to authenticate.

### Operator dashboard (`corre-dashboard`)

Embedded in the `corre run` process (port 5500 by default). Provides a web UI and REST API
for monitoring and managing apps at runtime.

Key API routes:

| Path | Description |
|------|-------------|
| `GET /` | Dashboard web UI |
| `GET /api/dashboard/status` | App execution status (JSON) |
| `POST /api/dashboard/run/{name}` | Trigger an app run immediately |
| `GET /api/dashboard/events` | SSE stream of real-time progress and log events |
| `GET /api/dashboard/logs/{date}` | Historical log entries for a given date |
| `GET,PUT /api/settings` | Read/update `corre.toml` settings |
| `GET,PUT /api/config/{name}` | Read/update per-app config files |
| `GET /api/registry/catalog` | Browse the MCP server registry |
| `GET /api/registry/search` | Search the registry |
| `POST /api/registry/refresh` | Force-refresh the registry cache |
| `GET /api/mcp/installed` | List installed MCP servers |
| `POST /api/mcp/install` | Install an MCP server from the registry |
| `POST /api/mcp/uninstall/{name}` | Uninstall an MCP server |
| `POST /api/mcp/test/{name}` | Test an MCP server connection |
| `GET /api/mcp/config/{name}` | Read MCP server config |
| `PUT /api/mcp/configure/{name}` | Update MCP server config |
| `GET /api/apps/installed` | List installed app plugins |
| `POST /api/apps/install` | Install an app plugin |
| `POST /api/apps/uninstall/{name}` | Uninstall an app plugin |
| `GET /api/services` | List managed services |
| `POST /api/services/start` | Start a service |
| `POST /api/services/stop/{name}` | Stop a service |
| `POST /api/system/restart` | Graceful restart |

### Scheduler (`corre-core` + `corre-cli`)

The scheduler wraps `tokio-cron-scheduler`. For each enabled app in config, the CLI
registers an async callback that:

1. Builds an MCP pool with the app's declared servers
2. Initializes the LLM provider (with per-app model/temperature overrides if configured)
3. Wraps MCP and LLM with the safety layer (if enabled)
4. Runs the app in an isolated `tokio` task
5. Forwards real-time progress events to the dashboard via SSE

The dashboard can also trigger apps on demand via `POST /api/dashboard/run/{name}`.
A panic or timeout in one app does not affect the scheduler or other apps.

### Daily Brief pipeline (`daily-brief`)

The daily brief runs as a subprocess app (its own binary communicating via CCPP).
A deterministic, multi-step pipeline with LLM calls at specific points:

1. **Parse** `config/topics.yml` into sections and search queries
2. **Search** each query via the `brave-search` MCP server
3. **Deduplicate** results by URL within each section (and cross-edition via `seen_urls`)
4. **Score** results for newsworthiness (LLM call, structured JSON output)
5. **Filter** to top results per section above the score threshold
6. **Summarise** each top result in 2-3 paragraphs (LLM call)
7. **Emit** `AppOutput` with articles grouped by section

### Safety layer (`corre-safety`)

Corre fetches external content (web search results) via MCP servers and feeds it to LLMs for
scoring and summarization. A malicious web page could embed prompt injection phrases in its
metadata — "ignore previous instructions", special tokens like `<|system|>`, or encoded
payloads — and the LLM might comply. The safety layer sits between MCP tool outputs and LLM
prompts to neutralize these attacks.

Safety is **enabled by default**. It wraps `McpCaller` and `LlmProvider` transparently, so
apps require no code changes.

The pipeline applies four stages to every MCP tool output:

1. **Validation** — truncates oversized outputs, strips null bytes, collapses whitespace
   obfuscation, and truncates token-stuffing runs
2. **Sanitization** — detects ~45 known injection phrases via Aho-Corasick (case-insensitive),
   neutralizes encoded payloads (base64, `eval()`, unicode escapes), escapes special LLM tokens,
   and prefixes role markers with `[DATA]`
3. **Leak detection** — scans for API keys (OpenAI, Anthropic, AWS, GitHub, Stripe, Slack),
   PEM private keys, Bearer tokens, and high-entropy hex strings; redacts matches
4. **Policy evaluation** — regex rules for shell injection, SQL injection, path traversal, XSS,
   and encoded exploits; user-supplied custom patterns can trigger an immediate block

LLM responses are also scanned for leaked secrets (catches exfiltration where the LLM was
tricked into outputting credentials from tool outputs).

#### Configuration

Add a `[safety]` section to `corre.toml` to tune the behavior:

```toml
[safety]
enabled = true                    # toggle the entire layer (default: true)
max_output_bytes = 100000         # truncate MCP outputs exceeding this size
sanitize_injections = true        # detect and neutralize injection phrases
detect_leaks = true               # scan for leaked API keys and credentials
boundary_wrap = true              # wrap tool outputs in XML delimiters
high_severity_action = "sanitize" # action for high-severity policy hits
custom_block_patterns = []        # additional regex patterns that trigger a block
require_sandbox = false           # require Landlock sandboxing (fail if unavailable)
```

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `true` | Master switch. Set to `false` to disable all safety processing. |
| `max_output_bytes` | `100000` | MCP tool outputs larger than this are truncated with a `[TRUNCATED]` marker. |
| `sanitize_injections` | `true` | Run Aho-Corasick + regex injection detection on tool outputs. |
| `detect_leaks` | `true` | Scan tool outputs and LLM responses for leaked credentials. |
| `boundary_wrap` | `true` | Wrap tool outputs in XML `<tool_output>` delimiters. |
| `high_severity_action` | `"sanitize"` | What to do when a high-severity policy rule matches: `"warn"` (log only), `"sanitize"` (redact the match), or `"block"` (replace the entire output). |
| `custom_block_patterns` | `[]` | List of regex patterns. Any match triggers an immediate block regardless of severity. |
| `require_sandbox` | `false` | If `true`, abort plugin/MCP execution when Landlock sandboxing is unavailable instead of falling back to unsandboxed execution. |

To disable safety entirely:

```toml
[safety]
enabled = false
```

Since all fields have defaults, omitting the `[safety]` section entirely is equivalent to
running with safety enabled at default settings.

### Configuration (`~/.local/share/corre/corre.toml`)

```toml
[general]
data_dir = "~/.local/share/corre"    # tilde-expanded at runtime
log_level = "info"                   # or RUST_LOG env var

[llm]
provider = "openai-compatible"
base_url = "https://api.venice.ai/api/v1"
model = "nvidia-nemotron-3-nano-30b-a3b"
api_key = "${VENICE_API_KEY}"        # ${VAR} syntax — resolved from env at runtime
temperature = 0.3
max_concurrent = 10                  # parallel LLM requests; tune to your provider's rate limit

# Provider-specific parameters passed through to the API request body:
# [llm.extra_body]
# stream = false
# reasoning_effort = "minimal"
# [llm.extra_body.venice_parameters]
# include_venice_system_prompt = false

[news]
bind = "127.0.0.1:5510"
title = "Corre News"
# editor_token = "your-secret"      # set to enable /settings/topics page

[registry]
url = "http://localhost:5580"       # registry API endpoint
cache_ttl_secs = 3600               # how long to cache registry responses
docker_registry = "ghcr.io/tree-corre"  # Docker image prefix for app containers

[[apps]]
name = "daily-brief"
description = "Researches topics and produces a daily news briefing"
schedule = "0 0 5 * * *"
mcp_servers = ["brave-search"]
config_path = "config/topics.yml"
enabled = true

# Per-app LLM overrides (model selection and generation params only):
# [apps.llm]
# model = "gpt-4o"
# temperature = 0.7
# max_concurrent = 5
```

MCP servers are configured as individual files under `config/mcp/` (see the MCP server pool
section above). To add a new app, either install a plugin through the dashboard or append
an `[[apps]]` entry referencing the MCP servers it needs.

## License

[MPL-2.0](LICENSE.md)
