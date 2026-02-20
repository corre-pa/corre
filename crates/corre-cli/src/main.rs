use anyhow::Context;
use clap::{Parser, Subcommand};
use corre_core::capability::CapabilityContext;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "corre", about = "Personal AI task scheduler and newspaper")]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "corre.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the full daemon (scheduler + web server)
    Run,
    /// Run a single capability immediately and exit
    RunNow {
        /// Name of the capability to run
        capability: String,
    },
    /// Start only the web server
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = corre_core::config::CorreConfig::load(&cli.config)
        .with_context(|| format!("Failed to load config from {}", cli.config.display()))?;

    let data_dir = config.data_dir();
    std::fs::create_dir_all(&data_dir)?;

    let log_dir = data_dir.join("capabilities_logs");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "capability.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| config.general.log_level.parse().unwrap_or_default());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(tracing_subscriber::fmt::layer().json().with_ansi(false).with_writer(non_blocking))
        .init();

    match cli.command {
        Commands::Run => cmd_run(config).await,
        Commands::RunNow { capability } => cmd_run_now(config, &capability).await,
        Commands::Serve => cmd_serve(config).await,
    }
}

async fn cmd_run(config: corre_core::config::CorreConfig) -> anyhow::Result<()> {
    // Shared cache for both web server and scheduler
    let data_dir = config.data_dir();
    let archive = corre_news::archive::Archive::new(&data_dir);
    let cache = Arc::new(corre_news::cache::EditionCache::load(archive));

    // Start web server in background
    let web_config = config.clone();
    let web_cache = cache.clone();
    let web_handle = tokio::spawn(async move { start_web_server_with_cache(&web_config, web_cache).await });

    // Start scheduler
    let mut scheduler = corre_core::scheduler::Scheduler::new().await?;
    let registry = Arc::new(corre_capabilities::registry::CapabilityRegistry::from_config(&config.capabilities));

    for cap_config in config.capabilities.iter().filter(|c| c.enabled) {
        let cap_name = cap_config.name.clone();
        let schedule = cap_config.schedule.clone();
        let config = config.clone();
        let registry = registry.clone();
        let cache = cache.clone();

        tracing::info!("Scheduling capability `{cap_name}` with cron `{schedule}`");

        let callback: Box<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync> =
            Box::new(move || {
                let cap_name = cap_name.clone();
                let config = config.clone();
                let registry = registry.clone();
                let cache = cache.clone();
                Box::pin(async move {
                    if let Err(e) = execute_capability_with_cache(&config, &registry, &cap_name, &cache).await {
                        tracing::error!("Capability `{cap_name}` failed: {e:#}");
                    }
                })
            });

        scheduler.add_async_job(&schedule, callback).await?;
    }

    scheduler.start().await?;
    tracing::info!("Scheduler started. Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down...");
    scheduler.shutdown().await?;

    web_handle.abort();
    Ok(())
}

async fn cmd_run_now(config: corre_core::config::CorreConfig, capability_name: &str) -> anyhow::Result<()> {
    let registry = corre_capabilities::registry::CapabilityRegistry::from_config(&config.capabilities);
    // Load cache to get seen_urls for cross-edition dedup, but store via archive directly
    let data_dir = config.data_dir();
    let archive = corre_news::archive::Archive::new(&data_dir);
    let cache = corre_news::cache::EditionCache::load(archive);
    let seen_urls = cache.seen_urls().await;
    execute_capability(&config, &registry, capability_name, seen_urls).await
}

async fn cmd_serve(config: corre_core::config::CorreConfig) -> anyhow::Result<()> {
    start_web_server(&config).await
}

/// Execute a capability in run-now mode (no long-lived cache, stores via archive directly).
async fn execute_capability(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    seen_urls: std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    let (edition, mcp_pool) = run_capability_pipeline(config, registry, cap_name, seen_urls).await?;

    let data_dir = config.data_dir();
    let archive = corre_news::archive::Archive::new(&data_dir);
    let path = archive.store(&edition)?;
    tracing::info!("Edition stored at {}", path.display());

    if let Ok(search) = corre_news::search::SearchIndex::open_or_create(&data_dir) {
        let _ = search.index_edition(&edition);
    }

    mcp_pool.shutdown().await;
    tracing::info!("Done. {} articles produced.", edition.article_count());
    Ok(())
}

/// Execute a capability in daemon mode (stores via the shared cache).
async fn execute_capability_with_cache(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    cache: &corre_news::cache::EditionCache,
) -> anyhow::Result<()> {
    let seen_urls = cache.seen_urls().await;
    let (edition, mcp_pool) = run_capability_pipeline(config, registry, cap_name, seen_urls).await?;

    let path = cache.store(&edition).await?;
    tracing::info!("Edition stored at {}", path.display());

    let data_dir = config.data_dir();
    if let Ok(search) = corre_news::search::SearchIndex::open_or_create(&data_dir) {
        let _ = search.index_edition(&edition);
    }

    mcp_pool.shutdown().await;
    tracing::info!("Done. {} articles produced.", edition.article_count());
    Ok(())
}

/// Shared pipeline: build MCP pool, run capability, generate tagline. Returns the edition and pool.
async fn run_capability_pipeline(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    seen_urls: std::collections::HashSet<String>,
) -> anyhow::Result<(corre_core::publish::Edition, corre_mcp::McpPool)> {
    let capability = registry.get(cap_name).with_context(|| format!("Unknown capability: `{cap_name}`"))?.clone();

    let mcp_defs = capability
        .manifest()
        .mcp_servers
        .iter()
        .filter_map(|name| config.mcp.servers.get(name).map(|cfg| (name.clone(), corre_mcp::McpServerDef::from_config(name, cfg))))
        .collect();

    let mcp_pool = corre_mcp::McpPool::new(mcp_defs);

    let llm: Box<dyn corre_core::capability::LlmProvider> = Box::new(corre_llm::OpenAiCompatProvider::from_config(&config.llm)?);

    let config_dir = std::env::current_dir()?;
    let ctx =
        CapabilityContext { mcp: Box::new(mcp_pool.clone()), llm, config_dir, max_concurrent_llm: config.llm.max_concurrent, seen_urls };

    tracing::info!("Running capability `{cap_name}`");

    let timeout = std::time::Duration::from_secs(300);
    let output = tokio::time::timeout(timeout, capability.execute(&ctx))
        .await
        .with_context(|| format!("Capability `{cap_name}` timed out after {timeout:?}"))??;

    let mut edition = corre_core::publish::Edition::new(chrono::Utc::now().date_naive(), output.sections);

    // Generate a dad joke tagline inspired by the headline
    let tagline_llm: Box<dyn corre_core::capability::LlmProvider> = Box::new(corre_llm::OpenAiCompatProvider::from_config(&config.llm)?);
    let tagline_request = corre_core::capability::LlmRequest {
        messages: vec![
            corre_core::capability::LlmMessage {
                role: corre_core::capability::LlmRole::System,
                content: "You are a newspaper sub-editor who writes witty taglines. Write a single short dad joke or pun \
                          (max 15 words) inspired by the given headline. Just the joke, no quotes, no explanation."
                    .into(),
            },
            corre_core::capability::LlmMessage { role: corre_core::capability::LlmRole::User, content: edition.headline.clone() },
        ],
        temperature: Some(0.9),
        max_tokens: Some(60),
        json_mode: false,
    };
    match tagline_llm.complete(tagline_request).await {
        Ok(resp) => {
            let tagline = resp.content.trim().trim_matches('"').to_string();
            if !tagline.is_empty() {
                edition.tagline = tagline;
            }
        }
        Err(e) => tracing::warn!("Failed to generate tagline, using default: {e}"),
    }

    Ok((edition, mcp_pool))
}

async fn start_web_server(config: &corre_core::config::CorreConfig) -> anyhow::Result<()> {
    let data_dir = config.data_dir();
    let archive = corre_news::archive::Archive::new(&data_dir);
    let cache = Arc::new(corre_news::cache::EditionCache::load(archive));
    start_web_server_with_cache(config, cache).await
}

async fn start_web_server_with_cache(
    config: &corre_core::config::CorreConfig,
    cache: Arc<corre_news::cache::EditionCache>,
) -> anyhow::Result<()> {
    let data_dir = config.data_dir();
    let search = corre_news::search::SearchIndex::open_or_create(&data_dir)?;

    let static_dir = PathBuf::from("static");

    let state = Arc::new(corre_news::server::AppState { cache, search, title: config.news.title.clone(), static_dir });

    let addr: std::net::SocketAddr = config.news.bind.parse()?;
    tracing::info!("CorreNews listening on http://{addr}");
    corre_news::server::serve(state, addr).await
}
