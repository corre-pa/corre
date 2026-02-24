//! Binary entry point for Corre. Parses CLI arguments, wires every workspace crate together,
//! and dispatches to the runtime modes: `run` (scheduler + dashboard), `run-now`
//! (one-shot capability execution), and `setup` (interactive wizard).

use anyhow::Context;
use clap::{Parser, Subcommand};
use corre_core::capability::{CapabilityContext, ProgressEvent, ProgressStatus};
use corre_core::tracker::ExecutionTracker;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

mod setup;

/// Returns the platform-appropriate default config path: `~/.local/share/corre/corre.toml` (Linux),
/// `~/Library/Application Support/corre/corre.toml` (macOS), etc.
fn default_config_path() -> PathBuf {
    setup::templates::resolved_data_dir().join("corre.toml")
}

fn default_env_path() -> PathBuf {
    setup::templates::resolved_data_dir().join(".env")
}

#[derive(Parser)]
#[command(name = "corre", about = "Personal AI task scheduler and newspaper")]
struct Cli {
    /// Path to config file [default: ~/.local/share/corre/corre.toml]
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the full daemon (scheduler + dashboard)
    Run {
        /// Bind address for the dashboard [default: 0.0.0.0:5500]
        #[arg(long, default_value = "0.0.0.0:5500")]
        dashboard_bind: String,
    },
    /// Run a single capability immediately and exit
    RunNow {
        /// Name of the capability to run
        capability: String,
    },
    /// Interactive setup wizard — configure LLM, API keys, topics, and systemd
    Setup,
    /// Check and install required external dependencies
    InstallDeps,
    /// Health check: verify data dir and config are accessible (exit 0/1)
    Health,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::from_filename_override(default_env_path()).ok();
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(default_config_path);

    // No subcommand: run setup if config is missing, otherwise show help
    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            if !config_path.exists() {
                return setup::run_setup().await;
            }
            Cli::parse_from(["corre", "--help"]);
            unreachable!()
        }
    };

    // Setup and InstallDeps don't need an existing config
    if matches!(command, Commands::Setup) {
        return setup::run_setup().await;
    }
    if matches!(command, Commands::InstallDeps) {
        setup::deps::check_dependencies(&console::Term::stderr())?;
        return Ok(());
    }
    if matches!(command, Commands::Health) {
        return cmd_health(&config_path);
    }

    let config = corre_core::config::CorreConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path.display()))?;

    let data_dir = config.data_dir();
    std::fs::create_dir_all(&data_dir).with_context(|| format!("failed to create data directory {}", data_dir.display()))?;

    let log_dir = data_dir.join("capabilities_logs");
    std::fs::create_dir_all(&log_dir).with_context(|| format!("failed to create log directory {}", log_dir.display()))?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "capability.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let stderr_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| config.general.log_level.parse().unwrap_or_default());
    let file_filter = tracing_subscriber::EnvFilter::new(
        "info,corre_core=debug,corre_mcp=debug,corre_llm=debug,corre_capabilities=debug,corre_safety=debug,corre_cli=debug",
    );

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_filter(stderr_filter))
        .with(tracing_subscriber::fmt::layer().json().with_ansi(false).with_writer(non_blocking).with_filter(file_filter))
        .init();

    let config_path = std::fs::canonicalize(&config_path).unwrap_or(config_path);

    // Discover plugins and merge auto-discovered ones into config
    let mut config = config;
    let plugins = corre_core::plugin::discover_plugins(&data_dir);
    merge_discovered_plugins(&mut config, &plugins);

    match command {
        Commands::Run { dashboard_bind } => cmd_run(config, config_path, plugins, dashboard_bind).await,
        Commands::RunNow { capability } => cmd_run_now(config, &capability, plugins).await,
        Commands::Setup | Commands::InstallDeps | Commands::Health => unreachable!(),
    }
}

/// Merge discovered plugins into the config: for each plugin not already in
/// the config's capabilities list, add a default entry.
fn merge_discovered_plugins(config: &mut corre_core::config::CorreConfig, plugins: &[corre_core::plugin::DiscoveredPlugin]) {
    let existing_names: std::collections::HashSet<String> = config.capabilities.iter().map(|c| c.name.clone()).collect();
    for plugin in plugins {
        if existing_names.contains(&plugin.manifest.plugin.name) {
            continue;
        }
        let cap_config = corre_core::plugin::plugin_to_capability_config(plugin);
        tracing::info!("Auto-discovered plugin `{}` at {}", cap_config.name, plugin.dir.display());
        config.capabilities.push(cap_config);
    }
}

async fn cmd_run(
    mut config: corre_core::config::CorreConfig,
    config_path: PathBuf,
    plugins: Vec<corre_core::plugin::DiscoveredPlugin>,
    dashboard_bind: String,
) -> anyhow::Result<()> {
    let data_dir = config.data_dir();

    // Build capability registry first, then filter config to only capabilities
    // that have a backing implementation (built-in or installed plugin).
    let registry = Arc::new(corre_capabilities::registry::CapabilityRegistry::from_config(&config.capabilities, &plugins, &data_dir));
    config.capabilities.retain(|c| registry.get(&c.name).is_some());

    // Create execution tracker for dashboard (only contains real capabilities)
    let tracker = ExecutionTracker::new(&config.capabilities);

    // Create run-now channel for dashboard triggers
    let (run_tx, mut run_rx) = tokio::sync::mpsc::channel::<String>(8);

    // Build registry client and installer
    let registry_client = Arc::new(corre_registry::RegistryClient::new(config.registry.url.clone(), config.registry.cache_ttl_secs));
    let installer = Arc::new(corre_registry::McpInstaller::new(data_dir.clone(), config.registry.url.clone()));

    // Shutdown channel: the restart endpoint sends `true` to trigger graceful exit
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    // Build dashboard state and router
    let dashboard_state = Arc::new(corre_dashboard::server::DashboardState {
        tracker: tracker.clone(),
        config: Arc::new(RwLock::new(config.clone())),
        config_path: config_path.clone(),
        run_trigger: run_tx,
        registry_client: registry_client.clone(),
        installer: installer.clone(),
        service_manager: Arc::new(corre_core::service::ServiceManager::new()),
        shutdown_signal: shutdown_tx,
        plugins: Arc::new(plugins.clone()),
    });
    let dashboard_router = corre_dashboard::server::build_router(dashboard_state);

    // Start metrics broadcaster
    corre_dashboard::server::spawn_metrics_broadcaster(tracker.clone());

    // Start dashboard server
    let addr: std::net::SocketAddr = dashboard_bind.parse().context("Invalid --dashboard-bind address")?;
    tracing::info!("Dashboard listening on http://{addr}");
    let web_handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, dashboard_router).await?;
        Ok::<(), anyhow::Error>(())
    });

    // Start scheduler
    let mut scheduler = corre_core::scheduler::Scheduler::new().await?;

    for cap_config in config.capabilities.iter().filter(|c| c.enabled) {
        let cap_name = cap_config.name.clone();
        let schedule = cap_config.schedule.clone();
        let config = config.clone();
        let registry = registry.clone();
        let tracker = tracker.clone();

        tracing::info!("Scheduling capability `{cap_name}` with cron `{schedule}`");

        let callback: Box<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync> =
            Box::new(move || {
                let cap_name = cap_name.clone();
                let config = config.clone();
                let registry = registry.clone();
                let tracker = tracker.clone();
                Box::pin(async move {
                    execute_capability_tracked(&config, &registry, &cap_name, tracker.clone()).await;
                })
            });

        scheduler.add_async_job(&schedule, callback).await?;
    }

    // Spawn run-now receiver that processes dashboard triggers
    let run_config = config.clone();
    let run_registry = registry.clone();
    let run_tracker = tracker.clone();
    tokio::spawn(async move {
        while let Some(cap_name) = run_rx.recv().await {
            let config = run_config.clone();
            let registry = run_registry.clone();
            let tracker = run_tracker.clone();
            tokio::spawn(async move {
                execute_capability_tracked(&config, &registry, &cap_name, tracker).await;
            });
        }
    });

    scheduler.start().await?;
    tracing::info!("Scheduler started. Press Ctrl+C to stop.");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Ctrl+C received, shutting down...");
        }
        _ = shutdown_rx.changed() => {
            tracing::info!("Restart requested via dashboard, shutting down...");
        }
    }

    scheduler.shutdown().await?;
    web_handle.abort();
    Ok(())
}

/// Execute a capability with tracker integration (mark running/completed/failed, push logs).
async fn execute_capability_tracked(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    tracker: Arc<ExecutionTracker>,
) {
    tracker.mark_running(cap_name).await;
    tracker.push_log(cap_name, "INFO", "Capability execution started").await;

    match run_capability_pipeline(config, registry, cap_name, Some(tracker.clone())).await {
        Ok((output, mcp_pool)) => {
            let article_count: usize = output.sections.iter().map(|s| s.articles.len()).sum();
            mcp_pool.shutdown().await;
            tracker.mark_completed(cap_name, article_count).await;
            tracker.push_log(cap_name, "INFO", &format!("Completed: {article_count} articles produced")).await;
        }
        Err(e) => {
            let error_msg = format!("{e:#}");
            tracing::error!("Capability `{cap_name}` failed: {error_msg}");
            tracker.mark_failed(cap_name, &error_msg).await;
            tracker.push_log(cap_name, "ERROR", &format!("Failed: {error_msg}")).await;
        }
    }
}

async fn cmd_run_now(
    config: corre_core::config::CorreConfig,
    capability_name: &str,
    plugins: Vec<corre_core::plugin::DiscoveredPlugin>,
) -> anyhow::Result<()> {
    let data_dir = config.data_dir();
    let registry = corre_capabilities::registry::CapabilityRegistry::from_config(&config.capabilities, &plugins, &data_dir);

    let (output, mcp_pool) = run_capability_pipeline(&config, &registry, capability_name, None).await?;

    mcp_pool.shutdown().await;
    let article_count: usize = output.sections.iter().map(|s| s.articles.len()).sum();
    tracing::info!("Done. {article_count} articles produced.");
    Ok(())
}

/// Shared pipeline: build MCP pool, run capability. Returns the output and pool.
async fn run_capability_pipeline(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    tracker: Option<Arc<ExecutionTracker>>,
) -> anyhow::Result<(corre_core::capability::CapabilityOutput, corre_mcp::McpPool)> {
    let capability = registry.get(cap_name).with_context(|| format!("Unknown capability: `{cap_name}`"))?.clone();

    let mcp_servers = config.resolved_mcp_servers()?;
    let required = &capability.manifest().mcp_servers;
    let missing: Vec<_> = required.iter().filter(|name| !mcp_servers.contains_key(name.as_str())).collect();
    if !missing.is_empty() {
        let names = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
        tracing::error!("Missing MCP server config for: {names}");
        anyhow::bail!("Missing MCP server config for: {names}");
    }

    let mcp_defs = required
        .iter()
        .filter_map(|name| mcp_servers.get(name).map(|cfg| (name.clone(), corre_mcp::McpServerDef::from_config(name, cfg))))
        .collect();

    let mcp_pool = corre_mcp::McpPool::new(mcp_defs);

    // Resolve per-capability LLM overrides, falling back to the global config
    let cap_config = config.capabilities.iter().find(|c| c.name == cap_name);
    let effective_llm = match cap_config.and_then(|c| c.llm.as_ref()) {
        Some(overrides) => config.llm.with_overrides(overrides),
        None => config.llm.clone(),
    };

    let raw_llm: Box<dyn corre_core::capability::LlmProvider> = Box::new(corre_llm::OpenAiCompatProvider::from_config(&effective_llm)?);

    // Conditionally wrap MCP and LLM with safety middleware
    let mcp: Box<dyn corre_core::capability::McpCaller> = if config.safety.enabled {
        tracing::info!("Safety layer enabled — wrapping MCP caller and LLM provider");
        Box::new(corre_safety::SafeMcpCaller::new(Box::new(mcp_pool.clone()), &config.safety))
    } else {
        Box::new(mcp_pool.clone())
    };
    let llm: Box<dyn corre_core::capability::LlmProvider> =
        if config.safety.enabled { Box::new(corre_safety::SafeLlmProvider::new(raw_llm, &config.safety)) } else { raw_llm };

    // Spawn a bridge task that forwards ProgressEvents to the ExecutionTracker.
    // The task self-terminates when the sender (held by ctx) is dropped.
    let progress_tx = tracker.as_ref().map(|t| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
        let t = t.clone();
        let name = cap_name.to_string();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    ProgressEvent::Progress { pct, phase } => {
                        t.update_progress(&name, pct.unwrap_or(0), &phase).await;
                    }
                    ProgressEvent::Log { level, message } => {
                        t.push_log(&name, &level, &message).await;
                    }
                }
            }
        });
        tx
    });

    let config_dir = config.data_dir().join(cap_name);
    let ctx = CapabilityContext {
        mcp,
        llm,
        config_dir,
        max_concurrent_llm: effective_llm.max_concurrent,
        seen_urls: std::collections::HashSet::new(),
        progress_tx,
    };

    tracing::info!("Running capability `{cap_name}`");
    if let Some(ref t) = tracker {
        t.push_log(cap_name, "INFO", "Building MCP pool and LLM provider").await;
    }

    let poll_interval = std::time::Duration::from_secs(1);
    let poll_deadline = std::time::Duration::from_secs(5);
    let mut execute_fut = std::pin::pin!(capability.execute(&ctx));

    let output = loop {
        match tokio::time::timeout(poll_interval, &mut execute_fut).await {
            Ok(result) => break result,
            Err(_) => match tokio::time::timeout(poll_deadline, capability.in_progress()).await {
                Ok(ProgressStatus::StillBusy(hint)) => {
                    if let Some(pct) = hint
                        && pct > 0
                        && pct < 100
                    {
                        if let Some(ref t) = tracker {
                            t.update_progress(cap_name, pct, "processing").await;
                        }
                    }
                    continue;
                }
                Ok(ProgressStatus::Done(partial)) => {
                    let n: usize = partial.sections.iter().map(|s| s.articles.len()).sum();
                    tracing::warn!("Capability `{cap_name}` returning partial results ({n} articles)");
                    if let Some(ref t) = tracker {
                        t.push_log(cap_name, "WARN", &format!("Returning partial results ({n} articles)")).await;
                    }
                    break Ok(partial);
                }
                Ok(ProgressStatus::Stuck) => {
                    break Err(anyhow::anyhow!("Capability `{cap_name}` is stuck"));
                }
                Err(_) => {
                    break Err(anyhow::anyhow!("Capability `{cap_name}` in_progress poll unresponsive"));
                }
            },
        }
    }?;

    if let Some(ref t) = tracker {
        let article_count: usize = output.sections.iter().map(|s| s.articles.len()).sum();
        t.push_log(cap_name, "INFO", &format!("Pipeline produced {article_count} articles")).await;
    }

    Ok((output, mcp_pool))
}

fn cmd_health(config_path: &std::path::Path) -> anyhow::Result<()> {
    let config = corre_core::config::CorreConfig::load(config_path)?;
    let data_dir = config.data_dir();
    anyhow::ensure!(data_dir.is_dir(), "data directory does not exist: {}", data_dir.display());
    Ok(())
}
