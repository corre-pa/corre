# corre-mcp

MCP server pool management for Corre. Starts, caches, and shuts down MCP server child processes
on behalf of apps.

## Role in the Corre project

Apps interact with external services through MCP servers. `corre-mcp` manages the
server lifecycle: servers are spawned lazily on first use, cached for the duration of an
app run, and shut down afterward. `McpPool` implements the `McpCaller` trait from
`corre-core`, so the rest of the system depends only on that abstraction.

## Key types

### `McpPool`

A lazily-started, connection-caching pool of MCP server child processes. Each server is spawned
via `rmcp`'s `TokioChildProcess` transport and kept alive until `shutdown` is called.

### `McpServerDef`

A runtime-ready server description built from `McpServerConfig`. Environment variables are
resolved to their actual values at construction time.

## Modules

| Module | Purpose |
|--------|---------|
| `pool` | `McpPool` -- server lifecycle and `McpCaller` implementation |
| `server_def` | `McpServerDef` -- config-to-runtime bridge |
