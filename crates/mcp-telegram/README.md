# mcp-telegram

An MCP server that exposes the Telegram Bot API as callable tools. Runs as a stdio child
process and is consumed by the Corre capability runner like any other MCP server.

## Role in the Corre project

Capabilities that need to send Telegram messages declare `mcp-telegram` in their `mcp_servers`
list. The binary has no dependency on any `corre-*` crate and can be used with any
MCP-compatible host.

## Tools

| Tool | Description |
|------|-------------|
| `send_message` | Posts a message to a Telegram chat (supports MarkdownV2) |
| `draft_message` | Returns a JSON representation without sending |

Both accept `chat_id` and `text` string parameters.

## Configuration

| Variable | Required | Description |
|----------|----------|-------------|
| `TELEGRAM_BOT_TOKEN` | Yes | Bot token from [@BotFather](https://t.me/BotFather) |

## Usage in `corre.toml`

```toml
[mcp.servers.telegram]
command = "mcp-telegram"
args = []
env = { TELEGRAM_BOT_TOKEN = "TELEGRAM_BOT_TOKEN" }
```

## Building

```sh
cargo build --release -p mcp-telegram
```
