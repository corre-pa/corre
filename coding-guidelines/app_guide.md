# Writing Corre Apps

A guide for AI (and human) agents building new Corre apps in Rust.

## Architecture overview

An app is a **standalone Rust binary** that communicates with the Corre host over
stdin/stdout using **CCPP** (Corre Capability Plugin Protocol), a JSON-RPC 2.0 protocol.
Apps never link against `corre-core` — they depend only on `corre-sdk`.

```
┌──────────────┐  stdin/stdout ┌─────────────┐
│  Corre Host  │◄────CCPP─────►│     App     │
│  (corre-cli) │   JSON-RPC    │   binary    │
└──────┬───────┘               └─────────────┘
       │                        depends on:
       │ manages                  corre-sdk
       ├── MCP server pool
       ├── LLM provider
       ├── Safety layer
       └── Sandbox (Landlock)
```

The host spawns the app binary as a child process, sends an `initialize` request with
context parameters, and the binary does its work by calling back into the host for MCP tools
and LLM completions. When done, it sends an `app/result` notification and exits.

## Crate layout

Every app lives in its own crate and produces both a library (for shared
types) and a binary (the app itself):

```
apps/my-app/
├── Cargo.toml
├── manifest.toml       # plugin metadata, permissions, config schema
└── src/
    ├── lib.rs          # shared types (used by this binary and any consumer)
    └── main.rs         # standalone binary — the app entry point
```

### Cargo.toml

```toml
[package]
name = "my-app"
version.workspace = true
edition.workspace = true
description = "Short description of what this app does"

[lib]
path = "src/lib.rs"

[[bin]]
name = "my-app"
path = "src/main.rs"

[dependencies]
corre-sdk = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
chrono = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
# Add others as needed (futures, serde_yaml_ng, etc.)
```

**Key rule:** The only Corre dependency is `corre-sdk`. Never depend on `corre-core`,
`corre-mcp`, `corre-llm`, or any other host crate.

## manifest.toml

The manifest declares the plugin's identity, permissions, default configuration, and any
external dependencies. The host reads this when the plugin is installed.

```toml
[plugin]
name = "my-app"
version = "0.1.0"
description = "What this app does"
protocol_version = "1.0"
binary_name = "my-app"
content_type = "newspaper"          # or "custom" for plugin-rendered HTML

[plugin.defaults]
schedule = "0 30 8 * * *"          # 6-field cron: sec min hour day month weekday
config_path = "config/my-cap.yml"  # relative to data_dir

# Optional: define a schema so the dashboard can generate an edit form
[plugin.defaults.config_schema]
root_key = "my-app"
format = "yaml"

[[plugin.defaults.config_schema.fields]]
key = "some_setting"
type = "text"
label = "A setting"
default = "hello"

[plugin.permissions]
mcp_servers = ["brave-search"]      # which MCP servers this app may call
llm_access = true                   # whether it can make LLM calls
max_concurrent_llm = 10             # concurrency limit for LLM calls

[[plugin.permissions.outputs]]
output_type = "filesystem"
target = "editions/{date}/edition.json"  # {date}, {data_dir}, {config_dir} are expanded
content_type = "application/json"

[plugin.permissions.sandbox]
network = ["api.venice.ai:443"]     # host:port allowlist
# r/w to {data_dir}/{app_name}/ is granted automatically

# Declare MCP server dependencies (installed if missing)
[mcp_dependencies.brave-search]
command = "npx"
args = ["-y", "@brave/brave-search-mcp-server"]
env = { BRAVE_API_KEY = "$BRAVE_API_KEY" }
```

### Permission fields reference

| Field                      | Type       | Description                                                                |
|----------------------------|------------|----------------------------------------------------------------------------|
| `mcp_servers`              | `[String]` | MCP server names this app may call                                         |
| `llm_access`               | `bool`     | Whether LLM calls are allowed (default: true)                              |
| `max_concurrent_llm`       | `usize`    | Max parallel LLM calls (default: 10)                                       |
| `outputs`                  | `[Output]` | Permitted output destinations                                              |
| `sandbox.network`          | `[String]` | Allowed `host:port` endpoints                                              |
| `sandbox.filesystem_read`  | `[String]` | Extra read paths (templates: `{data_dir}`, `{config_dir}`, `{plugin_dir}`) |
| `sandbox.filesystem_write` | `[String]` | Extra write paths                                                          |

### Content types

- **`newspaper`** (default) — output uses `Section`/`Article` types, rendered by CorreNews
- **`custom`** — output includes `CustomContent` with plugin-provided HTML/CSS/JS

### Execution modes

- **`oneshot`** (default) — run once, produce output, exit
- **`daemon`** — long-running, no timeout, restarted on crash, continuous output

### Config schema field types

| Type        | Description                                              |
|-------------|----------------------------------------------------------|
| `text`      | Single-line text input                                   |
| `textarea`  | Multi-line text input                                    |
| `select`    | Dropdown; requires `options = ["a", "b", "c"]`           |
| `text-list` | Comma-separated values stored as a YAML list             |
| `list`      | Repeatable group of sub-fields; requires nested `fields` |

### Optional sections

```toml
# Bundle a Docker service (e.g. a web UI)
[[plugin.services]]
name = "my-web-ui"
description = "Web dashboard for my-app"
image = "{docker_registry}/my-web-ui:latest"
ports = ["8080:8080"]
volumes = ["{data_dir}:/data:ro"]
optional = true

# Dashboard links
[[plugin.links]]
label = "Open Dashboard"
url = "http://{service:my-web-ui}:8080"
```

## The CCPP protocol lifecycle

```
Host                          App
  │                               │
  │──── initialize ──────────────►│  (Request: config_dir, config_path, mcp_servers, ...)
  │◄─── initialize response ──────│  (protocol_version, apps)
  │                               │
  │◄─── progress ─────────────────│  (phase, percent, message)
  │◄─── mcp/callTool ─────────────│  (server_name, tool_name, arguments)
  │──── mcp/callTool response ───►│
  │◄─── llm/complete ─────────────│  (messages, temperature, ...)
  │──── llm/complete response ───►│
  │◄─── output/write ─────────────│  (path, data, content_type)
  │──── output/write response ───►│
  │◄─── app/result ────────────────│  (AppOutput)
  │                               │
  │──── shutdown ────────────────►│  (optional, host may send at any time)
  │                               │
```

- `mcp/callTool`, `llm/complete`, `output/write` are **request-response** (blocking RPC)
- `progress`, `log`, `app/result`, `app/error` are **notifications** (fire-and-forget)
- Multiple RPC calls can be in-flight simultaneously (the SDK handles demultiplexing)

## src/main.rs — the binary skeleton

Every app binary follows the same structure:

```rust
use anyhow::Context as _;
use corre_sdk::types::{Article, AppOutput, Section, Source};
use corre_sdk::{AppClient, LlmMessage, LlmRequest, LlmRole};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ERROR my-app failed: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    // 1. Connect to the host
    let client = Arc::new(AppClient::from_stdio());
    let params = client.accept_initialize().await?;

    // 2. Set up tracing (logs to stderr + optional daily-rotating file)
    let _guard = corre_sdk::init_tracing(
        &params.app_name,
        params.log_dir.as_deref(),
        params.log_level.as_deref(),
    );

    // 3. Read context from params
    let config_dir = std::path::PathBuf::from(&params.config_dir);
    let max_concurrent_llm = params.max_concurrent_llm;

    // 4. Load user config (if applicable)
    if let Some(ref config_path) = params.config_path {
        let full_path = config_dir.join(config_path);
        let content = std::fs::read_to_string(&full_path)
            .with_context(|| format!("failed to read config {}", full_path.display()))?;
        // parse content...
    }

    // 5. Do the work (see pipeline patterns below)
    client.report_progress("starting", Some(0), None).await?;

    // ... your pipeline ...

    // 6. Build and send the result
    let output = AppOutput {
        app_name: "my-app".into(),
        produced_at: chrono::Utc::now(),
        sections: vec![],  // fill with your sections
        content_type: Default::default(),
        custom_content: None,
    };

    client.send_result(output).await?;
    Ok(())
}
```

### InitializeParams fields

The `accept_initialize()` call returns these fields from the host:

| Field                | Type             | Description                                 |
|----------------------|------------------|---------------------------------------------|
| `app_name`           | `String`         | Name from manifest                          |
| `config_dir`         | `String`         | Absolute path to data directory             |
| `config_path`        | `Option<String>` | Relative path to user config file           |
| `mcp_servers`        | `Vec<String>`    | MCP servers available to this app           |
| `max_concurrent_llm` | `usize`          | Max parallel LLM calls (default: 10)        |
| `timeout_secs`       | `u64`            | Execution timeout (default: 600)            |
| `log_dir`            | `Option<String>` | Path for log files                          |
| `log_level`          | `Option<String>` | Log verbosity                               |
| `seen_urls`          | `Vec<String>`    | URLs from previous runs (for deduplication) |

## AppClient API reference

All methods are `async` and return `anyhow::Result`.

### Host communication

| Method                                       | Description                                         |
|----------------------------------------------|-----------------------------------------------------|
| `accept_initialize()`                        | Wait for host handshake, returns `InitializeParams` |
| `report_progress(phase, percent, message)`   | Send progress update (notification)                 |
| `log(level, message)`                        | Send log entry to host dashboard                    |
| `send_result(output)`                        | Send final `AppOutput` (notification)               |
| `send_error(message, phase, partial_output)` | Report failure with optional partial results        |
| `read_message()`                             | Read next host message (shutdown, cancel)           |

### MCP tools

| Method                                    | Description                                        |
|-------------------------------------------|----------------------------------------------------|
| `call_tool(server_name, tool_name, args)` | Call an MCP tool, returns `serde_json::Value`      |
| `list_tools(server_name)`                 | List tools on an MCP server, returns `Vec<String>` |

### LLM

| Method                  | Description                                 |
|-------------------------|---------------------------------------------|
| `llm_complete(request)` | Send chat completion, returns `LlmResponse` |

### Output

| Method                                 | Description                          |
|----------------------------------------|--------------------------------------|
| `write_file(path, data, content_type)` | Write file to permitted path on host |
| `stream_text(chunk, is_final)`         | Stream text output (notification)    |
| `post_rest(url, body, content_type)`   | POST to permitted REST endpoint      |
| `fire_webhook(url, body)`              | Fire a webhook                       |

## Core types


### AppOutput

```rust
pub struct AppOutput {
    pub app_name: String,
    pub produced_at: DateTime<Utc>,
    pub sections: Vec<Section>,
    pub content_type: ContentType,            // Newspaper (default) or Custom
    pub custom_content: Option<CustomContent>, // only for content_type = Custom
}
```

### LlmRequest

```rust
pub struct LlmRequest {
    pub messages: Vec<LlmMessage>,
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    pub json_mode: bool,
}

// Convenience constructor for simple system + user prompt:
LlmRequest::simple("You are a helpful assistant.", "Summarise this article.")
```

### LlmMessage

```rust
pub struct LlmMessage {
    pub role: LlmRole,    // System, User, or Assistant
    pub content: String,
}
```

## SDK utility functions

Import from `corre_sdk::tools` and `corre_sdk::html`:

| Function                          | Module  | Description                                            |
|-----------------------------------|---------|--------------------------------------------------------|
| `parse_search_results(value)`     | `tools` | Parse MCP search results into `Vec<SearchResultItem>`  |
| `extract_json(text)`              | `tools` | Extract JSON from LLM output (handles markdown fences) |
| `normalize_freshness(s)`          | `tools` | Map `1d`/`1w`/`1m` to Brave API format                 |
| `is_retryable_overload(err)`      | `tools` | Detect 429/503/rate-limit errors                       |
| `parse_context_length_limit(err)` | `tools` | Extract available tokens from context-length errors    |
| `sanitize_html(s)`                | `html`  | Strip XSS vectors, allow basic formatting tags         |
| `sanitize_url(s)`                 | `html`  | Allow only `http://` and `https://` URLs               |
| `sanitize_custom_html(s)`         | `html`  | Wider allowlist for custom content HTML                |

## CCPP error codes

Apps should handle these error codes when making RPC calls:

| Code   | Constant               | Meaning                   | Action                         |
|--------|------------------------|---------------------------|--------------------------------|
| -32020 | `LLM_RATE_LIMITED`     | Rate limited              | Retry with backoff             |
| -32021 | `LLM_TRUNCATED`        | Context too long          | Reduce `max_completion_tokens` |
| -32022 | `LLM_PROVIDER_ERROR`   | Provider error            | Retry or fail gracefully       |
| -32023 | `LLM_AUTH_FAILED`      | Bad credentials           | **Fatal** — abort              |
| -32024 | `LLM_PAYMENT_REQUIRED` | Payment needed            | **Fatal** — abort              |
| -32010 | `MCP_TOOL_ERROR`       | Tool returned error       | Log and continue               |
| -32011 | `MCP_SERVER_DENIED`    | Server not in manifest    | Fix manifest                   |
| -32030 | `SAFETY_BLOCKED`       | Blocked by safety layer   | Content was filtered; skip     |
| -32040 | `OUTPUT_DENIED`        | Output path not permitted | Fix manifest outputs           |

## Pipeline patterns

### Pattern 1: Parallel MCP calls with join_all

```rust
use futures::future::join_all;

let mut handles = Vec::new();
for query in & queries {
let client = client.clone();
let q = query.clone();
handles.push(async move {
let args = serde_json::json ! ({ "query": q });
client.call_tool("brave-search", "brave_web_search", args).await
});
}
let results = join_all(handles).await;
```

### Pattern 2: Parallel LLM calls with semaphore

```rust
use std::sync::Arc;
use tokio::sync::Semaphore;

let semaphore = Arc::new(Semaphore::new(max_concurrent_llm));

let mut handles = Vec::new();
for item in & items {
let sem = semaphore.clone();
let client = client.clone();
let item = item.clone();
handles.push(tokio::spawn(async move {
let _permit = sem.acquire().await.unwrap();
let request = LlmRequest::simple(
"You are a helpful assistant.",
format ! ("Summarise: {}", item.title),
);
client.llm_complete(request).await
}));
}

let results = join_all(handles).await;
```

### Pattern 3: LLM retry with backoff

```rust
use corre_sdk::tools::{extract_json, is_retryable_overload, parse_context_length_limit};

let mut request = initial_request;
let mut result = None;

for attempt in 0..3u64 {
match client.llm_complete(request.clone()).await {
Ok(resp) => {
let json_str = extract_json( & resp.content);
match serde_json::from_str::< MyOutput > (json_str) {
Ok(parsed) => { result = Some(parsed); break; }
Err(e) => tracing::warn ! ("JSON parse failed (attempt {}): {e}", attempt + 1),
}
}
Err(e) => {
let err = e.to_string();
if let Some(available) = parse_context_length_limit( & err) {
request.max_completion_tokens = Some(available);
} else if is_retryable_overload( & err) {
// exponential backoff: 5s, 10s, 20s
} else {
tracing::warn ! ("LLM failed (attempt {}): {err}", attempt + 1);
}
tokio::time::sleep(std::time::Duration::from_secs(5 < < attempt)).await;
}
}
}
```

### Pattern 4: Structured JSON from LLM

Ask the LLM to return structured JSON by describing the schema in the system prompt:

```rust
let request = LlmRequest {
messages: vec![
    LlmMessage {
        role: LlmRole::System,
        content: "Respond with ONLY a raw JSON array. No markdown fencing.\n\
                      Each element: {\"score\": <0.0-1.0>, \"summary\": \"<string>\"}".into(),
    },
    LlmMessage {
        role: LlmRole::User,
        content: format!("Score these items:\n{items_json}"),
    },
],
temperature: Some(0.1),  // low temp for deterministic structured output
max_completion_tokens: None,
json_mode: false,
};
```

Always use `extract_json()` on the response to strip any markdown fences the LLM may add.

### Pattern 5: Progress reporting

Report progress at each major pipeline stage so the dashboard shows what's happening:

```rust
client.report_progress("loading_config", Some(5), None).await?;
// ... load config ...

client.report_progress("searching", Some(20), Some("12 queries")).await?;
// ... run searches ...

client.report_progress("scoring", Some(50), Some("48 results to score")).await?;
// ... score results ...

client.report_progress("summarizing", Some(75), None).await?;
// ... summarize ...

client.report_progress("writing_output", Some(95), None).await?;
// ... write files ...
```

### Pattern 6: Sanitize all external content

Always sanitize content from MCP tools before including it in output:

```rust
use corre_sdk::html::{sanitize_html, sanitize_url};

let article = Article {
title: item.title.clone(),
summary: sanitize_html( & raw_summary),
body: sanitize_html( & raw_body),
sources: vec![Source {
    title: item.title.clone(),
    url: sanitize_url(&item.url),
}],
score: 0.8,
};
```

### Pattern 7: Writing output files

Use `client.write_file()` for persistent output. The path is relative to `data_dir` and must
match a declared `outputs` entry in `manifest.toml`:

```rust
let output_path = format!("editions/{}/edition.json", today.format("%Y-%m-%d"));
let json = serde_json::to_string_pretty( & edition) ?;
client.write_file( & output_path, & json, Some("application/json")).await?;
```

### Pattern 8: Custom HTML content

For apps that render their own HTML instead of articles:

```rust
use corre_sdk::types::{AppOutput, ContentType, CustomContent};

let output = AppOutput {
app_name: "my-dashboard".into(),
produced_at: chrono::Utc::now(),
sections: vec![],
content_type: ContentType::Custom,
custom_content: Some(CustomContent {
html: "<div class=\"dashboard\">...</div>".into(),
css: Some(".dashboard { display: grid; }".into()),
js: None,
searchable_text: Some("plain text for search indexing".into()),
}),
};
```

## Checklist for new apps

1. **Create crate** under `apps/` with lib + bin targets
2. **Write `manifest.toml`** with plugin metadata, permissions, and config schema
3. **Depend only on `corre-sdk`** (plus general-purpose crates like serde, tokio, etc.)
4. **In `main.rs`**: connect via `AppClient::from_stdio()`, call `accept_initialize()`
5. **Initialize tracing** with `corre_sdk::init_tracing()`
6. **Load config** from `params.config_path` if your app is user-configurable
7. **Report progress** at each pipeline stage
8. **Sanitize** all external content with `sanitize_html()` / `sanitize_url()`
9. **Handle LLM errors** with retry + backoff (rate limits, context length)
10. **Use `extract_json()`** when parsing structured JSON from LLM responses
11. **Write output files** via `client.write_file()` (not direct filesystem access)
12. **Send result** via `client.send_result()` as the final step
13. **Write tests** for config parsing, data transforms, and output construction
14. **Declare all MCP servers** in the manifest's `permissions.mcp_servers`
15. **Declare all output paths** in the manifest's `permissions.outputs`

## Reference: daily-brief pipeline

The `daily-brief` app is the reference implementation. Its 8-step pipeline:

1. **Parse config** — load `topics.yml`, extract sections and search sources
2. **Search** — parallel Brave web + news searches via `call_tool("brave-search", ...)`
3. **Deduplicate** — by URL within each source + cross-edition dedup from `seen_urls`
4. **Score + summarize** — parallel LLM calls with semaphore, structured JSON output
5. **Filter** — drop scores <= 0.2, keep top 10 per source
6. **Group** — collect articles into sections preserving YAML ordering
7. **Build edition** — create Edition, generate tagline via LLM
8. **Persist** — write `edition.json` via `write_file()`, send `AppOutput`

Study `apps/daily-brief/src/main.rs` for the complete working implementation.
