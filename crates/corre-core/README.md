# corre-core

The foundational shared-types crate for the Corre project. It defines every trait, type, and
abstraction that the rest of the workspace depends on. No other Corre crate is a dependency of
`corre-core`; all data flows from this crate outward.

## Role in the Corre workspace

```
corre-cli
  |-- corre-core              <-- this crate
  |-- corre-mcp        --> corre-core
  |-- corre-llm        --> corre-core
  |-- corre-news       --> corre-core
  |-- corre-safety     --> corre-core
  |-- corre-plugin      --> corre-core, corre-mcp, corre-llm
```

## Modules

| Module | Purpose |
|--------|---------|
| `app` | Core traits (`App`, `McpCaller`, `LlmProvider`), `AppContext`, and `ProgressTracker` |
| `config` | Full `corre.toml` deserialization, per-MCP file configs, and env-var interpolation |
| `plugin` | Plugin discovery and manifest loading for subprocess-backed apps |
| `publish` | Publishing types: `Edition` > `Section` > `Article`, plus HTML sanitization  |
| `scheduler` | Thin `Scheduler` wrapper around `tokio-cron-scheduler` |
| `secret` | `${VAR}` interpolation for config files |
| `tracker` | `ExecutionTracker` and `SystemMetrics` for the real-time dashboard |

## Key types and traits

### `App` trait

The unit of work. Each app implements this trait:

```rust
#[async_trait]
pub trait App: Send + Sync {
    fn manifest(&self) -> &AppManifest;
    async fn execute(&self, ctx: &AppContext) -> anyhow::Result<AppOutput>;
    async fn in_progress(&self) -> ProgressStatus { ProgressStatus::StillBusy(None) }
}
```

### `McpCaller` and `LlmProvider`

Thin async traits that decouple apps from `corre-mcp` and `corre-llm`. The safety
layer wraps both transparently.

### Publishing types

`Edition` > `Section` > `Article`. An edition is a dated snapshot of app output.
`Edition::new` automatically selects the headline from the highest-scoring article.

### `CorreConfig`

Deserializes the full `corre.toml` file, including LLM settings, MCP server definitions,
per-app overrides, safety config, and registry settings.

### `ProgressTracker`

Thread-safe progress tracker that apps update as they work. The orchestrator calls
`evaluate()` after a timeout to decide whether to wait, publish partial results, or kill.
