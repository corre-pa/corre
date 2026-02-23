use std::sync::Arc;

use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool};
use wa_rs::Jid;
use wa_rs::bot::Bot;
use wa_rs::client::Client;
use wa_rs::store::SqliteStore;

/// WhatsApp messaging MCP server using wa-rs (pure Rust WhatsApp Web client).
///
/// Requires initial QR code pairing on first run. The session is persisted
/// to a SQLite database for subsequent runs.
///
/// Environment variables:
/// - `WHATSAPP_STORE_PATH`: Path to the WhatsApp session SQLite file (default: ~/.local/share/corre/whatsapp.db)
#[derive(Clone)]
struct WhatsAppServer {
    client: Arc<Client>,
}

impl std::fmt::Debug for WhatsAppServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhatsAppServer").finish()
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SendMessageParams {
    #[schemars(description = "Recipient phone number in E.164 format without '+' prefix (e.g. '14155552671')")]
    phone: String,
    #[schemars(description = "Message text to send")]
    text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct DraftMessageParams {
    #[schemars(description = "Recipient phone number in E.164 format without '+' prefix")]
    phone: String,
    #[schemars(description = "Message text")]
    text: String,
}

#[tool(tool_box)]
impl WhatsAppServer {
    #[tool(description = "Send a message via WhatsApp")]
    async fn send_message(&self, #[tool(aggr)] params: SendMessageParams) -> Result<String, String> {
        let phone = params.phone.trim_start_matches('+');
        let jid = Jid::pn(phone);

        let message = wa_rs::wa_rs_proto::whatsapp::Message { conversation: Some(params.text.clone()), ..Default::default() };

        let msg_id = self.client.send_message(jid, message).await.map_err(|e| format!("Failed to send WhatsApp message: {e}"))?;

        Ok(format!("Message sent to {phone}, id={msg_id}"))
    }

    #[tool(description = "Draft a WhatsApp message (returns content without sending)")]
    fn draft_message(&self, #[tool(aggr)] params: DraftMessageParams) -> String {
        serde_json::json!({
            "status": "drafted",
            "phone": params.phone,
            "text": params.text,
        })
        .to_string()
    }
}

#[tool(tool_box)]
impl ServerHandler for WhatsAppServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("WhatsApp messaging server via wa-rs (pure Rust)".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    tracing::info!("Starting mcp-whatsapp server");

    let store_path = std::env::var("WHATSAPP_STORE_PATH")
        .unwrap_or_else(|_| dirs::data_dir().unwrap().join("corre/whatsapp.db").to_string_lossy().into_owned());

    let backend = Arc::new(SqliteStore::new(&store_path).await?);

    let mut bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(wa_rs_tokio_transport::TokioWebSocketTransportFactory::new())
        .with_http_client(wa_rs_ureq_http::UreqHttpClient::new())
        .on_event(|event, _client| async move {
            use wa_rs::types::events::Event;
            match event {
                Event::PairingQrCode { code, .. } => {
                    tracing::info!("Scan QR code to pair WhatsApp:\n{code}");
                }
                Event::Connected(_) => {
                    tracing::info!("WhatsApp connected");
                }
                Event::LoggedOut(_) => {
                    tracing::warn!("WhatsApp logged out");
                }
                _ => {}
            }
        })
        .build()
        .await?;

    let client = bot.client();

    // Run the WhatsApp connection in the background
    tokio::spawn(async move {
        if let Err(e) = bot.run().await {
            tracing::error!("WhatsApp bot error: {e}");
        }
    });

    let server = WhatsAppServer { client };
    let transport = rmcp::transport::io::stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
