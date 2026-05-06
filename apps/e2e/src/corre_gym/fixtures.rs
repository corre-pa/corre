//! Loads the test-owned `corre.toml` and `.env` from `apps/e2e/tests/fixtures/`.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use corre_core::config::CorreConfig;

/// Absolute path to `apps/e2e/tests/fixtures/`.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

/// Load the test corre.toml, applying `${VAR}` interpolation to `[llm]` connection fields.
///
/// `CorreConfig::load` deserialises raw TOML; `${VAR}` references in `api_key` and the
/// gym `telegram_bot_token` are resolved lazily by their respective consumers. The
/// `base_url` and `model` fields are never resolved upstream, so we expand them here so
/// fixture files can use `${CORRE_TEST_LLM_*}` placeholders without manual sed-patching.
pub fn load_test_config() -> anyhow::Result<CorreConfig> {
    let dir = fixtures_dir();
    let _ = dotenvy::from_filename(dir.join(".env"));
    let mut cfg = CorreConfig::load(&dir.join("corre.toml")).context("loading test corre.toml")?;
    cfg.llm.base_url = corre_core::secret::resolve_value(&cfg.llm.base_url).context("resolving LLM base_url")?;
    cfg.llm.model = corre_core::secret::resolve_value(&cfg.llm.model).context("resolving LLM model")?;
    if cfg.llm.base_url.trim().is_empty() {
        anyhow::bail!("CORRE_TEST_LLM_BASE_URL is empty. Copy apps/e2e/tests/fixtures/.env.example to .env and fill it in.");
    }
    if cfg.llm.model.trim().is_empty() {
        anyhow::bail!("CORRE_TEST_LLM_MODEL is empty");
    }
    Ok(cfg)
}
