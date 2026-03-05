# corre-host

Subprocess app host and plugin registry for Corre. Spawns app plugin binaries,
brokers their CCPP (Corre Capability Plugin Protocol) JSON-RPC requests, and manages the
registry of all apps (built-in and plugin-based).

## Role in the Corre project

`corre-host` sits between `corre-core` (traits) and `corre-cli` (orchestration). It provides
the generic infrastructure for running any app as an isolated subprocess, regardless of
how it was implemented. The CLI registers apps through the `AppRegistry` and
calls `execute()` on each one — the registry dispatches to the appropriate implementation.

## Key types

### `AppRegistry`

Maps app names to `Arc<dyn App>` trait objects. Built from config entries and
discovered plugins:

```rust
let registry = AppRegistry::from_config(&configs, &plugins, &data_dir, "info");
let cap = registry.get("daily-brief").unwrap();
let output = cap.execute(&ctx).await?;
```

### `SubprocessApp`

Implements `App` for external plugin binaries. Handles the full CCPP lifecycle:

1. Spawns the plugin binary with optional Landlock + seccomp sandbox
2. Sends `initialize` request with config paths, MCP servers, and concurrency limits
3. Runs a concurrent message loop dispatching `mcp/callTool`, `llm/complete`, and `output/write`
4. Collects `progress` and `log` notifications for the dashboard
5. Extracts the final `app/result` or `app/error`

Multiple RPC requests can be in-flight simultaneously — they are dispatched concurrently via
`FuturesUnordered` and responses are routed back by request ID.

## CCPP protocol lifecycle

```
Host                           Plugin
  │                               │
  │──── initialize ──────────────►│
  │◄─── initialize response ─────│
  │                               │
  │◄─── mcp/callTool ───────────│  (concurrent RPC)
  │──── mcp/callTool response ──►│
  │◄─── llm/complete ───────────│
  │──── llm/complete response ──►│
  │◄─── output/write ───────────│
  │──── output/write response ──►│
  │                               │
  │◄─── progress ────────────────│  (notifications)
  │◄─── log ─────────────────────│
  │◄─── app/result ─────────────│
  │                               │
  │──── shutdown ───────────────►│  (optional)
```

## Security

- **Sandboxing**: plugin binaries can be restricted with Landlock filesystem rules and seccomp
  network filtering via `LandlockSandbox` from `corre-core`.
- **Output validation**: file write paths are matched against the plugin's declared
  `OutputDeclaration` entries using glob patterns.
- **Secret redaction**: API keys, tokens, and passwords are replaced with `[REDACTED]` in
  debug logs.
- **Stderr capture**: plugin stderr is captured in a background task, parsed for log levels,
  and forwarded to the host's structured logging.

## Error classification

LLM errors are classified by HTTP status code and mapped to CCPP error codes:

| HTTP status | CCPP code | Meaning | Fatal? |
|-------------|-----------|---------|--------|
| 429 | -32020 | Rate limited | No (retry) |
| 401, 403 | -32023 | Auth failed | Yes |
| 402 | -32024 | Payment required | Yes |
| Other | -32022 | Provider error | No |

## Modules

| Module | Purpose |
|--------|---------|
| `registry` | `AppRegistry` — app name to trait-object mapping |
| `subprocess` | `SubprocessApp` — CCPP host, message loop, RPC dispatch |
