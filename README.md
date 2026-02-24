# Corre

A personal AI task scheduler that runs modular *capabilities* on cron schedules and publishes
their output as a newspaper-style web interface called **CorreNews**.

## Goals

- **Privacy-centric**: All data stays local. Use whichever LLM provider you like, but Corre defaults to privacy-centric services that 
  use open-source models and don't log your sessions (Venice.ai, Brave Search, Kagi etc.).
- **Modular capabilities**: Each task (daily news brief, assistant, birthday reminders, ...) is a self-contained capability that can be 
  installed, configured, and removed independently.
- **MCP-native**: Capabilities interact with the outside world through
  [Model Context Protocol](https://modelcontextprotocol.io/) servers. We've curated a pool of MCP servers for web search, calendar 
  access, email sending, and more, and you can add your own.
- **Deterministic orchestration**: Scheduling is done with cron expressions, not open-ended LLM agent loops. We don't use LLMs to solve 
  problems that Unix solved 60 years ago.
- **Accessible anywhere**: CorreNews binds to `127.0.0.1` by default and is designed to sit behind a NAT-punching solution (Headscale, 
  WireGuard, Tor hidden service) so you can read your personal newspaper from any device without exposing it to the public internet.
- **Security-minded**: MCP servers and capabilities each run in their own sandbox. Capabilities have to provide a manifest of every MCP  
  server they use, and you _can_ configure fine-grained permissions for file access, network access and more. However, the capability 
  registry has pre-configured permissions to the absolute minimum so that you don't have to fiddle with manifests. 

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

Configuration lives in the data directory at `~/.local/share/corre/` (Linux) or
`~/Library/Application Support/corre/` (macOS). Running `corre setup` creates these files
automatically. You can override the config path with `corre -c /path/to/corre.toml`.

Edit `~/.local/share/corre/corre.toml` to point at your preferred LLM provider:

```toml
[llm]
provider = "openai-compatible"
base_url = "https://api.venice.ai/api/v1"   # or http://localhost:11434/v1 for Ollama
model = "zai-org-glm-4.7-flash"
api_key_env = "VENICE_API_KEY"               # name of the env var, not the key itself
```

Edit `~/.local/share/corre/config/topics.md` to choose your daily brief topics:

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

### 5. Start the full daemon

```sh
corre run
```

This starts the cron scheduler and the operator dashboard. Capabilities fire on their configured
schedules (the daily brief defaults to `0 0 5 * * *` -- 05:00 every day). The CorreNews web
server runs as a separate service (see `corre-news`).

### CLI reference

```
corre [OPTIONS] <COMMAND>

Commands:
  run       Start the full daemon (scheduler + dashboard)
  run-now   Run a single capability immediately and exit

Options:
  -c, --config <CONFIG>  Path to config file [default: ~/.local/share/corre/corre.toml]
  -h, --help             Print help
```

## Architecture

### Workspace layout

```
corre/                              # source repository
  templates/
    newspaper.html                  # full-page template (latest edition)
    edition.html                    # edition page with archive nav
  static/
    style.css                       # newspaper CSS
  crates/
    corre-core/                     # shared types and traits
    corre-mcp/                      # MCP server pool
    corre-llm/                      # LLM provider
    corre-news/                     # web server + archive + search
    corre-capabilities/             # capability implementations
    corre-safety/                   # prompt injection defense middleware
    corre-cli/                      # binary entry point

~/.local/share/corre/               # runtime data directory
  corre.toml                        # main config file
  config/
    topics.yml                      # user-editable per-capability config
  editions/
    YYYY-MM-DD/edition.json         # archived editions
  search_index/                     # Tantivy full-text search index
  .env                              # API keys (created by `corre setup`)
```

### Crate dependency graph

```
corre-cli
  |-- corre-core
  |-- corre-mcp        --> corre-core
  |-- corre-llm        --> corre-core
  |-- corre-safety     --> corre-core
  |-- corre-capabilities --> corre-core, corre-mcp, corre-llm
  |-- corre-dashboard  --> corre-core, corre-sdk, corre-registry

corre-news (standalone)
  |-- corre-core
  |-- corre-sdk
  |-- daily-brief      (Edition type)
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
3. Runs the capability with a 10-minute timeout
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

### Safety layer (`corre-safety`)

Corre fetches external content (web search results) via MCP servers and feeds it to LLMs for
scoring and summarization. A malicious web page could embed prompt injection phrases in its
metadata — "ignore previous instructions", special tokens like `<|system|>`, or encoded
payloads — and the LLM might comply. The safety layer sits between MCP tool outputs and LLM
prompts to neutralize these attacks.

Safety is **enabled by default**. It wraps `McpCaller` and `LlmProvider` transparently, so
capabilities require no code changes.

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
model = "zai-org-glm-4.7-flash"
api_key_env = "VENICE_API_KEY"       # env var name, never the actual key
temperature = 0.3

[news]
bind = "127.0.0.1:5510"
title = "CorreNews"

[mcp.servers.brave-search]
command = "npx"
args = ["-y", "@brave/brave-search-mcp-server"]
env = { BRAVE_API_KEY = "BRAVE_API_KEY" }   # env var name, same pattern as api_key_env

[[capabilities]]
name = "daily-brief"
schedule = "0 0 5 * * *"
mcp_servers = ["brave-search"]
config_path = "config/topics.md"
enabled = true
```

To add a new MCP server, append a `[mcp.servers.<name>]` block. To add a new capability, append
a `[[capabilities]]` entry referencing the MCP servers it needs.

## License

MIT
