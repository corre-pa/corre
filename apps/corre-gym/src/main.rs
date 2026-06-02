use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser;
use tokio::sync::{Mutex, oneshot};
use tracing_subscriber::EnvFilter;

use corre_core::config::CorreConfig;
use corre_gym::assistant::{AssistantHandler, RestTimer};
use corre_gym::config::GymConfig;
use corre_gym::db::Database;
use corre_gym::telegram::{Message, TelegramClient, Voice};
use corre_gym::voice::VoicePipeline;
use corre_gym::web;
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
    let (telegram, handler, allowed_ids, voice_pipeline, gym_config, db, bot_username) = setup().await?;
    let handler = Arc::new(handler);
    let rest_timers = Arc::new(RestTimerRegistry::default());

    tokio::select! {
        result = run_polling_loop(&telegram, &handler, &allowed_ids, voice_pipeline.as_ref(), &rest_timers) => {
            result
        }
        result = web::serve(
            &gym_config.bind,
            db,
            handler.clone(),
            gym_config.clone(),
            bot_username,
            None,
        ) => {
            result
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Ctrl+C received, shutting down");
            Ok(())
        }
    }
}

/// Tracks one in-flight rest timer per chat_id. Installing a new timer
/// cancels the previous one (so consecutive sets restart the countdown).
#[derive(Default)]
struct RestTimerRegistry {
    inner: Mutex<HashMap<i64, oneshot::Sender<()>>>,
}

impl RestTimerRegistry {
    /// Register a fresh cancellation receiver for `chat_id`, evicting (and
    /// signalling) any previously-installed timer.
    async fn install(&self, chat_id: i64) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        let mut guard = self.inner.lock().await;
        if let Some(prev) = guard.insert(chat_id, tx) {
            let _ = prev.send(());
        }
        rx
    }

    /// Cancel a pending timer for `chat_id`, if any. No-op when nothing is armed.
    async fn clear(&self, chat_id: i64) {
        let mut guard = self.inner.lock().await;
        if let Some(prev) = guard.remove(&chat_id) {
            let _ = prev.send(());
        }
    }

    /// Drop the registry entry for `chat_id` without signalling — used when a
    /// timer's spawned task completes naturally.
    async fn forget(&self, chat_id: i64) {
        let mut guard = self.inner.lock().await;
        guard.remove(&chat_id);
    }
}

async fn setup()
-> anyhow::Result<(TelegramClient, AssistantHandler, Vec<i64>, Option<VoicePipeline>, GymConfig, Arc<Mutex<Database>>, String)> {
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
    gym_config.resolve_endpoints()?;

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

    // 6. Open database (RwLock for parallel reads from web + Telegram)
    let db = Database::open(&data_dir.join(&gym_config.db_path))?;
    let db = Arc::new(Mutex::new(db));

    // 7. Create Telegram client, verify connection
    let telegram = TelegramClient::new(&gym_config.telegram_bot_token)?;
    let me = telegram.get_me().await?;
    let bot_username = me.username.clone().unwrap_or_default();
    debug_assert!(!bot_username.starts_with('@'), "bot_username should not start with @");
    tracing::info!("Bot @{bot_username} connected (id: {})", me.id);

    let allowed_ids = gym_config.telegram_allowed_ids.clone();

    // 8. Create handler
    let handler = AssistantHandler::new(db.clone(), llm, gym_config.clone()).await?;

    // 9. Voice pipeline (optional)
    let voice_pipeline = match &gym_config.voice {
        Some(voice_config) if voice_config.stt_enabled => {
            voice_config.validate()?;
            let pipeline = VoicePipeline::new(voice_config);
            match pipeline.verify().await {
                Ok(()) => {
                    tracing::info!(
                        stt_url = %voice_config.stt_url,
                        tts = if voice_config.tts_enabled { &voice_config.tts_url } else { "disabled" },
                        "Voice pipeline active"
                    );
                    Some(pipeline)
                }
                Err(e) => {
                    tracing::warn!("Voice services unreachable, voice disabled: {e:#}");
                    None
                }
            }
        }
        _ => {
            tracing::info!("Voice pipeline not configured");
            None
        }
    };

    Ok((telegram, handler, allowed_ids, voice_pipeline, gym_config, db, bot_username))
}

async fn run_polling_loop(
    telegram: &TelegramClient,
    handler: &Arc<AssistantHandler>,
    allowed_ids: &[i64],
    voice_pipeline: Option<&VoicePipeline>,
    rest_timers: &Arc<RestTimerRegistry>,
) -> anyhow::Result<()> {
    let mut offset = 0i64;

    loop {
        match telegram.get_updates(offset, 30).await {
            Ok(updates) => {
                for update in updates {
                    offset = update.update_id + 1;
                    if let Some(ref message) = update.message {
                        process_message(telegram, handler, voice_pipeline, message, allowed_ids, rest_timers).await;
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

async fn process_message(
    telegram: &TelegramClient,
    handler: &Arc<AssistantHandler>,
    voice_pipeline: Option<&VoicePipeline>,
    message: &Message,
    allowed_ids: &[i64],
    rest_timers: &Arc<RestTimerRegistry>,
) {
    if message.chat.chat_type != "private" {
        return;
    }
    let Some(ref from) = message.from else { return };
    if !allowed_ids.is_empty() && !allowed_ids.contains(&from.id) {
        tracing::debug!("Ignoring message from unauthorized user {}", from.id);
        return;
    }

    if let Some(ref text) = message.text {
        process_text_message(telegram, handler, message, text, rest_timers).await;
    } else if let Some(ref voice) = message.voice {
        process_voice_message(telegram, handler, voice_pipeline, message, voice, rest_timers).await;
    } else if message.audio.is_some() {
        if let Err(e) =
            telegram.send_message(message.chat.id, "Please use the microphone button to record voice messages directly.", None, None).await
        {
            tracing::warn!("Failed to send audio guidance: {e:#}");
        }
    }
}

async fn process_text_message(
    telegram: &TelegramClient,
    handler: &Arc<AssistantHandler>,
    message: &Message,
    text: &str,
    rest_timers: &Arc<RestTimerRegistry>,
) {
    let _ = telegram.send_chat_action(message.chat.id, "typing").await;

    let (reply_text, parse_mode, rest_timer, cancel_rest_timer) = match handler.handle_text_message(message, text).await {
        Ok(reply) => (reply.text, reply.parse_mode, reply.rest_timer, reply.cancel_rest_timer),
        Err(e) => {
            tracing::error!("Handler error: {e:#}");
            ("Something went wrong -- please try again later.".to_string(), None, None, false)
        }
    };

    if let Err(e) = send_long_message(telegram, message.chat.id, &reply_text, parse_mode).await {
        tracing::error!("Failed to send reply: {e:#}");
    }

    apply_rest_timer_directive(telegram, rest_timers, message.chat.id, rest_timer, cancel_rest_timer).await;
}

async fn process_voice_message(
    telegram: &TelegramClient,
    handler: &Arc<AssistantHandler>,
    voice_pipeline: Option<&VoicePipeline>,
    message: &Message,
    voice: &Voice,
    rest_timers: &Arc<RestTimerRegistry>,
) {
    // 0. Check if voice is enabled
    let Some(pipeline) = voice_pipeline else {
        if let Err(e) =
            telegram.send_message(message.chat.id, "Voice messages are not enabled. Please type your message instead.", None, None).await
        {
            tracing::warn!("Failed to send voice-disabled notice: {e:#}");
        }
        return;
    };

    // 1. Reject overly long messages
    if voice.duration as u32 > pipeline.max_duration_secs() {
        if let Err(e) = telegram
            .send_message(
                message.chat.id,
                "That voice message is too long. Please keep it under 60 seconds, or type your message.",
                None,
                None,
            )
            .await
        {
            tracing::warn!("Failed to send duration-limit notice: {e:#}");
        }
        return;
    }

    // 2. Start chat action refresh loop (re-sends every 4s to avoid 5s expiry)
    let stop_action = spawn_chat_action_loop(telegram, message.chat.id, "record_voice");

    // 3. Download OGG from Telegram
    let ogg_bytes = match download_voice(telegram, &voice.file_id).await {
        Ok(bytes) => bytes,
        Err(e) => {
            let _ = stop_action.send(());
            tracing::error!("Failed to download voice: {e:#}");
            if let Err(e) =
                telegram.send_message(message.chat.id, "I couldn't download that voice message. Could you try again?", None, None).await
            {
                tracing::warn!("Failed to send download-error notice: {e:#}");
            }
            return;
        }
    };

    // 4. Transcribe via whisper
    let transcript = match pipeline.speech_to_text(&ogg_bytes).await {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            let _ = stop_action.send(());
            if let Err(e) = telegram
                .send_message(message.chat.id, "I couldn't make out what you said. Could you try again, or type your message?", None, None)
                .await
            {
                tracing::warn!("Failed to send empty-transcript notice: {e:#}");
            }
            return;
        }
        Err(e) => {
            let _ = stop_action.send(());
            tracing::error!("STT failed: {e:#}");
            if let Err(e) = telegram
                .send_message(message.chat.id, "I had trouble understanding that voice message. Could you type it instead?", None, None)
                .await
            {
                tracing::warn!("Failed to send STT-error notice: {e:#}");
            }
            return;
        }
    };

    tracing::info!(duration = voice.duration, transcript = %transcript, "Voice transcribed");

    // 5. Switch chat action to "typing" for the LLM call
    let _ = stop_action.send(());
    let stop_action = spawn_chat_action_loop(telegram, message.chat.id, "typing");

    // 6. Process transcript through handler (identical to text messages)
    let (reply, rest_timer, cancel_rest_timer) = match handler.handle_text_message(message, &transcript).await {
        Ok(r) => (r.text, r.rest_timer, r.cancel_rest_timer),
        Err(e) => {
            tracing::error!("Handler error: {e:#}");
            ("I had trouble processing that -- could you try again?".to_string(), None, false)
        }
    };

    let _ = stop_action.send(());

    // 7. Send text reply with transcript echo (if configured)
    if pipeline.should_send_text() {
        let text_with_echo = format!("_Heard: \"{transcript}\"_\n\n{reply}");
        if let Err(e) = send_long_message(telegram, message.chat.id, &text_with_echo, None).await {
            tracing::error!("Failed to send text reply: {e:#}");
        }
    }

    // 8. Synthesize and send voice reply (if configured)
    if pipeline.should_send_voice() {
        match pipeline.text_to_speech(&reply).await {
            Ok(Some(ogg_bytes)) => {
                let _ = telegram.send_chat_action(message.chat.id, "upload_voice").await;
                if let Err(e) = telegram.send_voice(message.chat.id, &ogg_bytes, None).await {
                    tracing::error!("Failed to send voice reply: {e:#}");
                    // Fallback: send text if we haven't already
                    if !pipeline.should_send_text() {
                        let text_with_echo = format!("_Heard: \"{transcript}\"_\n\n{reply}");
                        if let Err(e) = send_long_message(telegram, message.chat.id, &text_with_echo, None).await {
                            tracing::warn!("Failed to send fallback text: {e:#}");
                        }
                    }
                }
            }
            Ok(None) => {} // TTS disabled
            Err(e) => {
                tracing::warn!("TTS synthesis failed: {e:#}");
                // Graceful degradation: send text if we haven't already
                if !pipeline.should_send_text() {
                    let text_with_echo = format!("_Heard: \"{transcript}\"_\n\n{reply}");
                    if let Err(e) = send_long_message(telegram, message.chat.id, &text_with_echo, None).await {
                        tracing::warn!("Failed to send fallback text: {e:#}");
                    }
                }
            }
        }
    }

    apply_rest_timer_directive(telegram, rest_timers, message.chat.id, rest_timer, cancel_rest_timer).await;
}

/// Apply a `Reply::rest_timer` / `Reply::cancel_rest_timer` directive: cancel
/// any pending timer first (so consecutive sets restart the countdown), then
/// arm a fresh one if requested.
async fn apply_rest_timer_directive(
    telegram: &TelegramClient,
    rest_timers: &Arc<RestTimerRegistry>,
    chat_id: i64,
    rest_timer: Option<RestTimer>,
    cancel: bool,
) {
    if cancel {
        rest_timers.clear(chat_id).await;
    }
    if let Some(timer) = rest_timer {
        spawn_rest_timer(telegram.clone(), rest_timers.clone(), chat_id, timer).await;
    }
}

/// Spawn a tokio task that emits the three timed rest notifications. The task
/// installs itself in the registry so a subsequent `clear`/`install` cancels it.
async fn spawn_rest_timer(telegram: TelegramClient, registry: Arc<RestTimerRegistry>, chat_id: i64, timer: RestTimer) {
    let rx = registry.install(chat_id).await;
    tokio::spawn(async move {
        run_rest_timer_segments(rx, timer, |text| {
            let telegram = telegram.clone();
            async move {
                if let Err(e) = telegram.send_message(chat_id, &text, None, None).await {
                    tracing::warn!("Failed to send rest-timer message to chat {chat_id}: {e:#}");
                }
            }
        })
        .await;
        registry.forget(chat_id).await;
    });
}

/// The pure timer state machine. Sleeps in three segments — the 10 s warning,
/// the 5 s warning, then completion — and calls `send` between each. Aborts as
/// soon as `rx` resolves (a cancellation).
///
/// Edge cases:
///   - duration ≤ 5 s: skip both warnings, fire "rest complete" after the wait.
///   - duration ≤ 10 s: skip the 10 s warning, fire the 5 s warning and the
///     completion message.
async fn run_rest_timer_segments<F, Fut>(mut rx: oneshot::Receiver<()>, timer: RestTimer, send: F)
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _ = timer.exercise_name; // routing happens via chat_id; name is for diagnostics only
    let total = u64::from(timer.duration_secs);

    if total > 10 {
        if !sleep_or_cancel(&mut rx, Duration::from_secs(total - 10)).await {
            return;
        }
        send("Rest timer: 10 seconds left before your next set.".to_string()).await;
        if !sleep_or_cancel(&mut rx, Duration::from_secs(5)).await {
            return;
        }
        send("Rest timer: 5 seconds left.".to_string()).await;
        if !sleep_or_cancel(&mut rx, Duration::from_secs(5)).await {
            return;
        }
    } else if total > 5 {
        if !sleep_or_cancel(&mut rx, Duration::from_secs(total - 5)).await {
            return;
        }
        send("Rest timer: 5 seconds left.".to_string()).await;
        if !sleep_or_cancel(&mut rx, Duration::from_secs(5)).await {
            return;
        }
    } else if !sleep_or_cancel(&mut rx, Duration::from_secs(total)).await {
        return;
    }

    send("Rest complete — time for your next set.".to_string()).await;
}

/// Race a sleep against a cancellation channel. Returns `true` if the full
/// sleep elapsed, `false` if cancellation arrived first.
async fn sleep_or_cancel(rx: &mut oneshot::Receiver<()>, dur: Duration) -> bool {
    tokio::select! {
        _ = rx => false,
        _ = tokio::time::sleep(dur) => true,
    }
}

async fn download_voice(telegram: &TelegramClient, file_id: &str) -> anyhow::Result<Vec<u8>> {
    let file = telegram.get_file(file_id).await?;
    let file_path = file.file_path.context("Telegram returned no file_path")?;
    telegram.download_file_bytes(&file_path).await
}

/// Re-sends a chat action every 4 seconds until the returned sender is dropped or signalled.
/// Telegram chat actions expire after 5 seconds, so this keeps the UI responsive during
/// long operations (transcription, LLM calls, synthesis).
fn spawn_chat_action_loop(telegram: &TelegramClient, chat_id: i64, action: &str) -> tokio::sync::oneshot::Sender<()> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let telegram = telegram.clone();
    let action = action.to_string();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut rx => break,
                _ = tokio::time::sleep(Duration::from_secs(4)) => {
                    let _ = telegram.send_chat_action(chat_id, &action).await;
                }
            }
        }
    });
    tx
}

/// Splits messages exceeding Telegram's 4096 character limit.
async fn send_long_message(telegram: &TelegramClient, chat_id: i64, text: &str, parse_mode: Option<&str>) -> anyhow::Result<()> {
    const MAX_LEN: usize = 4096;

    if text.len() <= MAX_LEN {
        telegram.send_message(chat_id, text, parse_mode, None).await?;
        return Ok(());
    }

    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= MAX_LEN {
            telegram.send_message(chat_id, remaining, parse_mode, None).await?;
            break;
        }

        let chunk = &remaining[..MAX_LEN];
        // Try splitting at the last newline
        let split_at = chunk.rfind('\n').or_else(|| chunk.rfind(' ')).unwrap_or(MAX_LEN);

        telegram.send_message(chat_id, &remaining[..split_at], parse_mode, None).await?;
        remaining = remaining[split_at..].trim_start();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn make_timer(duration_secs: u32) -> RestTimer {
        RestTimer { duration_secs, exercise_name: "Bench Press".to_string(), is_superset: false }
    }

    /// Drives the segment loop with virtualised time so each test runs in a
    /// handful of microseconds. Returns the messages sent in order. The
    /// `cancel_after` option simulates a clear/eviction by sending on the
    /// cancellation channel after the given delay.
    async fn run_segments_collecting(duration_secs: u32, cancel_after: Option<Duration>) -> Vec<String> {
        let messages: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let sink = messages.clone();
        let (tx, rx) = oneshot::channel();
        let timer = make_timer(duration_secs);

        let send = move |text: String| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(text);
            }
        };

        let segments = tokio::spawn(async move { run_rest_timer_segments(rx, timer, send).await });
        let mut tx_holder = Some(tx);
        if let Some(after) = cancel_after {
            tokio::time::sleep(after).await;
            if let Some(tx) = tx_holder.take() {
                let _ = tx.send(());
            }
        }
        // Hold the sender until segments completes so the natural "no
        // cancellation" case doesn't surface as a dropped-sender event on `rx`.
        segments.await.unwrap();
        drop(tx_holder);
        let guard = messages.lock().unwrap();
        guard.clone()
    }

    #[tokio::test(start_paused = true)]
    async fn rest_timer_segments_fire_all_three_messages_in_order() {
        // 120 s rest → 110 s wait, "10 s left", 5 s wait, "5 s left", 5 s wait, "complete".
        let messages = run_segments_collecting(120, None).await;
        assert_eq!(
            messages,
            vec![
                "Rest timer: 10 seconds left before your next set.".to_string(),
                "Rest timer: 5 seconds left.".to_string(),
                "Rest complete — time for your next set.".to_string(),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rest_timer_cancel_halts_subsequent_messages() {
        // Cancel after 50 s — before the 10 s warning fires (which is at 110 s).
        let messages = run_segments_collecting(120, Some(Duration::from_secs(50))).await;
        assert!(messages.is_empty(), "cancellation before first segment must suppress all messages");
    }

    #[tokio::test(start_paused = true)]
    async fn rest_timer_short_duration_skips_10s_warning() {
        // 8 s rest → wait 3 s, "5 s left", wait 5 s, "complete". No 10 s warning.
        let messages = run_segments_collecting(8, None).await;
        assert_eq!(messages, vec!["Rest timer: 5 seconds left.".to_string(), "Rest complete — time for your next set.".to_string()]);
    }

    #[tokio::test(start_paused = true)]
    async fn rest_timer_very_short_duration_only_completion() {
        // 3 s rest: skip both warnings.
        let messages = run_segments_collecting(3, None).await;
        assert_eq!(messages, vec!["Rest complete — time for your next set.".to_string()]);
    }

    #[tokio::test]
    async fn registry_install_evicts_previous_chat_timer() {
        let registry = Arc::new(RestTimerRegistry::default());
        let rx1 = registry.install(42).await;
        let mut rx2 = registry.install(42).await;
        // The first receiver must have been cancelled by the second install.
        assert!(rx1.await.is_ok(), "previous receiver must observe cancellation");
        // The new receiver is still pending.
        assert!(rx2.try_recv().is_err());
        registry.clear(42).await;
        assert!(rx2.await.is_ok());
    }

    #[tokio::test]
    async fn registry_clear_signals_pending_receiver() {
        let registry = Arc::new(RestTimerRegistry::default());
        let rx = registry.install(7).await;
        registry.clear(7).await;
        assert!(rx.await.is_ok());
    }
}
