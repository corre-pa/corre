//! Standalone `corre-news` binary -- serves only the newspaper web interface.
//!
//! Designed for the Docker `news` container: no dashboard, no scheduler, no
//! capability execution. Just the newspaper, archive, and search.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use corre_core::config::CorreConfig;
use corre_news::{NewsConfig, archive::Archive, cache::EditionCache, search::SearchIndex, server::AppState};

#[derive(Parser)]
#[command(name = "corre-news", about = "CorreNews standalone web server")]
struct Cli {
    /// Path to corre.toml config file
    #[arg(short, long, default_value = default_config_path())]
    config: PathBuf,
}

fn default_config_path() -> &'static str {
    // Matches corre-cli's default; overridden by -c in Docker.
    "corre.toml"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load .env from the data dir (best-effort, same as corre-cli)
    let default_data_dir = dirs::data_dir().map(|d| d.join("corre")).unwrap_or_else(|| PathBuf::from("."));
    let _ = dotenvy::from_filename_override(default_data_dir.join(".env")).ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    let config = CorreConfig::load(&cli.config).context("loading config")?;
    let data_dir = config.data_dir();

    let archive = Archive::new(&data_dir);
    let cache = Arc::new(EditionCache::load(archive));
    let search = SearchIndex::open_readonly(&data_dir)?;

    let news = NewsConfig::from_toml_table(Some(&config.news));
    let addr: std::net::SocketAddr = news.bind.parse().context("parsing news.bind address")?;

    // Periodically rescan the filesystem for new editions written by corre-core
    let refresh_cache = cache.clone();
    let state = Arc::new(AppState { cache, search, config_path: cli.config, config: Arc::new(RwLock::new(config)), data_dir });
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // skip the immediate first tick
        loop {
            interval.tick().await;
            refresh_cache.refresh().await;
        }
    });

    tracing::info!("CorreNews listening on http://{addr}");
    corre_news::server::serve(state, addr).await
}
