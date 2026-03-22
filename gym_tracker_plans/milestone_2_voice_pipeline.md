# Milestone 2: Voice Pipeline (Telegram)

## Goal

Users send voice messages to the Telegram bot and receive voice responses. Uses local
whisper.cpp for speech-to-text and Piper TTS for text-to-speech. No external API calls.

## Prerequisites

- Milestone 1 complete (Telegram text chat working)
- whisper.cpp compiled and available as `whisper-cli` binary
- Piper TTS installed with at least one English voice model
- ffmpeg installed (for audio format conversion)

## Voice message flow

```
User speaks into Telegram
  -> Telegram encodes as OGG/Opus, sends to bot
  -> Bot receives Update with message.voice field
  -> Bot downloads OGG file via getFile API
  -> ffmpeg converts OGG to 16kHz WAV (whisper input format)
  -> whisper-cli transcribes WAV to text
  -> Text enters same pipeline as M1 (context -> LLM -> actions -> reply)
  -> Reply text piped to Piper TTS -> WAV output
  -> ffmpeg converts WAV to OGG/Opus (Telegram voice format)
  -> Bot sends OGG as voice message via sendVoice API
  -> (Also sends text reply alongside, based on response_mode config)
```

## File structure

```
crates/corre-gym/src/
    voice/
      mod.rs              Re-exports
      stt.rs              whisper.cpp subprocess wrapper
      tts.rs              Piper TTS subprocess wrapper
      audio.rs            Audio format conversion (via ffmpeg subprocess)
    telegram/
      client.rs           Add: get_file(), download_file(), send_voice()
    main.rs               Add voice message handling to the polling loop
```

## Config additions

```toml
[gym.voice]
stt_enabled = true
whisper_binary = "whisper-cli"        # path to whisper.cpp CLI binary
whisper_model_path = "/data/models/ggml-base.en.bin"  # path to Whisper GGML model
tts_enabled = true
piper_binary = "piper"                # path to Piper binary
piper_model_path = "/data/models/en_US-lessac-medium.onnx"  # path to Piper ONNX model
piper_config_path = "/data/models/en_US-lessac-medium.onnx.json"  # Piper model config
response_mode = "both"                # "voice" | "text" | "both"
ffmpeg_binary = "ffmpeg"              # path to ffmpeg
temp_dir = "/tmp/corre-gym-audio"     # scratch directory for audio files
```

```rust
#[derive(Debug, Deserialize)]
pub struct VoiceConfig {
    #[serde(default = "default_true")]
    pub stt_enabled: bool,
    #[serde(default = "default_whisper_binary")]
    pub whisper_binary: String,
    pub whisper_model_path: String,
    #[serde(default = "default_true")]
    pub tts_enabled: bool,
    #[serde(default = "default_piper_binary")]
    pub piper_binary: String,
    pub piper_model_path: String,
    pub piper_config_path: Option<String>,
    #[serde(default = "default_response_mode")]
    pub response_mode: ResponseMode,  // Voice, Text, Both
    #[serde(default = "default_ffmpeg")]
    pub ffmpeg_binary: String,
    #[serde(default = "default_temp_dir")]
    pub temp_dir: String,
}
```

## Speech-to-text: whisper.cpp (voice/stt.rs)

### WhisperStt struct

```rust
pub struct WhisperStt {
    binary: PathBuf,
    model_path: PathBuf,
}

impl WhisperStt {
    pub fn new(binary: &str, model_path: &str) -> Result<Self>;

    /// Transcribe a WAV file to text.
    /// The WAV must be 16kHz mono (whisper's expected format).
    pub async fn transcribe(&self, wav_path: &Path) -> Result<String>;

    /// Check that the binary and model file exist.
    pub fn verify(&self) -> Result<()>;
}
```

### Implementation

Spawn whisper-cli as a subprocess:

```rust
async fn transcribe(&self, wav_path: &Path) -> Result<String> {
    let output = Command::new(&self.binary)
        .arg("-m").arg(&self.model_path)
        .arg("-f").arg(wav_path)
        .arg("--no-timestamps")
        .arg("--print-special").arg("false")
        .arg("--language").arg("en")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await
        .context("failed to run whisper-cli")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("whisper-cli failed: {stderr}");
    }

    let transcript = String::from_utf8(output.stdout)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(transcript)
}
```

### whisper.cpp installation notes

whisper.cpp must be compiled from source or obtained as a pre-built binary:

```sh
# Build from source
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp
cmake -B build
cmake --build build --config Release
# Binary at build/bin/whisper-cli

# Download model
./models/download-ggml-model.sh base.en
# Model at models/ggml-base.en.bin
```

For Docker, include in the Dockerfile build stage.

## Text-to-speech: Piper (voice/tts.rs)

### PiperTts struct

```rust
pub struct PiperTts {
    binary: PathBuf,
    model_path: PathBuf,
    config_path: Option<PathBuf>,
}

impl PiperTts {
    pub fn new(binary: &str, model_path: &str, config_path: Option<&str>) -> Result<Self>;

    /// Synthesize text to a WAV file.
    pub async fn synthesize(&self, text: &str, output_path: &Path) -> Result<()>;

    /// Check that the binary and model file exist.
    pub fn verify(&self) -> Result<()>;
}
```

### Implementation

```rust
async fn synthesize(&self, text: &str, output_path: &Path) -> Result<()> {
    let mut cmd = Command::new(&self.binary);
    cmd.arg("--model").arg(&self.model_path)
       .arg("--output_file").arg(output_path);

    if let Some(ref config) = self.config_path {
        cmd.arg("--config").arg(config);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn piper")?;

    // Write text to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).await?;
        drop(stdin);
    }

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("piper failed: {stderr}");
    }

    Ok(())
}
```

### Piper installation notes

```sh
# Download pre-built binary
# https://github.com/rhasspy/piper/releases

# Download a voice model
# https://huggingface.co/rhasspy/piper-voices
# e.g. en_US-lessac-medium.onnx + en_US-lessac-medium.onnx.json
```

## Audio format conversion (voice/audio.rs)

Use ffmpeg subprocess for reliable format conversion:

```rust
pub struct AudioConverter {
    ffmpeg_binary: String,
    temp_dir: PathBuf,
}

impl AudioConverter {
    pub fn new(ffmpeg_binary: &str, temp_dir: &str) -> Result<Self>;

    /// Convert OGG/Opus to 16kHz mono WAV (whisper input format).
    pub async fn ogg_to_wav(&self, ogg_path: &Path) -> Result<PathBuf>;

    /// Convert WAV to OGG/Opus (Telegram voice message format).
    pub async fn wav_to_ogg(&self, wav_path: &Path) -> Result<PathBuf>;

    /// Clean up temporary files for a given session.
    pub fn cleanup(&self, prefix: &str) -> Result<()>;
}
```

### Implementation

```rust
async fn ogg_to_wav(&self, ogg_path: &Path) -> Result<PathBuf> {
    let wav_path = self.temp_dir.join(format!("{}.wav", Uuid::new_v4()));
    let output = Command::new(&self.ffmpeg_binary)
        .args(["-i"])
        .arg(ogg_path)
        .args(["-ar", "16000", "-ac", "1", "-f", "wav"])
        .arg(&wav_path)
        .args(["-y", "-loglevel", "error"])
        .output()
        .await
        .context("failed to run ffmpeg")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg ogg->wav failed: {stderr}");
    }
    Ok(wav_path)
}

async fn wav_to_ogg(&self, wav_path: &Path) -> Result<PathBuf> {
    let ogg_path = self.temp_dir.join(format!("{}.ogg", Uuid::new_v4()));
    let output = Command::new(&self.ffmpeg_binary)
        .args(["-i"])
        .arg(wav_path)
        .args(["-c:a", "libopus", "-b:a", "64k", "-f", "ogg"])
        .arg(&ogg_path)
        .args(["-y", "-loglevel", "error"])
        .output()
        .await
        .context("failed to run ffmpeg")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg wav->ogg failed: {stderr}");
    }
    Ok(ogg_path)
}
```

## Telegram client additions (telegram/client.rs)

```rust
impl TelegramClient {
    /// Get file path for a file_id.
    pub async fn get_file(&self, file_id: &str) -> Result<String>;

    /// Download a file to a local path.
    pub async fn download_file(&self, file_path: &str, dest: &Path) -> Result<()>;

    /// Send a voice message (OGG/Opus file).
    pub async fn send_voice(&self, chat_id: i64, ogg_path: &Path) -> Result<Message>;
}
```

### Implementation

```rust
async fn get_file(&self, file_id: &str) -> Result<String> {
    let url = format!("https://api.telegram.org/bot{}/getFile", self.token);
    let resp: ApiResponse<File> = self.client
        .post(&url)
        .json(&json!({"file_id": file_id}))
        .send().await?
        .json().await?;
    Ok(resp.result.file_path.context("no file_path in response")?)
}

async fn download_file(&self, file_path: &str, dest: &Path) -> Result<()> {
    let url = format!("https://api.telegram.org/file/bot{}/{}", self.token, file_path);
    let bytes = self.client.get(&url).send().await?.bytes().await?;
    tokio::fs::write(dest, &bytes).await?;
    Ok(())
}

async fn send_voice(&self, chat_id: i64, ogg_path: &Path) -> Result<Message> {
    let url = format!("https://api.telegram.org/bot{}/sendVoice", self.token);
    let file_bytes = tokio::fs::read(ogg_path).await?;
    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name("voice.ogg")
        .mime_str("audio/ogg")?;
    let form = reqwest::multipart::Form::new()
        .text("chat_id", chat_id.to_string())
        .part("voice", part);
    let resp: ApiResponse<Message> = self.client
        .post(&url)
        .multipart(form)
        .send().await?
        .json().await?;
    Ok(resp.result)
}
```

## Main loop changes (main.rs)

Update the polling loop to handle voice messages:

```rust
if let Some(message) = update.message {
    if let Some(text) = &message.text {
        // Text message (existing M1 flow)
        telegram.send_chat_action(message.chat.id, "typing").await.ok();
        let reply = handler.handle_text_message(&message, text).await;
        telegram.send_message(message.chat.id, &reply, Some("Markdown")).await?;

    } else if let Some(voice) = &message.voice {
        // Voice message (new M2 flow)
        telegram.send_chat_action(message.chat.id, "record_voice").await.ok();

        match handler.handle_voice_message(&message, &voice.file_id).await {
            Ok((reply_text, reply_ogg)) => {
                // Send text reply if configured
                if matches!(response_mode, ResponseMode::Text | ResponseMode::Both) {
                    telegram.send_message(message.chat.id, &reply_text, Some("Markdown")).await?;
                }
                // Send voice reply if configured and TTS succeeded
                if let Some(ogg_path) = reply_ogg {
                    if matches!(response_mode, ResponseMode::Voice | ResponseMode::Both) {
                        telegram.send_voice(message.chat.id, &ogg_path).await?;
                        tokio::fs::remove_file(&ogg_path).await.ok();
                    }
                }
            }
            Err(e) => {
                tracing::error!("Voice processing failed: {e:#}");
                telegram.send_message(
                    message.chat.id,
                    "Sorry, I couldn't process that voice message. Try sending it as text.",
                    None,
                ).await?;
            }
        }
    }
}
```

## Voice handler (assistant/handler.rs)

```rust
impl AssistantHandler {
    /// Process a voice message. Returns (reply_text, Option<reply_ogg_path>).
    pub async fn handle_voice_message(
        &self,
        message: &Message,
        file_id: &str,
    ) -> Result<(String, Option<PathBuf>)> {
        // 1. Download OGG from Telegram
        let file_path = self.telegram.get_file(file_id).await?;
        let ogg_path = self.audio.temp_path("input", "ogg");
        self.telegram.download_file(&file_path, &ogg_path).await?;

        // 2. Convert OGG to WAV
        let wav_path = self.audio.ogg_to_wav(&ogg_path).await?;

        // 3. Transcribe
        let transcript = self.stt.transcribe(&wav_path).await?;
        tracing::info!("Transcribed voice: {transcript}");

        // 4. Process as text (same as M1)
        let reply_text = self.handle_text_message(message, &transcript).await;

        // 5. Synthesize reply voice (if TTS enabled)
        let reply_ogg = if self.tts_enabled {
            let reply_wav = self.audio.temp_path("reply", "wav");
            self.tts.synthesize(&reply_text, &reply_wav).await?;
            let reply_ogg = self.audio.wav_to_ogg(&reply_wav).await?;
            // Clean up intermediate files
            tokio::fs::remove_file(&reply_wav).await.ok();
            Some(reply_ogg)
        } else {
            None
        };

        // 6. Clean up input files
        tokio::fs::remove_file(&ogg_path).await.ok();
        tokio::fs::remove_file(&wav_path).await.ok();

        Ok((reply_text, reply_ogg))
    }
}
```

## UX considerations

### Typing/recording indicators

- Text messages: send `typing` action before LLM call
- Voice messages: send `record_voice` action while processing (visible in Telegram UI)

### Latency

The full voice pipeline has non-trivial latency:
1. Download OGG from Telegram: ~100ms
2. Convert to WAV: ~50ms
3. whisper.cpp transcription: ~1-3s (base.en model on CPU)
4. LLM call: ~1-5s
5. Piper TTS synthesis: ~0.5-2s
6. Convert to OGG: ~50ms
7. Upload to Telegram: ~100ms

Total: ~3-10 seconds. The `record_voice` indicator keeps the user informed.

### Error handling

- If whisper.cpp fails: fall back to asking user to type
- If Piper TTS fails: send text-only reply (graceful degradation)
- If ffmpeg fails: send text-only reply with error note

## Dockerfile additions

```dockerfile
# Stage: build whisper.cpp
FROM debian:bookworm-slim AS whisper-build
RUN apt-get update && apt-get install -y git cmake build-essential
RUN git clone https://github.com/ggerganov/whisper.cpp /whisper
WORKDIR /whisper
RUN cmake -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build --config Release
RUN ./models/download-ggml-model.sh base.en

# Stage: download piper
FROM debian:bookworm-slim AS piper-download
RUN apt-get update && apt-get install -y wget tar
RUN wget https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz \
    && tar xzf piper_linux_x86_64.tar.gz
# Download voice model
RUN wget https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx
RUN wget https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json

# Final stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ffmpeg && rm -rf /var/lib/apt/lists/*
COPY --from=whisper-build /whisper/build/bin/whisper-cli /usr/local/bin/
COPY --from=whisper-build /whisper/models/ggml-base.en.bin /data/models/
COPY --from=piper-download /piper/piper /usr/local/bin/
COPY --from=piper-download /en_US-lessac-medium.onnx /data/models/
COPY --from=piper-download /en_US-lessac-medium.onnx.json /data/models/
COPY --from=builder /app/corre-gym /app/
```

## Tests

### STT tests
- `transcribe_returns_text` -- run whisper-cli on a known test WAV file
- `transcribe_handles_silence` -- empty/silent audio returns empty string
- `verify_checks_binary_exists` -- returns error if binary missing

### TTS tests
- `synthesize_creates_wav` -- verify output file exists and is valid WAV
- `synthesize_handles_empty_text` -- graceful handling of empty input
- `verify_checks_model_exists` -- returns error if model missing

### Audio conversion tests
- `ogg_to_wav_round_trip` -- convert OGG->WAV->OGG, verify output is valid
- `temp_files_cleaned_up` -- verify cleanup removes temp files

### Integration tests
- End-to-end: WAV input -> transcribe -> LLM -> synthesize -> OGG output
- Graceful fallback when whisper unavailable (returns error, caller sends text)
- Graceful fallback when piper unavailable (returns None for voice reply)

## Verification

```sh
# Unit tests (some require whisper-cli and piper installed)
cargo test -p corre-gym -- voice

# Manual testing
# 1. Ensure whisper-cli, piper, ffmpeg are in PATH
# 2. Run the bot
cargo run -p corre-gym -- -c ~/.local/share/corre/corre.toml

# 3. In Telegram, send a voice message: "I just did five sets of squats at 100 kilos"
# 4. Verify: bot transcribes correctly, logs the exercise, replies with voice
```
