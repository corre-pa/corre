use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use corre_core::config::CorreConfig;
use corre_gym::assistant::AssistantHandler;
use corre_gym::config::GymConfig;
use corre_gym::db::Database;
use corre_gym::telegram::{Message, TelegramClient};
use corre_llm::OpenAiCompatProvider;

#[derive(Parser)]
#[command(name = "corre-gym", about = "Personal gym trainer Telegram bot")]
struct Cli {
    /// Path to corre.toml config file
    #[arg(short, long, default_value_os_t = default_config_path())]
    config: PathBuf,
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir().map(|d| d.join("corre")).unwrap_or_else(|| PathBuf::from("."))
}

fn default_config_path() -> PathBuf {
    default_data_dir().join("corre.toml")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (telegram, handler, allowed_ids) = setup().await?;
    run_polling_loop(&telegram, &handler, &allowed_ids).await
}

async fn setup() -> anyhow::Result<(TelegramClient, AssistantHandler, Vec<i64>)> {
    // 1. Load .env from data dir (best-effort, same as corre-news)
    let default_data_dir = default_data_dir();
    let _ = dotenvy::from_filename(default_data_dir.join(".env")).ok();

    // 2. Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    // 3. Parse CLI args
    let cli = Cli::parse();

    // 4. Load config
    tracing::info!(path = %cli.config.display(), "Loading config");
    let config = CorreConfig::load(&cli.config).context("loading config")?;
    let data_dir = config.data_dir();
    tracing::debug!(raw_gym = %config.gym, "Raw [gym] table from config");
    let mut gym_config = GymConfig::from_toml_table(Some(&config.gym))?;
    gym_config.resolve_secrets()?;

    // 5. Build LLM provider (with optional [gym.llm] overrides)
    tracing::debug!(gym_llm = ?gym_config.llm, "Gym LLM override");
    let effective_llm = match gym_config.llm.as_ref() {
        Some(overrides) => config.llm.with_overrides(overrides),
        None => config.llm.clone(),
    };
    tracing::info!(model = %effective_llm.model, base_url = %effective_llm.base_url, "LLM config loaded");
    let raw_llm: Box<dyn corre_core::app::LlmProvider> = Box::new(OpenAiCompatProvider::from_config(&effective_llm)?);
    let llm: Box<dyn corre_core::app::LlmProvider> = if config.safety.enabled {
        tracing::info!("Safety layer enabled — wrapping LLM provider");
        Box::new(corre_safety::SafeLlmProvider::new(raw_llm, &config.safety))
    } else {
        raw_llm
    };

    // 6. Open database
    let db = Database::open(&data_dir.join(&gym_config.db_path))?;
    let db = Arc::new(Mutex::new(db));

    // 7. Create Telegram client, verify connection
    let telegram = TelegramClient::new(&gym_config.telegram_bot_token)?;
    let me = telegram.get_me().await?;
    tracing::info!("Bot @{} connected (id: {})", me.username.as_deref().unwrap_or("?"), me.id);

    let allowed_ids = gym_config.telegram_allowed_ids.clone();

    // 8. Create handler
    let handler = AssistantHandler::new(db, llm, gym_config).await?;

    Ok((telegram, handler, allowed_ids))
}

async fn run_polling_loop(telegram: &TelegramClient, handler: &AssistantHandler, allowed_ids: &[i64]) -> anyhow::Result<()> {
    let mut offset = 0i64;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("Shutting down");
                break Ok(());
            }
            result = telegram.get_updates(offset, 30) => {
                match result {
                    Ok(updates) => {
                        for update in updates {
                            offset = update.update_id + 1;
                            if let Some(ref message) = update.message {
                                process_message(telegram, handler, message, allowed_ids).await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("get_updates failed: {e:#}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn process_message(telegram: &TelegramClient, handler: &AssistantHandler, message: &Message, allowed_ids: &[i64]) {
    // Skip non-private chats, messages without sender, messages without text
    if message.chat.chat_type != "private" {
        return;
    }
    let Some(ref from) = message.from else { return };
    let Some(ref text) = message.text else { return };

    // Authorization: reject users not in the allow-list (if non-empty)
    if !allowed_ids.is_empty() && !allowed_ids.contains(&from.id) {
        tracing::debug!("Ignoring message from unauthorized user {}", from.id);
        return;
    }

    let _ = telegram.send_chat_action(message.chat.id, "typing").await;

    let reply = match handler.handle_text_message(message, text).await {
        Ok(text) => text,
        Err(e) => {
            let msg = format!("{e:#}");
            tracing::error!("Handler error: {msg}");
            if msg.contains("401") || msg.contains("Unauthorized") || msg.contains("Authentication") {
                "I could not access the AI engine. You'll need to check that I'm properly configured with a valid API key.".to_string()
            } else {
                "I had trouble processing that -- could you try again?".to_string()
            }
        }
    };

    if let Err(e) = send_long_message(telegram, message.chat.id, &reply).await {
        tracing::error!("Failed to send reply: {e:#}");
    }
}

/// Splits messages exceeding Telegram's 4096 character limit.
async fn send_long_message(telegram: &TelegramClient, chat_id: i64, text: &str) -> anyhow::Result<()> {
    const MAX_LEN: usize = 4096;

    if text.len() <= MAX_LEN {
        telegram.send_message(chat_id, text, None, None).await?;
        return Ok(());
    }

    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= MAX_LEN {
            telegram.send_message(chat_id, remaining, None, None).await?;
            break;
        }

        let chunk = &remaining[..MAX_LEN];
        // Try splitting at the last newline
        let split_at = chunk.rfind('\n').or_else(|| chunk.rfind(' ')).unwrap_or(MAX_LEN);

        telegram.send_message(chat_id, &remaining[..split_at], None, None).await?;
        remaining = remaining[split_at..].trim_start();
    }

    Ok(())
}
