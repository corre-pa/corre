# Corre

A personal AI task scheduler that runs modular *capabilities* on cron schedules and publishes
their output as a newspaper-style web interface called **CorreNews**.

## Goals

- **Privacy-first**: All data stays local. Corre never phones home or shares data with third
  parties unless you explicitly point it at an external API. Use whichever LLM provider you trust
  (Venice.ai, Ollama, OpenAI, etc.).
- **Modular capabilities**: Each task (daily news brief, stock portfolio review, fantasy sports
  assistant, birthday reminders, ...) is a self-contained capability that can be installed,
  configured, and removed independently.
- **MCP-native**: Capabilities interact with the outside world through
  [Model Context Protocol](https://modelcontextprotocol.io/) servers. Existing MCP servers work
  out of the box -- just add them to `corre.toml`.
- **Deterministic orchestration**: Scheduling is done with cron expressions, not open-ended LLM
  agent loops. LLM calls happen at well-defined steps (scoring, summarising) within a structured
  pipeline.
- **Newspaper output**: Results are compiled into dated editions and served on a local web server
  with a classic newspaper layout. Editions are archived as JSON and indexed for full-text search.
- **Accessible anywhere**: CorreNews binds to `127.0.0.1` by default and is designed to sit
  behind a NAT-punching solution (Headscale, WireGuard, Tor hidden service) so you can read your
  personal newspaper from any device without exposing it to the public internet.

### Included capability: Daily Research Brief

The first end-to-end capability ships with the MVP. Each morning it:

1. Reads topics from `config/topics.md`
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

Install the Fetch MCP server (Rust binary, no Node/Python needed):

```sh
cargo install mcp-server-fetch
```

### 1. Clone and build

```sh
git clone <repo-url> corre && cd corre
cargo build --release
```

The binary is at `target/release/corre`.

### 2. Set up API keys

Corre never stores secrets in config files. Instead, `corre.toml` references environment
variable *names*, and you set the actual values in your shell or a `.env` file.

```sh
# LLM provider (Venice.ai by default, or any OpenAI-compatible API)
export VENICE_API_KEY="your-venice-api-key"

# Brave Search (used by the daily-brief capability)
export BRAVE_API_KEY="BRAVE_API_KEY"
```

### 3. Configure

Edit `corre.toml` to point at your preferred LLM provider:

```toml
[llm]
provider = "openai-compatible"
base_url = "https://api.venice.ai/api/v1"   # or http://localhost:11434/v1 for Ollama
model = "llama-3.3-70b"
api_key_env = "VENICE_API_KEY"               # name of the env var, not the key itself
```

Edit `config/topics.md` to choose your daily brief topics:

```markdown
## Technology
- Rust programming language news
- AI and machine learning developments

## World News
- Geopolitics and international relations
```

### 4. Run a capability manually

```sh
# One-shot run -- executes the daily brief immediately and exits
corre run-now daily-brief
```

The edition is written to `~/.local/share/corre/editions/YYYY-MM-DD/edition.json`.

### 5. Start the web server

```sh
corre serve
```

Open <http://127.0.0.1:3200> to view CorreNews.

### 6. Start the full daemon

```sh
corre run
```

This starts both the cron scheduler and the web server. Capabilities fire on their configured
schedules (the daily brief defaults to `0 5 * * *` -- 05:00 every day).

### CLI reference

```
corre [OPTIONS] <COMMAND>

Commands:
  run       Start the full daemon (scheduler + web server)
  run-now   Run a single capability immediately and exit
  serve     Start only the web server

Options:
  -c, --config <CONFIG>  Path to config file [default: corre.toml]
  -h, --help             Print help
```

## Architecture

### Workspace layout

```
corre/
  corre.toml                      # main config file
  config/
    topics.md                     # user-editable per-capability config
  templates/
    newspaper.html                # full-page template (latest edition)
    edition.html                  # edition page with archive nav
  static/
    style.css                     # newspaper CSS
  crates/
    corre-core/                   # shared types and traits
    corre-mcp/                    # MCP server pool
    corre-llm/                    # LLM provider
    corre-news/                   # web server + archive + search
    corre-capabilities/           # capability implementations
    corre-cli/                    # binary entry point
```

### Crate dependency graph

```
corre-cli
  |-- corre-core
  |-- corre-mcp        --> corre-core
  |-- corre-llm        --> corre-core
  |-- corre-news       --> corre-core
  |-- corre-capabilities --> corre-core, corre-mcp, corre-llm
```

`corre-core` sits at the bottom with zero internal dependencies. It defines the trait
abstractions that the other crates implement or consume.

### Key abstractions (`corre-core`)

**`Capability`** -- the unit of work. Each capability declares a manifest (name, cron schedule,
required MCP servers) and an `execute` method that receives a context and returns articles.

```rust
#[async_trait]
pub trait Capability: Send + Sync {
    fn manifest(&self) -> &CapabilityManifest;
    async fn execute(&self, ctx: &CapabilityContext) -> anyhow::Result<CapabilityOutput>;
}
```

**`LlmProvider`** -- abstracts over any chat-completion API. The single implementation
(`OpenAiCompatProvider` in `corre-llm`) speaks the OpenAI wire format, which Venice.ai, Ollama,
and many others support.

**`McpCaller`** -- abstracts over MCP tool invocation, allowing capabilities to call MCP tools
without depending on `corre-mcp` directly.

**Publishing types** -- `Edition` > `Section` > `Article`. An edition is a dated collection of
sections, each containing scored articles with sources. The highest-scoring article title becomes
the edition headline.

### MCP server pool (`corre-mcp`)

MCP servers are defined in `corre.toml` under `[mcp.servers.*]`. They are started lazily as
stdio child processes on first use, cached for the duration of a capability run, and shut down
afterward. Environment variable references in the `env` table (e.g. `$BRAVE_API_KEY`) are
resolved at spawn time from the host environment.

### CorreNews web server (`corre-news`)

- **Axum** HTTP server with askama-rendered newspaper templates
- **Filesystem archive**: editions stored as dated JSON at
  `~/.local/share/corre/editions/YYYY-MM-DD/edition.json`
- **Tantivy** full-text search index over all archived articles, queryable at `GET /search?q=...`

Routes:

| Path | Description |
|------|-------------|
| `GET /` | Latest edition |
| `GET /edition/:date` | Specific edition with archive navigation |
| `GET /search?q=...&limit=N` | Full-text search (returns JSON) |
| `GET /static/*` | CSS and static assets |

### Scheduler (`corre-core` + `corre-cli`)

The scheduler wraps `tokio-cron-scheduler`. For each enabled capability in config, the CLI
registers an async callback that:

1. Builds an MCP pool with the capability's declared servers
2. Initializes the LLM provider
3. Runs the capability with a 5-minute timeout
4. Stores the resulting edition and updates the search index

Capabilities run in isolated `tokio` tasks. A panic or timeout in one capability does not affect
the scheduler or other capabilities.

### Daily Brief pipeline (`corre-capabilities`)

A deterministic, multi-step pipeline with LLM calls at specific points:

1. **Parse** `config/topics.md` into sections and search queries
2. **Search** each query via the `brave-search` MCP server
3. **Deduplicate** results by URL within each section
4. **Score** results for newsworthiness (LLM call, structured JSON output)
5. **Filter** to top 5 results per section above the score threshold
6. **Summarise** each top result in 2-3 paragraphs (LLM call)
7. **Emit** `CapabilityOutput` with articles grouped by section

### Configuration (`corre.toml`)

```toml
[general]
data_dir = "~/.local/share/corre"    # tilde-expanded at runtime
log_level = "info"                   # or RUST_LOG env var

[llm]
provider = "openai-compatible"
base_url = "https://api.venice.ai/api/v1"
model = "llama-3.3-70b"
api_key_env = "VENICE_API_KEY"       # env var name, never the actual key
temperature = 0.3
max_tokens = 4096

[news]
bind = "127.0.0.1:3200"
title = "CorreNews"

[mcp.servers.brave-search]
command = "npx"
args = ["-y", "@brave/brave-search-mcp-server"]
env = { BRAVE_API_KEY = "BRAVE_API_KEY" }   # env var name, same pattern as api_key_env

[[capabilities]]
name = "daily-brief"
schedule = "0 5 * * *"
mcp_servers = ["brave-search"]
config_path = "config/topics.md"
enabled = true
```

To add a new MCP server, append a `[mcp.servers.<name>]` block. To add a new capability, append
a `[[capabilities]]` entry referencing the MCP servers it needs.

## License

MIT
