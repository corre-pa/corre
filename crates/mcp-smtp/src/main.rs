use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool};

/// SMTP email MCP server.
///
/// Sends or drafts emails via SMTP. Configured through environment variables:
/// - `SMTP_HOST`: SMTP server hostname
/// - `SMTP_PORT`: SMTP server port (default: 587)
/// - `SMTP_USER`: SMTP username
/// - `SMTP_PASSWORD`: SMTP password
/// - `SMTP_FROM`: Sender email address
#[derive(Debug, Clone, Default)]
struct SmtpServer;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SendEmailParams {
    #[schemars(description = "Recipient email address")]
    to: String,
    #[schemars(description = "Email subject line")]
    subject: String,
    #[schemars(description = "Email body text")]
    body: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct DraftEmailParams {
    #[schemars(description = "Recipient email address")]
    to: String,
    #[schemars(description = "Email subject line")]
    subject: String,
    #[schemars(description = "Email body text")]
    body: String,
}

#[tool(tool_box)]
impl SmtpServer {
    #[tool(description = "Send an email via SMTP")]
    async fn send_email(&self, #[tool(aggr)] params: SendEmailParams) -> Result<String, String> {
        use lettre::message::header::ContentType;
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

        let host = std::env::var("SMTP_HOST").map_err(|_| "SMTP_HOST not set")?;
        let port: u16 = std::env::var("SMTP_PORT").unwrap_or_else(|_| "587".into()).parse().map_err(|_| "Invalid SMTP_PORT")?;
        let user = std::env::var("SMTP_USER").map_err(|_| "SMTP_USER not set")?;
        let password = std::env::var("SMTP_PASSWORD").map_err(|_| "SMTP_PASSWORD not set")?;
        let from = std::env::var("SMTP_FROM").map_err(|_| "SMTP_FROM not set")?;

        let email = Message::builder()
            .from(from.parse().map_err(|e| format!("Invalid from address: {e}"))?)
            .to(params.to.parse().map_err(|e| format!("Invalid to address: {e}"))?)
            .subject(&params.subject)
            .header(ContentType::TEXT_PLAIN)
            .body(params.body)
            .map_err(|e| format!("Failed to build email: {e}"))?;

        let creds = Credentials::new(user, password);
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(&host)
            .map_err(|e| format!("Failed to create SMTP transport: {e}"))?
            .port(port)
            .credentials(creds)
            .build();

        mailer.send(email).await.map_err(|e| format!("Failed to send email: {e}"))?;
        Ok(format!("Email sent to {}", params.to))
    }

    #[tool(description = "Draft an email (returns the email content without sending)")]
    fn draft_email(&self, #[tool(aggr)] params: DraftEmailParams) -> String {
        serde_json::json!({
            "status": "drafted",
            "to": params.to,
            "subject": params.subject,
            "body": params.body,
        })
        .to_string()
    }
}

#[tool(tool_box)]
impl ServerHandler for SmtpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("SMTP email server for sending and drafting emails".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    tracing::info!("Starting mcp-smtp server");

    let server = SmtpServer;
    let transport = rmcp::transport::io::stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
