use anyhow::Context as _;
use corre_core::config::AppLlmConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GymConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    pub telegram_bot_token: String,
    /// Telegram user IDs allowed to use the bot. Empty = allow all (dev mode).
    #[serde(default)]
    pub telegram_allowed_ids: Vec<i64>,
    #[serde(default = "default_timezone")]
    pub default_timezone: String,
    #[serde(default = "default_history_limit")]
    pub conversation_history_limit: usize,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_max_message_length")]
    pub max_message_length: usize,
    #[serde(default = "default_session_timeout_hours")]
    pub session_timeout_hours: u32,
    #[serde(default)]
    pub llm: Option<AppLlmConfig>,
    #[serde(default)]
    pub voice: Option<VoiceConfig>,
}

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
    /// Piper speaker name for multi-speaker models (e.g. "prudence", "spike", "obadiah", "poppy").
    /// Empty string = use model default.
    #[serde(default)]
    pub tts_speaker: String,
    /// Speaking speed multiplier. 1.0 = normal, 1.5 = 50% faster, 0.75 = 25% slower.
    /// Range: 0.25 to 4.0.
    #[serde(default = "default_tts_speed")]
    pub tts_speed: f32,
    #[serde(default)]
    pub response_mode: ResponseMode,
    #[serde(default = "default_max_voice_duration")]
    pub max_voice_duration_secs: u32,
}

impl VoiceConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        url::Url::parse(&self.stt_url).with_context(|| format!("invalid stt_url: {}", self.stt_url))?;
        url::Url::parse(&self.tts_url).with_context(|| format!("invalid tts_url: {}", self.tts_url))?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Voice,
    Text,
    #[default]
    Both,
}

fn default_true() -> bool {
    true
}

fn default_stt_url() -> String {
    "http://whisper:5005".into()
}

fn default_stt_language() -> String {
    "en".into()
}

fn default_tts_url() -> String {
    "http://piper:5000".into()
}

fn default_tts_voice() -> String {
    "en_GB-semaine-medium".into()
}

fn default_tts_speed() -> f32 {
    1.0
}

fn default_max_voice_duration() -> u32 {
    60
}

fn default_bind() -> String {
    "127.0.0.1:5520".into()
}

fn default_timezone() -> String {
    "Europe/London".into()
}

fn default_history_limit() -> usize {
    20
}

fn default_db_path() -> String {
    "gym-tracker.db".into()
}

fn default_max_message_length() -> usize {
    2000
}

fn default_session_timeout_hours() -> u32 {
    4
}

impl GymConfig {
    pub fn from_toml_table(table: Option<&toml::Value>) -> anyhow::Result<Self> {
        table.cloned().ok_or_else(|| anyhow::anyhow!("missing [gym] section in corre.toml")).and_then(|v| v.try_into().map_err(Into::into))
    }

    /// Resolve `${VAR}` references in secret and URL fields.
    pub fn resolve_secrets(&mut self) -> anyhow::Result<()> {
        self.telegram_bot_token =
            corre_core::secret::resolve_value(&self.telegram_bot_token).context("resolving TELEGRAM_GYM_BOT_TOKEN")?;
        if let Some(ref mut voice) = self.voice {
            voice.stt_url = corre_core::secret::resolve_value(&voice.stt_url).context("resolving stt_url")?;
            voice.tts_url = corre_core::secret::resolve_value(&voice.tts_url).context("resolving tts_url")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_gym_toml(voice_section: &str) -> toml::Value {
        let s = format!(
            r#"
            telegram_bot_token = "123456:ABC"
            {voice_section}
            "#
        );
        toml::from_str(&s).unwrap()
    }

    #[test]
    fn voice_config_absent() {
        let val = minimal_gym_toml("");
        let config: GymConfig = val.try_into().unwrap();
        assert!(config.voice.is_none());
    }

    #[test]
    fn voice_config_defaults() {
        let val = minimal_gym_toml("[voice]");
        let config: GymConfig = val.try_into().unwrap();
        let voice = config.voice.unwrap();
        assert!(voice.stt_enabled);
        assert_eq!(voice.stt_url, "http://whisper:5005");
        assert_eq!(voice.stt_language, "en");
        assert!(voice.tts_enabled);
        assert_eq!(voice.tts_url, "http://piper:5000");
        assert_eq!(voice.tts_voice, "en_GB-semaine-medium");
        assert_eq!(voice.tts_speaker, "");
        assert!((voice.tts_speed - 1.0).abs() < f32::EPSILON);
        assert_eq!(voice.response_mode, ResponseMode::Both);
        assert_eq!(voice.max_voice_duration_secs, 60);
    }

    #[test]
    fn voice_config_custom() {
        let val = minimal_gym_toml(
            r#"
            [voice]
            stt_enabled = false
            stt_url = "http://localhost:9090"
            stt_language = "de"
            tts_enabled = false
            tts_url = "http://localhost:9091"
            tts_voice = "de_DE-thorsten-medium"
            tts_speaker = "spike"
            tts_speed = 1.3
            response_mode = "voice"
            max_voice_duration_secs = 30
            "#,
        );
        let config: GymConfig = val.try_into().unwrap();
        let voice = config.voice.unwrap();
        assert!(!voice.stt_enabled);
        assert_eq!(voice.stt_url, "http://localhost:9090");
        assert_eq!(voice.stt_language, "de");
        assert!(!voice.tts_enabled);
        assert_eq!(voice.tts_url, "http://localhost:9091");
        assert_eq!(voice.tts_voice, "de_DE-thorsten-medium");
        assert_eq!(voice.tts_speaker, "spike");
        assert!((voice.tts_speed - 1.3).abs() < 0.01);
        assert_eq!(voice.response_mode, ResponseMode::Voice);
        assert_eq!(voice.max_voice_duration_secs, 30);
    }

    #[test]
    fn response_mode_variants() {
        let v: ResponseMode = serde_json::from_str(r#""voice""#).unwrap();
        assert_eq!(v, ResponseMode::Voice);
        let v: ResponseMode = serde_json::from_str(r#""text""#).unwrap();
        assert_eq!(v, ResponseMode::Text);
        let v: ResponseMode = serde_json::from_str(r#""both""#).unwrap();
        assert_eq!(v, ResponseMode::Both);
    }

    #[test]
    fn voice_config_invalid_url() {
        let voice = VoiceConfig {
            stt_enabled: true,
            stt_url: "not-a-url".into(),
            stt_language: "en".into(),
            tts_enabled: true,
            tts_url: "http://piper:5000".into(),
            tts_voice: "en_GB-semaine-medium".into(),
            tts_speaker: String::new(),
            tts_speed: 1.0,
            response_mode: ResponseMode::Both,
            max_voice_duration_secs: 60,
        };
        assert!(voice.validate().is_err());
    }

    #[test]
    fn voice_config_valid_urls() {
        let voice = VoiceConfig {
            stt_enabled: true,
            stt_url: "http://whisper:5005".into(),
            stt_language: "en".into(),
            tts_enabled: true,
            tts_url: "http://piper:5000".into(),
            tts_voice: "en_GB-semaine-medium".into(),
            tts_speaker: "prudence".into(),
            tts_speed: 1.2,
            response_mode: ResponseMode::Both,
            max_voice_duration_secs: 60,
        };
        assert!(voice.validate().is_ok());
    }
}
