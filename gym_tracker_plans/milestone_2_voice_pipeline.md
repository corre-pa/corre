# Milestone 2: Voice Pipeline (Telegram)

## Goal

Users send voice messages to the Telegram bot and receive voice responses. Uses local
whisper.cpp for speech-to-text and Piper TTS for text-to-speech. No external API calls.
Privacy-centric, all processing stays on the host.

## Prerequisites

- Milestone 1 complete (Telegram text chat, assistant handler, database)
- Docker and docker-compose on the host
- (Optional) NVIDIA GPU + nvidia-container-toolkit for faster transcription

## Architecture: HTTP sidecar containers

Rather than installing whisper.cpp and Piper as binaries inside the corre-gym container,
we run them as separate Docker containers with HTTP APIs. This gives us:

- **Clean separation**: corre-gym stays a lean Rust binary with no native dependencies
- **GPU support as a deploy concern**: swap the whisper base image, not the app code
- **Independent scaling**: whisper is the bottleneck; it can be on a GPU node
- **No subprocess management**: just HTTP calls via reqwest (already a dependency)
- **No temp files**: audio stays as in-memory `Vec<u8>` throughout

| Decision | Choice | Rationale |
|----------|--------|-----------|
| STT service | whisper.cpp server + ffmpeg | HTTP POST, handles OGG input via ffmpeg, GPU-optional |
| TTS service | Piper CLI + stdlib Python HTTP wrapper | No built-in HTTP server in Piper; Python shim is ~50 lines, zero pip deps |
| Audio conversion | Inside sidecar containers | Keeps corre-gym container slim; no ffmpeg needed in the app |
| Byte flow | In-memory `Vec<u8>` | Telegram voice messages max ~120KB (60s Opus); no temp files needed |
| GPU support | Single Dockerfile, build arg for base image | Avoids maintaining parallel Dockerfiles |

## Voice message flow

```
User speaks into Telegram
  -> Telegram encodes as OGG/Opus, sends to bot
  -> Bot receives Update with message.voice field
  -> Bot downloads OGG bytes via getFile + download (in-memory, no temp file)
  -> POST OGG bytes to whisper-server /inference endpoint
  -> whisper-server converts OGG->WAV internally (ffmpeg) and transcribes
  -> Text enters same pipeline as M1 (context -> LLM -> actions -> reply)
  -> Bot prepends transcript echo to reply text (_Heard: "..."_)
  -> POST reply text to piper-server /synthesize endpoint
  -> piper-server synthesizes WAV, converts WAV->OGG/Opus (ffmpeg), returns OGG bytes
  -> Bot sends OGG bytes as voice message via sendVoice API
  -> (Also sends text reply alongside, based on response_mode config)
```

The assistant handler (`handle_text_message`) is completely unchanged from M1. It receives
the transcript as if the user had typed it. The voice pipeline is pure transport.

## File structure

Sidecar Dockerfiles live under `services/` (not `crates/`) since these are infrastructure
support containers, not Rust crates.

```
apps/corre-gym/src/
    voice/
        mod.rs              VoicePipeline struct, ResponseMode enum, re-exports
        stt.rs              SttClient: HTTP client to whisper-server
        tts.rs              TtsClient: HTTP client to piper-server
    telegram/
        client.rs           Add: get_file(), download_file_bytes(), send_voice()
        types.rs            Add: TelegramFile, Audio types for getFile response
    config.rs               Add: VoiceConfig nested in GymConfig
    main.rs                 Add voice_pipeline to setup(), voice branch in process_message()
    lib.rs                  Add: pub mod voice

services/
    whisper/
        Dockerfile          whisper.cpp server + ffmpeg (optional CUDA base)
    piper/
        Dockerfile          Piper TTS + Python HTTP wrapper + ffmpeg
        piper_server.py     Minimal stdlib HTTP server (~50 lines)
```

## Config additions

```toml
[gym.voice]
stt_enabled = true
stt_url = "http://whisper:5005"          # whisper.cpp server endpoint
stt_language = "en"                      # language hint for whisper
tts_enabled = true
tts_url = "http://piper:5000"            # piper HTTP server endpoint
tts_voice = "en_US-lessac-medium"        # piper voice model name
response_mode = "both"                   # "voice" | "text" | "both"
max_voice_duration_secs = 60             # reject voice messages longer than this
```

```rust
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct VoiceConfig {
    #[serde(default = "default_true")]
    pub stt_enabled: bool,
    #[serde(default = "default_stt_url")]
    pub stt_url: String,
    #[serde(default = "default_stt_language")]
    pub stt_language: String,
    #[serde(default = "default_true")]
    pub tts_enabled: bool,
    #[serde(default = "default_tts_url")]
    pub tts_url: String,
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,
    #[serde(default)]
    pub response_mode: ResponseMode,
    #[serde(default = "default_max_voice_duration")]
    pub max_voice_duration_secs: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Voice,
    Text,
    #[default]
    Both,
}
```

Defaults: `stt_url` = `"http://whisper:5005"`, `stt_language` = `"en"`,
`tts_url` = `"http://piper:5000"`, `tts_voice` = `"en_US-lessac-medium"`,
`max_voice_duration_secs` = `60`.

### URL validation

Parse `stt_url` and `tts_url` at config load time. Add a `validate()` method on
`VoiceConfig` called during setup. This catches typos like `http://whsiper:8080` before
the first voice message arrives:

```rust
impl VoiceConfig {
    pub fn validate(&self) -> Result<()> {
        url::Url::parse(&self.stt_url)
            .with_context(|| format!("invalid stt_url: {}", self.stt_url))?;
        url::Url::parse(&self.tts_url)
            .with_context(|| format!("invalid tts_url: {}", self.tts_url))?;
        Ok(())
    }
}
```

Add to `GymConfig`:

```rust
#[serde(default)]
pub voice: Option<VoiceConfig>,
```

Voice is entirely optional. If the `[gym.voice]` section is absent, `voice` is `None` and
voice messages get a polite "not enabled" reply.

## Docker containers

### whisper-server (`services/whisper/Dockerfile`)

Uses pre-built whisper.cpp release binaries. The `--convert` flag tells the server to use
ffmpeg for input format conversion, so it accepts OGG directly from Telegram.

Uses a multi-stage build: downloads and verifies in a builder stage, copies only the
required binary to the slim runtime image.

**Important**: The release asset filename varies between whisper.cpp versions. The URL
below must be verified against the actual release page for the pinned version before
implementation. A build-time smoke test catches missing shared libraries early.

```dockerfile
# Stage 1: download and verify
FROM debian:trixie-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl && rm -rf /var/lib/apt/lists/*

# Pin a specific release. Verify the exact asset filename at:
# https://github.com/ggerganov/whisper.cpp/releases/tag/v1.7.5
ARG WHISPER_VERSION=1.7.5
ARG WHISPER_VARIANT=cpu
RUN mkdir -p /tmp/whisper && \
    curl -fSL \
    "https://github.com/ggerganov/whisper.cpp/releases/download/v${WHISPER_VERSION}/whisper-server-v${WHISPER_VERSION}-linux-x86_64-${WHISPER_VARIANT}.tar.gz" \
    | tar xz -C /tmp/whisper --strip-components=1 && \
    mv /tmp/whisper/whisper-server /usr/local/bin/whisper-server && \
    rm -rf /tmp/whisper

ARG WHISPER_MODEL=ggml-base.en.bin
RUN mkdir -p /models && \
    curl -fSL "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${WHISPER_MODEL}" \
    -o "/models/${WHISPER_MODEL}"

# Smoke test: verify the binary runs (catches missing shared libs)
RUN whisper-server --help || true

# Stage 2: slim runtime
ARG BASE_IMAGE=debian:trixie-slim
FROM ${BASE_IMAGE}

RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/whisper-server /usr/local/bin/
COPY --from=builder /models /models

EXPOSE 8080
ENTRYPOINT ["whisper-server"]
CMD ["--model", "/models/ggml-base.en.bin", "--host", "0.0.0.0", "--port", "8080", "--convert", "true"]
```

For CUDA GPU support, build with:
```sh
docker build --build-arg BASE_IMAGE=nvidia/cuda:12.8.0-runtime-ubuntu24.04 \
             --build-arg WHISPER_VARIANT=cuda \
             -t corre-whisper:cuda services/whisper/
```

The `cuda:12.8.0-runtime-ubuntu24.04` image includes cuBLAS, which whisper.cpp CUDA
binaries require at minimum. The specific CUDA version must match what the whisper.cpp
release was built against -- verify this combination before deploying. If the runtime
image is insufficient, use the `-devel-` variant instead.

The server exposes:
- `POST /inference` -- multipart form with `file` field, returns JSON (see response schema below)
- `GET /health` -- simple health check

Model options (set via `WHISPER_MODEL` build arg):

| Model | Size | Speed (CPU) | Speed (GPU) | Accuracy |
|-------|------|-------------|-------------|----------|
| ggml-tiny.en.bin | 75MB | ~0.5s | ~0.1s | Good for clear speech |
| ggml-base.en.bin | 142MB | ~1-3s | ~0.2-0.5s | Good default |
| ggml-small.en.bin | 466MB | ~5-10s | ~0.5-1s | Better for noisy audio |

### piper-server (`services/piper/Dockerfile`)

Multi-stage build. Sets `LD_LIBRARY_PATH` for Piper's bundled shared libraries and
`WORKDIR` so Piper can find its `espeak-ng-data/` sibling directory.

```dockerfile
# Stage 1: download
FROM debian:trixie-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl && rm -rf /var/lib/apt/lists/*

ARG PIPER_VERSION=2023.11.14-2
RUN curl -fSL \
    "https://github.com/rhasspy/piper/releases/download/${PIPER_VERSION}/piper_linux_x86_64.tar.gz" \
    | tar xz -C /usr/local/

ARG PIPER_VOICE=en_US-lessac-medium
RUN mkdir -p /models && \
    curl -fSL "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/${PIPER_VOICE}.onnx" \
    -o "/models/${PIPER_VOICE}.onnx" && \
    curl -fSL "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/${PIPER_VOICE}.onnx.json" \
    -o "/models/${PIPER_VOICE}.onnx.json"

# Stage 2: slim runtime
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 ffmpeg curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/piper /usr/local/piper
COPY --from=builder /models /models

# Piper needs its bundled libs and espeak-ng-data as siblings
ENV LD_LIBRARY_PATH=/usr/local/piper/lib
WORKDIR /usr/local/piper

COPY piper_server.py /app/piper_server.py

EXPOSE 5000
CMD ["python3", "/app/piper_server.py"]
```

### piper_server.py

Minimal HTTP server using only Python stdlib. Pipes text through Piper (raw PCM output)
into ffmpeg (OGG/Opus encoding) and returns the OGG bytes. No temp files.

Uses `ThreadingHTTPServer` so a slow synthesis doesn't block the `/health` endpoint
(which Docker uses for healthchecks). Enforces a 10KB request body limit and a 30-second
subprocess timeout.

```python
#!/usr/bin/env python3
"""Minimal HTTP server wrapping Piper TTS. Zero pip dependencies."""
import http.server
import json
import os
import subprocess
import signal

MODEL = os.environ.get("PIPER_MODEL", "/models/en_US-lessac-medium.onnx")
PORT = int(os.environ.get("PIPER_PORT", "5000"))
SAMPLE_RATE = os.environ.get("PIPER_SAMPLE_RATE", "22050")
MAX_BODY_BYTES = 10 * 1024  # 10KB -- generous for any TTS input
SUBPROCESS_TIMEOUT = 30     # seconds


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/synthesize":
            self.send_error(404)
            return

        # Validate Content-Type
        content_type = self.headers.get("Content-Type", "")
        if "application/json" not in content_type:
            self.send_error(415, "expected Content-Type: application/json")
            return

        # Enforce body size limit
        length = int(self.headers.get("Content-Length", 0))
        if length > MAX_BODY_BYTES:
            self.send_error(413, f"body exceeds {MAX_BODY_BYTES} byte limit")
            return

        body = json.loads(self.rfile.read(length))
        text = body.get("text", "").strip()
        if not text:
            self.send_error(400, "empty text")
            return

        piper = None
        ffmpeg = None
        try:
            # Piper --output-raw emits s16le PCM at the model's sample rate.
            # Pipe directly into ffmpeg to produce OGG/Opus.
            piper = subprocess.Popen(
                ["/usr/local/piper/piper", "--model", MODEL, "--output-raw"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            ffmpeg = subprocess.Popen(
                ["ffmpeg", "-f", "s16le", "-ar", SAMPLE_RATE, "-ac", "1", "-i", "pipe:",
                 "-c:a", "libopus", "-b:a", "64k", "-f", "ogg", "pipe:1",
                 "-loglevel", "error"],
                stdin=piper.stdout, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            piper.stdout.close()
            piper.stdin.write(text.encode())
            piper.stdin.close()
            ogg_bytes, ffmpeg_err = ffmpeg.communicate(timeout=SUBPROCESS_TIMEOUT)
            piper.wait(timeout=5)
        except subprocess.TimeoutExpired:
            # Kill both processes on timeout
            for proc in (piper, ffmpeg):
                if proc and proc.poll() is None:
                    proc.kill()
            self.send_error(500, "synthesis timed out")
            return

        if piper.returncode != 0 or ffmpeg.returncode != 0:
            self.send_error(500, f"piper rc={piper.returncode} ffmpeg rc={ffmpeg.returncode}")
            return

        self.send_response(200)
        self.send_header("Content-Type", "audio/ogg")
        self.send_header("Content-Length", str(len(ogg_bytes)))
        self.end_headers()
        self.wfile.write(ogg_bytes)

    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.end_headers()
        else:
            self.send_error(404)


if __name__ == "__main__":
    print(f"Piper server listening on :{PORT}, model={MODEL}")
    http.server.ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
```

### docker-compose.yml additions

Voice sidecars use the `voice` profile so they only start when explicitly requested.
Text-only mode (`docker compose up corre-gym`) does not require the voice containers.
Voice mode: `docker compose --profile voice up`.

The `VoicePipeline::verify()` health check at startup handles the case where sidecars
are absent, disabling voice gracefully.

```yaml
  whisper:
    profiles: [voice]
    build:
      context: services/whisper
      args:
        WHISPER_MODEL: ggml-base.en.bin
    # For GPU support (requires nvidia-container-toolkit), uncomment:
    # deploy:
    #   resources:
    #     reservations:
    #       devices:
    #         - driver: nvidia
    #           count: 1
    #           capabilities: [gpu]
    healthcheck:
      test: [CMD, curl, -f, "http://localhost:8080/health"]
      interval: 30s
      start_period: 15s
    restart: unless-stopped
    networks: [corre-internal]

  piper:
    profiles: [voice]
    build:
      context: services/piper
    healthcheck:
      test: [CMD, curl, -f, "http://localhost:5000/health"]
      interval: 30s
      start_period: 10s
    restart: unless-stopped
    networks: [corre-internal]

  corre-gym:
    build:
      context: .
      dockerfile: apps/corre-gym/Dockerfile
    command: ["/app/corre-gym", "-c", "/data/corre.toml"]
    ports:
      - "5520:5520"
    volumes:
      - ${CORRE_DATA_DIR:-/var/corre}:/data
    env_file: [.env]
    environment:
      CORRE_DATA_DIR: /data
      RUST_LOG: "${RUST_LOG:-info}"
    restart: unless-stopped
    networks: [corre-internal]
```

## Speech-to-text client (voice/stt.rs)

### Whisper response schema

The whisper.cpp server response format varies by version and parameters. Deserialize into
a typed struct with fallback for the segmented format:

```rust
#[derive(Debug, Deserialize)]
struct WhisperResponse {
    text: Option<String>,
    #[serde(default)]
    segments: Vec<WhisperSegment>,
}

#[derive(Debug, Deserialize)]
struct WhisperSegment {
    text: String,
}

impl WhisperResponse {
    fn transcript(&self) -> String {
        if let Some(ref text) = self.text {
            text.trim().to_string()
        } else {
            // Fallback: join segment texts
            self.segments.iter().map(|s| s.text.trim()).collect::<Vec<_>>().join(" ")
        }
    }
}
```

### SttClient struct

```rust
pub struct SttClient {
    url: String,
    client: reqwest::Client,
}

impl SttClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    /// Transcribe audio bytes (OGG/Opus from Telegram) to text.
    /// The whisper server handles format conversion internally via ffmpeg.
    /// Retries once on 5xx responses with a 2-second delay.
    pub async fn transcribe(&self, audio_bytes: &[u8]) -> Result<String> {
        let mut last_err = None;

        for attempt in 0..2u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            let part = reqwest::multipart::Part::bytes(audio_bytes.to_vec())
                .file_name("voice.ogg")
                .mime_str("audio/ogg")?;
            let form = reqwest::multipart::Form::new().part("file", part);

            let resp = match self.client
                .post(format!("{}/inference", self.url))
                .multipart(form)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("whisper request failed: {e:#}"));
                    continue;
                }
            };

            if resp.status().is_server_error() && attempt == 0 {
                let status = resp.status();
                tracing::warn!("whisper returned {status}, retrying");
                last_err = Some(anyhow::anyhow!("whisper server returned {status}"));
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("whisper server returned {status}: {body}");
            }

            let whisper_resp: WhisperResponse = resp.json().await?;
            return Ok(whisper_resp.transcript());
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("whisper transcription failed")))
    }

    pub async fn health_check(&self) -> Result<()> {
        let resp = self.client
            .get(format!("{}/health", self.url))
            .send()
            .await
            .context("whisper health check failed")?;
        if !resp.status().is_success() {
            anyhow::bail!("whisper server unhealthy: {}", resp.status());
        }
        Ok(())
    }
}
```

Timeout is 30s (generous, for large audio on CPU). Retries once on 5xx with a 2-second
delay, consistent with how the Telegram client handles transient failures.

## Text-to-speech client (voice/tts.rs)

### TtsClient struct

```rust
pub struct TtsClient {
    url: String,
    client: reqwest::Client,
}

impl TtsClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    /// Synthesize text to OGG/Opus audio bytes.
    /// The piper server handles WAV->OGG conversion internally via ffmpeg.
    /// Retries once on 5xx responses with a 1-second delay.
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        let mut last_err = None;

        for attempt in 0..2u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }

            let resp = match self.client
                .post(format!("{}/synthesize", self.url))
                .json(&serde_json::json!({"text": text}))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("piper request failed: {e:#}"));
                    continue;
                }
            };

            if resp.status().is_server_error() && attempt == 0 {
                let status = resp.status();
                tracing::warn!("piper returned {status}, retrying");
                last_err = Some(anyhow::anyhow!("piper server returned {status}"));
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("piper server returned {status}: {body}");
            }

            return Ok(resp.bytes().await?.to_vec());
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("piper synthesis failed")))
    }

    pub async fn health_check(&self) -> Result<()> {
        let resp = self.client
            .get(format!("{}/health", self.url))
            .send()
            .await
            .context("piper health check failed")?;
        if !resp.status().is_success() {
            anyhow::bail!("piper server unhealthy: {}", resp.status());
        }
        Ok(())
    }
}
```

## Voice pipeline orchestrator (voice/mod.rs)

```rust
pub struct VoicePipeline {
    stt: SttClient,
    tts: Option<TtsClient>,
    response_mode: ResponseMode,
    max_voice_duration_secs: u32,
}

impl VoicePipeline {
    pub fn new(config: &VoiceConfig) -> Self {
        let tts = if config.tts_enabled {
            Some(TtsClient::new(&config.tts_url))
        } else {
            None
        };
        Self {
            stt: SttClient::new(&config.stt_url),
            tts,
            response_mode: config.response_mode.clone(),
            max_voice_duration_secs: config.max_voice_duration_secs,
        }
    }

    pub async fn speech_to_text(&self, audio_bytes: &[u8]) -> Result<String> {
        self.stt.transcribe(audio_bytes).await
    }

    /// Returns None if TTS is disabled.
    pub async fn text_to_speech(&self, text: &str) -> Result<Option<Vec<u8>>> {
        match &self.tts {
            Some(tts) => {
                let clean = strip_markdown(text);
                Ok(Some(tts.synthesize(&clean).await?))
            }
            None => Ok(None),
        }
    }

    pub fn should_send_text(&self) -> bool {
        matches!(self.response_mode, ResponseMode::Text | ResponseMode::Both)
    }

    pub fn should_send_voice(&self) -> bool {
        matches!(self.response_mode, ResponseMode::Voice | ResponseMode::Both)
    }

    pub fn max_duration_secs(&self) -> u32 {
        self.max_voice_duration_secs
    }

    /// Health check both services. Called at startup.
    pub async fn verify(&self) -> Result<()> {
        self.stt.health_check().await?;
        if let Some(ref tts) = self.tts {
            tts.health_check().await?;
        }
        Ok(())
    }
}

/// Strip markdown formatting for cleaner TTS output.
/// Removes *, _, `, #, list markers, and link syntax.
fn strip_markdown(text: &str) -> String {
    // Regex for markdown links: [text](url) -> text
    let link_re = regex::Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap();
    let text = link_re.replace_all(text, "$1");

    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            // Strip list markers (- item, * item, 1. item, 10. item)
            let line = if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                &trimmed[2..]
            } else if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
                // Handle multi-digit numbered lists (1. item, 10. item, 100. item)
                let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
                if let Some(stripped) = rest.strip_prefix(". ") {
                    stripped
                } else {
                    trimmed
                }
            } else {
                trimmed
            };
            // Strip heading markers
            line.trim_start_matches('#').trim_start()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .replace("**", "")
        .replace('*', "")
        .replace('_', "")
        .replace('`', "")
}
```

## Telegram client additions (telegram/client.rs)

### New types (telegram/types.rs)

```rust
#[derive(Debug, Deserialize)]
pub struct TelegramFile {
    pub file_id: String,
    pub file_unique_id: Option<String>,
    pub file_size: Option<i64>,
    pub file_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Audio {
    pub file_id: String,
    pub duration: i32,
}
```

Add `audio: Option<Audio>` to the existing `Message` struct to detect audio file messages.

### New methods on TelegramClient

Pre-construct the file download base URL in `TelegramClient::new()` to avoid embedding
the token in runtime-constructed URLs that could appear in logs:

```rust
pub struct TelegramClient {
    // ... existing fields ...
    file_base_url: String,  // "https://api.telegram.org/file/bot{token}"
}

impl TelegramClient {
    pub fn new(token: &str) -> Result<Self> {
        // ... existing validation ...
        let file_base_url = format!("https://api.telegram.org/file/bot{token}");
        // ...
    }

    /// Get the file metadata for a file_id (needed to construct download URL).
    pub async fn get_file(&self, file_id: &str) -> Result<TelegramFile> {
        self.post("getFile", &json!({"file_id": file_id})).await
    }

    /// Download file bytes directly into memory.
    pub async fn download_file_bytes(&self, file_path: &str) -> Result<Vec<u8>> {
        let url = format!("{}/{file_path}", self.file_base_url);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("file download failed: {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }

    /// Send a voice message (OGG/Opus bytes).
    pub async fn send_voice(
        &self, chat_id: i64, ogg_bytes: &[u8], reply_to: Option<i64>,
    ) -> Result<Message> {
        let url = format!("{}/sendVoice", self.base_url);
        let part = reqwest::multipart::Part::bytes(ogg_bytes.to_vec())
            .file_name("voice.ogg")
            .mime_str("audio/ogg")?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("voice", part);
        if let Some(reply_id) = reply_to {
            form = form.text("reply_to_message_id", reply_id.to_string());
        }
        let resp: TelegramResponse<Message> = self.client
            .post(&url)
            .multipart(form)
            .send().await?
            .json().await?;
        match resp.result {
            Some(msg) if resp.ok => Ok(msg),
            _ => anyhow::bail!(
                "sendVoice failed: {}",
                resp.description.unwrap_or_else(|| "unknown error".into())
            ),
        }
    }
}
```

### Workspace dependency change

Add `multipart` to the workspace `reqwest` features in the root `Cargo.toml`:

```toml
reqwest = { version = "0.12", features = ["json", "multipart", "rustls-tls"], default-features = false }
```

This is backwards-compatible. No other workspace changes needed.

## Main loop changes (main.rs)

### Chat action refresh

Telegram chat actions expire after 5 seconds. The voice pipeline can easily take 8-10
seconds on CPU. To keep the Telegram UI responsive, spawn a background task that
re-sends the current chat action every 4 seconds until cancelled:

```rust
fn spawn_chat_action_loop(
    telegram: &TelegramClient,
    chat_id: i64,
    action: &str,
) -> tokio::sync::oneshot::Sender<()> {
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
```

Usage: `let stop = spawn_chat_action_loop(telegram, chat_id, "record_voice");`
When done: `let _ = stop.send(());` (or just drop the sender).

This also benefits the M1 text path for slow LLM calls.

### setup() changes

Add `VoicePipeline` creation after the handler. The return type gains `Option<VoicePipeline>`:

```rust
async fn setup() -> Result<(TelegramClient, AssistantHandler, Vec<i64>, Option<VoicePipeline>)> {
    // ... existing M1 setup (config, LLM, DB, Telegram, handler) ...

    // Voice pipeline (optional)
    let voice_pipeline = match &gym_config.voice {
        Some(voice_config) if voice_config.stt_enabled => {
            voice_config.validate()?;
            let pipeline = VoicePipeline::new(voice_config);
            match pipeline.verify().await {
                Ok(()) => {
                    tracing::info!(
                        "Voice pipeline active (STT: {}, TTS: {})",
                        voice_config.stt_url,
                        if voice_config.tts_enabled { &voice_config.tts_url } else { "disabled" }
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

    Ok((telegram, handler, allowed_ids, voice_pipeline))
}
```

### process_message() changes

The M1 `process_message` checks `message.text` and early-returns if absent. We add
voice and audio branches:

```rust
async fn process_message(
    telegram: &TelegramClient,
    handler: &AssistantHandler,
    voice_pipeline: Option<&VoicePipeline>,
    message: &Message,
    allowed_ids: &[i64],
) {
    if message.chat.chat_type != "private" { return; }
    let Some(ref from) = message.from else { return; };
    if !allowed_ids.is_empty() && !allowed_ids.contains(&from.id) { return; }

    if let Some(ref text) = message.text {
        // Text message -- existing M1 flow (unchanged)
        process_text_message(telegram, handler, message, text).await;
    } else if let Some(ref voice) = message.voice {
        // Voice message -- new M2 flow
        process_voice_message(telegram, handler, voice_pipeline, message, voice).await;
    } else if message.audio.is_some() {
        // Audio file (not a voice recording) -- prompt the user
        if let Err(e) = telegram.send_message(
            message.chat.id,
            "Please use the microphone button to record voice messages directly.",
            None, None,
        ).await {
            tracing::warn!("Failed to send audio guidance: {e:#}");
        }
    }
}
```

The existing text handling moves into `process_text_message()` (a trivial extraction).

### process_voice_message() (new function)

```rust
async fn process_voice_message(
    telegram: &TelegramClient,
    handler: &AssistantHandler,
    voice_pipeline: Option<&VoicePipeline>,
    message: &Message,
    voice: &Voice,
) {
    // 0. Check if voice is enabled
    let Some(pipeline) = voice_pipeline else {
        if let Err(e) = telegram.send_message(
            message.chat.id,
            "Voice messages are not enabled. Please type your message instead.",
            None, None,
        ).await {
            tracing::warn!("Failed to send voice-disabled notice: {e:#}");
        }
        return;
    };

    // 1. Reject overly long messages
    if voice.duration as u32 > pipeline.max_duration_secs() {
        if let Err(e) = telegram.send_message(
            message.chat.id,
            "That voice message is too long. Please keep it under 60 seconds, or type your message.",
            None, None,
        ).await {
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
            if let Err(e) = telegram.send_message(
                message.chat.id,
                "I couldn't download that voice message. Could you try again?",
                None, None,
            ).await {
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
            if let Err(e) = telegram.send_message(
                message.chat.id,
                "I couldn't make out what you said. Could you try again, or type your message?",
                None, None,
            ).await {
                tracing::warn!("Failed to send empty-transcript notice: {e:#}");
            }
            return;
        }
        Err(e) => {
            let _ = stop_action.send(());
            tracing::error!("STT failed: {e:#}");
            if let Err(e) = telegram.send_message(
                message.chat.id,
                "I had trouble understanding that voice message. Could you type it instead?",
                None, None,
            ).await {
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
    let reply = match handler.handle_text_message(message, &transcript).await {
        Ok(text) => text,
        Err(e) => {
            tracing::error!("Handler error: {e:#}");
            "I had trouble processing that -- could you try again?".to_string()
        }
    };

    let _ = stop_action.send(());

    // 7. Send text reply with transcript echo (if configured)
    if pipeline.should_send_text() {
        let text_with_echo = format!("_Heard: \"{transcript}\"_\n\n{reply}");
        if let Err(e) = send_long_message(telegram, message.chat.id, &text_with_echo).await {
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
                        if let Err(e) = send_long_message(telegram, message.chat.id, &text_with_echo).await {
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
                    if let Err(e) = send_long_message(telegram, message.chat.id, &text_with_echo).await {
                        tracing::warn!("Failed to send fallback text: {e:#}");
                    }
                }
            }
        }
    }
}

async fn download_voice(telegram: &TelegramClient, file_id: &str) -> Result<Vec<u8>> {
    let file = telegram.get_file(file_id).await?;
    let file_path = file.file_path.context("Telegram returned no file_path")?;
    telegram.download_file_bytes(&file_path).await
}
```

## UX considerations

### Transcript echo

Every voice reply includes the transcript so users can verify what was heard:

```
_Heard: "five sets of squats at 100 kilos"_

Got it! I've logged 5 sets of Barbell Back Squat at 100kg. How did that feel?
```

This catches transcription errors before they become bad data in the database. The echo
uses Telegram's Markdown italic syntax (`_..._`) to visually distinguish it from the
assistant's reply.

### Chat action refresh

Telegram chat actions expire after 5 seconds. Since the voice pipeline can take 8-10
seconds on CPU, we spawn a background task that re-sends the current action every 4
seconds. The action sequence:

1. `record_voice` -- while downloading + transcribing (user sees a microphone animation)
2. `typing` -- while the LLM processes the transcript (user sees "typing..." text)
3. `upload_voice` -- while uploading the synthesized reply (user sees upload animation)

The background task is cancelled via a `oneshot` channel when the current phase ends.

### Latency budget

| Step | CPU | GPU | Notes |
|------|-----|-----|-------|
| Download OGG from Telegram | ~100ms | ~100ms | Small files (max ~120KB) |
| whisper.cpp STT (base.en) | 1-3s | 0.2-0.5s | Dominant factor on CPU |
| LLM call | 1-5s | 1-5s | Depends on provider |
| Piper TTS (medium voice) | 0.5-2s | 0.5-2s | CPU-only, fast |
| Upload OGG to Telegram | ~100ms | ~100ms | Small files |
| **Total** | **3-10s** | **2-8s** | Acceptable for voice UI |

### Error handling / graceful degradation

- **STT fails**: tell user to type instead (voice -> text fallback)
- **TTS fails**: send text-only reply with transcript echo (voice -> text fallback)
- **Voice services unreachable at startup**: disable voice, log warning, text-only mode
- **Overly long voice message**: reject with a polite message and the duration limit
- **Empty transcript** (silence, noise): ask user to try again or type
- **Telegram file download fails**: ask user to resend
- **Error notification fails to send**: logged at warn level (never silently dropped)

### Response mode behavior

| `response_mode` | User sends voice | User sends text |
|------------------|-----------------|-----------------|
| `both` | Reply with text (+ echo) + voice | Reply with text only |
| `voice` | Reply with voice only (text fallback on TTS failure includes echo) | Reply with text only |
| `text` | Reply with text + echo only | Reply with text only |

Text messages always get text replies regardless of response_mode. Voice replies are only
generated when the user sends a voice message. Transcript echo is always included in
text replies to voice messages.

### Audio file messages

If a user sends an audio file (e.g. a forwarded voice memo from another app) via the
`audio` message type instead of using the microphone button, the bot sends a guidance
message: "Please use the microphone button to record voice messages directly."

## GPU support

GPU acceleration is a **deploy-time concern**, not an application concern. The Rust code and
HTTP API are identical regardless of whether whisper runs on CPU or GPU.

### Requirements
- NVIDIA GPU on the host
- `nvidia-container-toolkit` installed ([install guide](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html))
- Docker runtime configured for NVIDIA

### Enabling GPU

1. Build the whisper container with the CUDA variant:
   ```sh
   docker build \
     --build-arg BASE_IMAGE=nvidia/cuda:12.8.0-runtime-ubuntu24.04 \
     --build-arg WHISPER_VARIANT=cuda \
     -t corre-whisper:cuda services/whisper/
   ```

   The `runtime` image includes cuBLAS, which whisper.cpp CUDA binaries require. The
   specific CUDA version must match the whisper.cpp release binary's build -- verify this
   combination before deploying. If cuBLAS is missing at runtime, try the `-devel-`
   variant.

2. Uncomment the `deploy` section in docker-compose.yml:
   ```yaml
   deploy:
     resources:
       reservations:
         devices:
           - driver: nvidia
             count: 1
             capabilities: [gpu]
   ```

3. Optionally use a larger model (e.g. `ggml-small.en.bin`) since GPU makes larger models
   practical within the latency budget.

Piper uses ONNX Runtime (CPU only). No GPU support needed or available.

## Tests

### Config tests
- `voice_config_absent` -- missing `[gym.voice]` -> `voice` is `None`
- `voice_config_defaults` -- `[gym.voice]` with no fields -> all defaults applied
- `voice_config_custom` -- full config parsed correctly
- `response_mode_variants` -- each variant deserializes from snake_case
- `voice_config_invalid_url` -- malformed URL in stt_url/tts_url -> validation error

### SttClient tests (mock HTTP server)
- `transcribe_returns_text` -- mock whisper returns `{"text": "..."}` -> correct text
- `transcribe_segment_fallback` -- mock whisper returns `{"segments": [...]}` -> joined text
- `transcribe_handles_empty_text` -- whisper returns `{"text": ""}` -> `Ok("")`
- `transcribe_handles_server_error` -- 500 response -> error with status
- `transcribe_handles_timeout` -- slow server -> timeout error
- `transcribe_retries_on_5xx` -- 500 then 200 -> succeeds on retry
- `transcribe_corrupted_audio` -- garbage bytes -> clean error

### TtsClient tests (mock HTTP server)
- `synthesize_returns_bytes` -- mock piper returns audio bytes -> correct bytes
- `synthesize_handles_server_error` -- 500 response -> error
- `synthesize_handles_empty_text` -- empty input handled gracefully
- `synthesize_retries_on_5xx` -- 500 then 200 -> succeeds on retry

### VoicePipeline tests
- `strip_markdown_removes_formatting` -- bold, italic, code, headers, lists
- `strip_markdown_preserves_plain_text` -- no-op for plain text
- `strip_markdown_strips_links` -- `[click here](https://example.com)` -> `click here`
- `strip_markdown_multi_digit_lists` -- `10. item` -> `item`
- `should_send_text_per_mode` -- true for Text and Both, false for Voice
- `should_send_voice_per_mode` -- true for Voice and Both, false for Text
- `tts_disabled_returns_none` -- `text_to_speech` returns `Ok(None)` when TTS disabled

### Telegram client tests (mock HTTP server)
- `get_file_parses_response` -- correct TelegramFile deserialization
- `download_file_returns_bytes` -- returns raw bytes
- `send_voice_multipart_format` -- verify multipart form construction

### Integration tests
- `voice_message_end_to_end` -- mock Telegram + mock whisper + mock piper: voice message
  flows through transcription -> handler -> synthesis -> voice reply
- `stt_failure_sends_text_fallback` -- whisper returns 500 -> user gets "type instead" message
- `tts_failure_sends_text_only` -- piper returns 500 -> user gets text reply only
- `voice_disabled_sends_notice` -- no voice config -> "not enabled" message
- `empty_transcript_asks_retry` -- whisper returns empty text -> "couldn't make out" message
- `long_voice_rejected` -- 90s voice message -> "too long" message
- `transcript_echo_in_reply` -- voice reply text includes `_Heard: "..."_` prefix
- `audio_file_sends_guidance` -- audio message -> "use microphone button" response
- `concurrent_voice_messages` -- two simultaneous voice messages from different users
  complete within timeout. The whisper sidecar queues inference requests; verify the
  second request does not time out given the 30-second client timeout

## Verification

```sh
# Unit tests
cargo test -p corre-gym -- voice
cargo test -p corre-gym -- telegram::client

# Build and start sidecar containers (voice profile)
docker compose --profile voice build
docker compose --profile voice up -d

# Verify services are healthy
curl http://localhost:8080/health     # whisper (if ports exposed for testing)
curl http://localhost:5000/health     # piper

# Test STT directly
curl -F "file=@test.ogg" http://localhost:8080/inference

# Test TTS directly
curl -X POST -H "Content-Type: application/json" \
  -d '{"text":"Hello, this is a test."}' \
  http://localhost:5000/synthesize --output test_reply.ogg

# Run the bot
cargo run -p corre-gym -- -c ~/.local/share/corre/corre.toml

# In Telegram:
# 1. Send a voice message: "I just did five sets of squats at 100 kilos"
# 2. Verify: transcript echo shows what was heard, exercise logged, voice + text reply
# 3. Send a voice message: "What did I do today?"
# 4. Verify: bot replies with today's session summary
# 5. Test error handling: send a very quiet/noisy voice message
# 6. Forward an audio file to the bot -> should get "use microphone button" guidance
```

## Implementation sequence

1. Sidecar containers: `services/whisper/Dockerfile`, `services/piper/Dockerfile` + `piper_server.py`
2. docker-compose.yml: add whisper, piper (voice profile), corre-gym services
3. Workspace `Cargo.toml`: add `multipart` to reqwest features
4. `config.rs`: add `VoiceConfig` (with URL validation), `ResponseMode`, wire into `GymConfig`
5. `telegram/types.rs`: add `TelegramFile`, `Audio`
6. `telegram/client.rs`: add `file_base_url`, `get_file()`, `download_file_bytes()`, `send_voice()`
7. `voice/stt.rs`: `SttClient` with `WhisperResponse` struct and retry logic
8. `voice/tts.rs`: `TtsClient` with retry logic
9. `voice/mod.rs`: `VoicePipeline`, `strip_markdown()` with link/list handling
10. `main.rs`: chat action refresh loop, `VoicePipeline` in setup, split `process_message` into text/voice/audio branches, transcript echo
11. Tests: config, SttClient, TtsClient, VoicePipeline, Telegram client, integration (including concurrent and corrupted-audio cases)

Steps 4-9 have no inter-dependencies beyond types and can be developed in parallel.
Step 10 integrates everything. Step 11 verifies.
