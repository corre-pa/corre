# corre-capabilities

Built-in capability implementations and the capability registry for Corre.

## Role in the Corre project

This crate contains the concrete capability logic that produces CorreNews editions. It sits
near the top of the dependency graph, consuming `corre-core` (traits), `corre-mcp` (MCP calls),
`corre-llm` (LLM completions), and `corre-db` (contact database). The CLI wires everything
together and delegates execution to capabilities registered here.

## Capabilities

### Daily Research Brief (`daily_brief`)

A multi-step pipeline that:

1. Reads topics from `config/topics.yml`
2. Searches the web via the Brave Search MCP server
3. Deduplicates results by URL
4. Scores results for newsworthiness (LLM call)
5. Summarises the top stories (LLM call)
6. Emits a `CapabilityOutput` grouped by section

### Rolodex (`rolodex`)

Automated personal contact engagement. Checks the SQLite contact database for outreach
strategies that are due (birthday messages, news searches, profile scrapes, check-ins),
executes each strategy, and publishes the results.

## Key types

### `CapabilityRegistry`

Maps capability names to boxed `Capability` trait objects. Instantiates subprocess-backed
plugin capabilities from `DiscoveredPlugin` entries.

```rust
let registry = CapabilityRegistry::new(&config.capabilities, &plugins, &db_path);
let cap = registry.get("daily-brief").unwrap();
```

### `SubprocessCapability`

Runs an external plugin binary using the CCPP v1 protocol over stdin/stdout, allowing
third-party capabilities without recompiling Corre.

## Modules

| Module | Purpose |
|--------|---------|
| `daily_brief` | Daily Research Brief pipeline |
| `rolodex` | Contact engagement capability |
| `rolodex_import` | CSV/vCard/JSON import helpers for the contact database |
| `registry` | `CapabilityRegistry` construction and lookup |
| `subprocess` | CCPP v1 subprocess capability runner |
| `tools` | Re-exports shared utilities from `corre-sdk` |
