//! MCP server binary for the Telegram Bot API.
//!
//! Exposes `send_message` and `draft_message` tools over the MCP stdio transport.
//! Reads `TELEGRAM_BOT_TOKEN` from the environment at call time.

use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool};

/// Telegram Bot API MCP server.
///
/// Sends messages via Telegram Bot API. Configured through environment variables:
/// - `TELEGRAM_BOT_TOKEN`: Telegram bot token from @BotFather
#[derive(Debug, Clone, Default)]
struct TelegramServer;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SendMessageParams {
    #[schemars(description = "Telegram chat ID to send the message to")]
    chat_id: String,
    #[schemars(description = "Message text (supports Telegram MarkdownV2)")]
    text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct DraftMessageParams {
    #[schemars(description = "Telegram chat ID")]
    chat_id: String,
    #[schemars(description = "Message text")]
    text: String,
}

#[tool(tool_box)]
impl TelegramServer {
    #[tool(description = "Send a message via Telegram Bot API")]
    async fn send_message(&self, #[tool(aggr)] params: SendMessageParams) -> Result<String, String> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").map_err(|_| "TELEGRAM_BOT_TOKEN not set")?;
        let url = format!("https://api.telegram.org/bot{token}/sendMessage");

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": params.chat_id,
                "text": params.text,
            }))
            .send()
            .await
            .map_err(|e| format!("Telegram API request failed: {e}"))?;

        if response.status().is_success() {
            Ok(format!("Message sent to chat {}", params.chat_id))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(format!("Telegram API error: {body}"))
        }
    }

    #[tool(description = "Draft a Telegram message (returns content without sending)")]
    fn draft_message(&self, #[tool(aggr)] params: DraftMessageParams) -> String {
        serde_json::json!({
            "status": "drafted",
            "chat_id": params.chat_id,
            "text": params.text,
        })
        .to_string()
    }
}

#[tool(tool_box)]
impl ServerHandler for TelegramServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Telegram Bot API server for sending messages".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(false).without_time().with_target(false).init();
    tracing::info!("Starting mcp-telegram server");

    let server = TelegramServer;
    let transport = rmcp::transport::io::stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
