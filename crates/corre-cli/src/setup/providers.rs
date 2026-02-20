/// LLM provider choices presented during setup.
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        label: "Venice.ai (recommended — privacy-focused, no data retention)",
        key: "venice",
        base_url: "https://api.venice.ai/api/v1",
        default_model: "zai-org-glm-4.7-flash",
        api_key_env: "VENICE_API_KEY",
        needs_api_key: true,
        signup_url: Some("https://venice.ai/sign-up"),
        guidance: "\
Venice.ai is a privacy-focused LLM provider — your data is never stored or used for training.

  1. Create an account at venice.ai
  2. Navigate to Settings > API Keys
  3. Click \"Generate API Key\" and copy it",
    },
    ProviderInfo {
        label: "Ollama (fully local, no API key needed)",
        key: "ollama",
        base_url: "http://localhost:11434/v1",
        default_model: "llama3.1",
        api_key_env: "OLLAMA_API_KEY",
        needs_api_key: false,
        signup_url: Some("https://ollama.com/download"),
        guidance: "\
Ollama runs models entirely on your machine — no data leaves your network.

  Install from ollama.com/download, then pull a model:  ollama pull llama3.1",
    },
    ProviderInfo {
        label: "OpenAI",
        key: "openai",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o-mini",
        api_key_env: "OPENAI_API_KEY",
        needs_api_key: true,
        signup_url: Some("https://platform.openai.com/api-keys"),
        guidance: "\
  1. Go to platform.openai.com/api-keys
  2. Click \"Create new secret key\" and copy it",
    },
    ProviderInfo {
        label: "Other OpenAI-compatible endpoint",
        key: "custom",
        base_url: "",
        default_model: "",
        api_key_env: "LLM_API_KEY",
        needs_api_key: true,
        signup_url: None,
        guidance: "Enter the base URL and model name for your OpenAI-compatible API.",
    },
];

pub struct ProviderInfo {
    pub label: &'static str,
    pub key: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    pub api_key_env: &'static str,
    pub needs_api_key: bool,
    pub signup_url: Option<&'static str>,
    pub guidance: &'static str,
}
