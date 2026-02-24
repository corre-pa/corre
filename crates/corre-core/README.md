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
  |-- corre-capabilities --> corre-core, corre-mcp, corre-llm
```

## Modules

| Module | Purpose |
|--------|---------|
| `capability` | Core traits (`Capability`, `McpCaller`, `LlmProvider`), `CapabilityContext`, and `ProgressTracker` |
| `config` | Full `corre.toml` deserialization, per-MCP file configs, and env-var interpolation |
| `plugin` | Plugin discovery and manifest loading for subprocess-backed capabilities |
| `publish` | Publishing types: `Edition` > `Section` > `Article`, plus HTML sanitization |
| `scheduler` | Thin `Scheduler` wrapper around `tokio-cron-scheduler` |
| `secret` | `${VAR}` interpolation for config files |
| `tracker` | `ExecutionTracker` and `SystemMetrics` for the real-time dashboard |

## Key types and traits

### `Capability` trait

The unit of work. Each capability implements this trait:

```rust
#[async_trait]
pub trait Capability: Send + Sync {
    fn manifest(&self) -> &CapabilityManifest;
    async fn execute(&self, ctx: &CapabilityContext) -> anyhow::Result<CapabilityOutput>;
    async fn in_progress(&self) -> ProgressStatus { ProgressStatus::StillBusy(None) }
}
```

### `McpCaller` and `LlmProvider`

Thin async traits that decouple capabilities from `corre-mcp` and `corre-llm`. The safety
layer wraps both transparently.

### Publishing types

`Edition` > `Section` > `Article`. An edition is a dated snapshot of capability output.
`Edition::new` automatically selects the headline from the highest-scoring article.

### `CorreConfig`

Deserializes the full `corre.toml` file, including LLM settings, MCP server definitions,
per-capability overrides, safety config, and registry settings.

### `ProgressTracker`

Thread-safe progress tracker that capabilities update as they work. The orchestrator calls
`evaluate()` after a timeout to decide whether to wait, publish partial results, or kill.
