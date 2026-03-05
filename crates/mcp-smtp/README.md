# mcp-smtp

An MCP server that exposes SMTP email functionality as callable tools. Runs as a stdio child
process and is consumed by the Corre app runner like any other MCP server.

## Role in the Corre project

Apps that need to send email declare `mcp-smtp` in their `mcp_servers` list. The server
is started lazily by `corre-mcp`, lives for the duration of an app run, and is shut down
once complete.

The binary has no dependency on any `corre-*` crate and can be used with any MCP-compatible host.

## Tools

| Tool | Description |
|------|-------------|
| `send_email` | Builds and delivers a plain-text email via SMTP |
| `draft_email` | Returns the email as a JSON object without sending |

Both accept `to`, `subject`, and `body` string parameters.

## Configuration

All connection details come from environment variables:

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SMTP_HOST` | Yes | -- | SMTP relay hostname |
| `SMTP_PORT` | No | `587` | TCP port (STARTTLS) |
| `SMTP_USER` | Yes | -- | Authentication username |
| `SMTP_PASSWORD` | Yes | -- | Authentication password |
| `SMTP_FROM` | Yes | -- | Sender address |

## Usage in `corre.toml`

```toml
[mcp.servers.smtp]
command = "mcp-smtp"
args = []
env = { SMTP_HOST = "SMTP_HOST", SMTP_USER = "SMTP_USER", SMTP_PASSWORD = "SMTP_PASSWORD", SMTP_FROM = "SMTP_FROM" }
```

## Building

```sh
cargo build --release -p mcp-smtp
```
