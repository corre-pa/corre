# corre-sdk

The library for writing Corre app plugins. Provides the CCPP v1 (Corre Capability Plugin
Protocol) types, the `AppClient` async helper, shared output types, and utility functions.

## Role in the Corre project

Plugin authors depend on `corre-sdk` (and only `corre-sdk`) to build apps that run as
external subprocess binaries. The host (`corre-plugin::SubprocessApp`) speaks the
same CCPP v1 protocol over stdin/stdout.

Internal crates (`corre-core`, `corre-plugin`) also re-export types from `corre-sdk` to
avoid duplication.

## Plugin directory layout

```
~/.local/share/corre/plugins/my-plugin/
  manifest.toml       # PluginManifest (name, version, schedule, MCP servers)
  bin/app              # executable binary
  static/              # optional static assets served at /plugin/my-plugin/static/
```

## Key types

| Type | Module | Purpose |
|------|--------|---------|
| `AppOutput` | `types` | Top-level output: sections, articles, content type |
| `AppManifest` | `types` | Name, description, schedule, MCP server requirements |
| `AppClient` | `client` | Async helper for reading host requests and sending responses |
| `LlmRequest` / `LlmResponse` | `llm` | Types for the `llm/complete` CCPP method |
| `PluginManifest` | `manifest` | Serde types for `manifest.toml` |
| `Message` / `Request` / `Response` | `protocol` | CCPP v1 JSON-RPC 2.0 wire types |

## Quick start

```rust
use corre_sdk::client::AppClient;
use corre_sdk::types::AppOutput;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = AppClient::from_stdio().await?;
    let init = client.read_initialize().await?;

    // Use client.call_mcp_tool() and client.llm_complete() ...

    let output = AppOutput { /* ... */ };
    client.send_result(output).await?;
    Ok(())
}
```

## Modules

| Module | Purpose |
|--------|---------|
| `client` | `AppClient` async helper for host interaction |
| `codec` | Newline-delimited JSON codec for the transport layer |
| `html` | HTML sanitization helpers (`sanitize_html`, `sanitize_custom_html`) |
| `llm` | LLM request/response types for `llm/complete` |
| `manifest` | `PluginManifest` serde types |
| `protocol` | CCPP v1 JSON-RPC 2.0 message types |
| `tools` | Search result parsing, JSON extraction, freshness mapping, error helpers |
| `types` | Core output types (`AppOutput`, `Section`, `Article`, etc.) |
