use anyhow::Context;
use clap::{Parser, Subcommand};
use corre_core::capability::{CapabilityContext, ProgressStatus};
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
    /// Start the full daemon (scheduler + web server)
    Run,
    /// Run a single capability immediately and exit
    RunNow {
        /// Name of the capability to run
        capability: String,
    },
    /// Start only the web server
    Serve,
    /// Interactive setup wizard — configure LLM, API keys, topics, and systemd
    Setup,
    /// Check and install required external dependencies
    InstallDeps,
    /// Manage the rolodex contact database
    Rolodex {
        #[command(subcommand)]
        action: RolodexAction,
    },
}

#[derive(Subcommand)]
enum RolodexAction {
    /// Import contacts from an external source
    Import {
        /// Source format: csv, google, outlook, facebook, vcard
        source: String,
        /// Path to the import file
        file: std::path::PathBuf,
        /// How to handle duplicates: skip, merge, replace
        #[arg(long, default_value = "skip")]
        duplicates: String,
    },
    /// List all contacts
    List {
        /// Filter by minimum importance level
        #[arg(long)]
        importance: Option<String>,
    },
    /// Add a new contact interactively
    Add,
    /// Edit a contact by ID
    Edit {
        /// Contact ID
        id: String,
    },
    /// Delete a contact by ID
    Delete {
        /// Contact ID
        id: String,
    },
    /// Search contacts by name or email
    Search {
        /// Search query
        query: String,
    },
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

    let config = corre_core::config::CorreConfig::load(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path.display()))?;

    let data_dir = config.data_dir();
    std::fs::create_dir_all(&data_dir)?;

    let log_dir = data_dir.join("capabilities_logs");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "capability.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let stderr_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| config.general.log_level.parse().unwrap_or_default());
    let file_filter = tracing_subscriber::EnvFilter::new(
        "info,corre_core=debug,corre_mcp=debug,corre_llm=debug,corre_capabilities=debug,corre_safety=debug,corre_news=debug,corre_cli=debug",
    );

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_filter(stderr_filter))
        .with(tracing_subscriber::fmt::layer().json().with_ansi(false).with_writer(non_blocking).with_filter(file_filter))
        .init();

    let config_path = std::fs::canonicalize(&config_path).unwrap_or(config_path);

    match command {
        Commands::Run => cmd_run(config, config_path).await,
        Commands::RunNow { capability } => cmd_run_now(config, &capability).await,
        Commands::Serve => cmd_serve(config, config_path).await,
        Commands::Rolodex { action } => cmd_rolodex(config, action),
        Commands::Setup | Commands::InstallDeps => unreachable!(),
    }
}

async fn cmd_run(config: corre_core::config::CorreConfig, config_path: PathBuf) -> anyhow::Result<()> {
    // Shared cache for both web server and scheduler
    let data_dir = config.data_dir();
    let archive = corre_news::archive::Archive::new(&data_dir);
    let cache = Arc::new(corre_news::cache::EditionCache::load(archive));

    // Create execution tracker for dashboard
    let tracker = ExecutionTracker::new(&config.capabilities);

    // Create run-now channel for dashboard triggers
    let (run_tx, mut run_rx) = tokio::sync::mpsc::channel::<String>(8);

    // Build dashboard state and router
    let dashboard_state = Arc::new(corre_dashboard::server::DashboardState {
        tracker: tracker.clone(),
        config: Arc::new(RwLock::new(config.clone())),
        config_path: config_path.clone(),
        run_trigger: run_tx,
    });
    let dashboard_router = corre_dashboard::server::build_router(dashboard_state);

    // Start metrics broadcaster
    corre_dashboard::server::spawn_metrics_broadcaster(tracker.clone());

    // Start web server with dashboard routes merged in
    let web_config = config.clone();
    let web_cache = cache.clone();
    let web_config_path = config_path.clone();
    let web_handle =
        tokio::spawn(async move { start_web_server_with_dashboard(&web_config, web_cache, &web_config_path, dashboard_router).await });

    // Start scheduler
    let mut scheduler = corre_core::scheduler::Scheduler::new().await?;
    let registry = Arc::new(corre_capabilities::registry::CapabilityRegistry::from_config(&config.capabilities));

    for cap_config in config.capabilities.iter().filter(|c| c.enabled) {
        let cap_name = cap_config.name.clone();
        let schedule = cap_config.schedule.clone();
        let config = config.clone();
        let registry = registry.clone();
        let cache = cache.clone();
        let tracker = tracker.clone();

        tracing::info!("Scheduling capability `{cap_name}` with cron `{schedule}`");

        let callback: Box<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync> =
            Box::new(move || {
                let cap_name = cap_name.clone();
                let config = config.clone();
                let registry = registry.clone();
                let cache = cache.clone();
                let tracker = tracker.clone();
                Box::pin(async move {
                    execute_capability_tracked(&config, &registry, &cap_name, &cache, &tracker).await;
                })
            });

        scheduler.add_async_job(&schedule, callback).await?;
    }

    // Spawn run-now receiver that processes dashboard triggers
    let run_config = config.clone();
    let run_registry = registry.clone();
    let run_cache = cache.clone();
    let run_tracker = tracker.clone();
    tokio::spawn(async move {
        while let Some(cap_name) = run_rx.recv().await {
            let config = run_config.clone();
            let registry = run_registry.clone();
            let cache = run_cache.clone();
            let tracker = run_tracker.clone();
            tokio::spawn(async move {
                execute_capability_tracked(&config, &registry, &cap_name, &cache, &tracker).await;
            });
        }
    });

    scheduler.start().await?;
    tracing::info!("Scheduler started. Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down...");
    scheduler.shutdown().await?;

    web_handle.abort();
    Ok(())
}

/// Execute a capability with tracker integration (mark running/completed/failed, push logs).
async fn execute_capability_tracked(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    cache: &corre_news::cache::EditionCache,
    tracker: &ExecutionTracker,
) {
    tracker.mark_running(cap_name).await;
    tracker.push_log(cap_name, "INFO", "Capability execution started").await;

    let seen_urls = cache.seen_urls().await;
    match run_capability_pipeline(config, registry, cap_name, seen_urls, Some(tracker)).await {
        Ok((edition, mcp_pool)) => {
            let article_count = edition.article_count();
            match cache.store(&edition).await {
                Ok(path) => {
                    tracing::info!("Edition stored at {}", path.display());
                    tracker.push_log(cap_name, "INFO", &format!("Edition stored at {}", path.display())).await;
                }
                Err(e) => {
                    tracing::error!("Failed to store edition: {e:#}");
                    tracker.push_log(cap_name, "ERROR", &format!("Failed to store edition: {e:#}")).await;
                }
            }

            let data_dir = config.data_dir();
            if let Ok(search) = corre_news::search::SearchIndex::open_or_create(&data_dir) {
                let _ = search.index_edition(&edition);
            }

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

async fn cmd_run_now(config: corre_core::config::CorreConfig, capability_name: &str) -> anyhow::Result<()> {
    let registry = corre_capabilities::registry::CapabilityRegistry::from_config(&config.capabilities);
    // Load cache to get seen_urls for cross-edition dedup, but store via archive directly
    let data_dir = config.data_dir();
    let archive = corre_news::archive::Archive::new(&data_dir);
    let cache = corre_news::cache::EditionCache::load(archive);
    let seen_urls = cache.seen_urls().await;
    execute_capability(&config, &registry, capability_name, seen_urls).await
}

async fn cmd_serve(config: corre_core::config::CorreConfig, config_path: PathBuf) -> anyhow::Result<()> {
    // In serve-only mode, show dashboard with all capabilities as Idle and run-now disabled
    let tracker = ExecutionTracker::new(&config.capabilities);

    // Create a dummy sender that will never be read (run-now disabled in serve mode)
    let (run_tx, _run_rx) = tokio::sync::mpsc::channel::<String>(1);

    let dashboard_state = Arc::new(corre_dashboard::server::DashboardState {
        tracker: tracker.clone(),
        config: Arc::new(RwLock::new(config.clone())),
        config_path: config_path.clone(),
        run_trigger: run_tx,
    });
    let dashboard_router = corre_dashboard::server::build_router(dashboard_state);

    corre_dashboard::server::spawn_metrics_broadcaster(tracker);

    let data_dir = config.data_dir();
    let archive = corre_news::archive::Archive::new(&data_dir);
    let cache = Arc::new(corre_news::cache::EditionCache::load(archive));
    start_web_server_with_dashboard(&config, cache, &config_path, dashboard_router).await
}

/// Execute a capability in run-now mode (no long-lived cache, stores via archive directly).
async fn execute_capability(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    seen_urls: std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    let (edition, mcp_pool) = run_capability_pipeline(config, registry, cap_name, seen_urls, None).await?;

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

/// Shared pipeline: build MCP pool, run capability, generate tagline. Returns the edition and pool.
async fn run_capability_pipeline(
    config: &corre_core::config::CorreConfig,
    registry: &corre_capabilities::registry::CapabilityRegistry,
    cap_name: &str,
    seen_urls: std::collections::HashSet<String>,
    tracker: Option<&ExecutionTracker>,
) -> anyhow::Result<(corre_core::publish::Edition, corre_mcp::McpPool)> {
    let capability = registry.get(cap_name).with_context(|| format!("Unknown capability: `{cap_name}`"))?.clone();

    let mcp_defs = capability
        .manifest()
        .mcp_servers
        .iter()
        .filter_map(|name| config.mcp.servers.get(name).map(|cfg| (name.clone(), corre_mcp::McpServerDef::from_config(name, cfg))))
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

    let config_dir = config.data_dir();
    let ctx = CapabilityContext { mcp, llm, config_dir, max_concurrent_llm: effective_llm.max_concurrent, seen_urls };

    tracing::info!("Running capability `{cap_name}`");
    if let Some(t) = tracker {
        t.push_log(cap_name, "INFO", "Building MCP pool and LLM provider").await;
    }

    let timeout_dur = std::time::Duration::from_secs(600);
    let poll_deadline = std::time::Duration::from_secs(5);
    let mut execute_fut = std::pin::pin!(capability.execute(&ctx));
    let mut next_poll = timeout_dur;

    let output = loop {
        match tokio::time::timeout(next_poll, &mut execute_fut).await {
            Ok(result) => break result,
            Err(_) => {
                tracing::info!("Capability `{cap_name}` exceeded {next_poll:?}, polling in_progress");
                if let Some(t) = tracker {
                    t.push_log(cap_name, "INFO", &format!("Exceeded {next_poll:?}, polling progress")).await;
                }
                match tokio::time::timeout(poll_deadline, capability.in_progress()).await {
                    Ok(ProgressStatus::StillBusy(hint)) => {
                        next_poll = match hint {
                            Some(pct) if pct > 0 && pct < 100 => {
                                // Update tracker with progress percentage
                                if let Some(t) = tracker {
                                    t.update_progress(cap_name, pct, "processing").await;
                                    t.push_log(cap_name, "INFO", &format!("{pct}% complete")).await;
                                }
                                let remaining_ratio = (100 - pct) as f64 / pct as f64;
                                let secs = (remaining_ratio * timeout_dur.as_secs_f64()) as u64 + 30;
                                tracing::info!("... {pct}% complete, polling again in {secs}s");
                                std::time::Duration::from_secs(secs)
                            }
                            _ => {
                                tracing::info!("... still busy (no hint), polling again in {timeout_dur:?}");
                                timeout_dur
                            }
                        };
                        continue;
                    }
                    Ok(ProgressStatus::Done(partial)) => {
                        let n: usize = partial.sections.iter().map(|s| s.articles.len()).sum();
                        tracing::warn!("Capability `{cap_name}` returning partial results ({n} articles)");
                        if let Some(t) = tracker {
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
                }
            }
        }
    }?;

    if let Some(t) = tracker {
        let article_count: usize = output.sections.iter().map(|s| s.articles.len()).sum();
        t.push_log(cap_name, "INFO", &format!("Pipeline produced {article_count} articles, generating tagline")).await;
        t.update_progress(cap_name, 90, "generating_tagline").await;
    }

    let mut edition = corre_core::publish::Edition::new(chrono::Utc::now().date_naive(), output.sections);

    // Generate a dad joke tagline inspired by the headline
    let tagline_llm: Box<dyn corre_core::capability::LlmProvider> = Box::new(corre_llm::OpenAiCompatProvider::from_config(&effective_llm)?);
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
        max_completion_tokens: Some(200),
        json_mode: false,
    };
    match tagline_llm.complete(tagline_request).await {
        Ok(resp) => {
            let tagline = resp.content.trim().trim_matches('"').to_string();
            if !tagline.is_empty() {
                edition.tagline = tagline;
            }
        }
        Err(e) => tracing::warn!("Failed to generate tagline, using default: {e}."),
    }

    Ok((edition, mcp_pool))
}

async fn start_web_server_with_dashboard(
    config: &corre_core::config::CorreConfig,
    cache: Arc<corre_news::cache::EditionCache>,
    config_path: &std::path::Path,
    dashboard_router: axum::Router,
) -> anyhow::Result<()> {
    let data_dir = config.data_dir();
    let search = corre_news::search::SearchIndex::open_or_create(&data_dir)?;

    let state = Arc::new(corre_news::server::AppState {
        cache,
        search,
        config_path: config_path.to_path_buf(),
        config: Arc::new(RwLock::new(config.clone())),
    });

    let addr: std::net::SocketAddr = config.news.bind.parse()?;
    tracing::info!("CorreNews listening on http://{addr}");
    tracing::info!("Dashboard available at http://{addr}/dashboard");
    corre_news::server::serve_with_extra_routes(state, dashboard_router, addr).await
}

fn cmd_rolodex(config: corre_core::config::CorreConfig, action: RolodexAction) -> anyhow::Result<()> {
    let db_path = config.data_dir().join("rolodex.db");
    let db = corre_db::Database::open(&db_path)?;

    match action {
        RolodexAction::Import { source, file, duplicates } => {
            use corre_capabilities::rolodex_import::{DuplicateAction, ImportResult};

            let dup_action = match duplicates.as_str() {
                "merge" => DuplicateAction::Merge,
                "replace" => DuplicateAction::Replace,
                _ => DuplicateAction::Skip,
            };

            let result: ImportResult = match source.as_str() {
                "csv" => corre_capabilities::rolodex_import::import_csv(&db, &file, dup_action)?,
                "google" => corre_capabilities::rolodex_import::import_google(&db, &file, dup_action)?,
                "outlook" => corre_capabilities::rolodex_import::import_outlook(&db, &file, dup_action)?,
                "facebook" => corre_capabilities::rolodex_import::import_facebook(&db, &file, dup_action)?,
                "vcard" | "vcf" => corre_capabilities::rolodex_import::import_vcard(&db, &file, dup_action)?,
                _ => anyhow::bail!("Unknown import source: {source}. Supported: csv, google, outlook, facebook, vcard"),
            };

            println!("Import complete: {} imported, {} skipped", result.imported, result.skipped);
            if !result.errors.is_empty() {
                println!("Errors ({}):", result.errors.len());
                result.errors.iter().for_each(|e| println!("  - {e}"));
            }
        }
        RolodexAction::List { importance } => {
            let contacts = match importance.as_deref() {
                Some(level) => {
                    let min = corre_db::Importance::from_str_loose(level);
                    db.contacts_by_importance(min)?
                }
                None => db.list_contacts()?,
            };

            if contacts.is_empty() {
                println!("No contacts found.");
            } else {
                println!("{:<36}  {:<20}  {:<30}  {:<10}  {}", "ID", "Name", "Email", "Importance", "Birthday");
                println!("{}", "-".repeat(110));
                for c in &contacts {
                    println!(
                        "{:<36}  {:<20}  {:<30}  {:<10}  {}",
                        c.id,
                        c.full_name(),
                        c.email.as_deref().unwrap_or("-"),
                        c.importance,
                        c.birthday.as_deref().unwrap_or("-")
                    );
                }
                println!("\n{} contacts total", contacts.len());
            }
        }
        RolodexAction::Add => {
            let first_name = dialoguer::Input::<String>::new().with_prompt("First name").interact_text()?;
            let last_name = dialoguer::Input::<String>::new().with_prompt("Last name").interact_text()?;
            let email: String = dialoguer::Input::new().with_prompt("Email (optional)").default(String::new()).interact_text()?;
            let phone: String = dialoguer::Input::new().with_prompt("Phone (optional)").default(String::new()).interact_text()?;
            let birthday: String =
                dialoguer::Input::new().with_prompt("Birthday YYYY-MM-DD (optional)").default(String::new()).interact_text()?;

            let importance_options = ["low", "medium", "high", "veryhigh"];
            let importance_idx = dialoguer::Select::new().with_prompt("Importance").items(&importance_options).default(1).interact()?;
            let importance = corre_db::Importance::from_str_loose(importance_options[importance_idx]);

            let contact = corre_db::contacts::new_contact(
                first_name,
                last_name,
                if email.is_empty() { None } else { Some(email) },
                if phone.is_empty() { None } else { Some(phone) },
                if birthday.is_empty() { None } else { Some(birthday) },
                importance,
            );

            db.insert_contact(&contact)?;
            db.assign_default_strategies(&contact)?;
            println!("Contact added: {} ({})", contact.full_name(), contact.id);
        }
        RolodexAction::Edit { id } => {
            let Some(contact) = db.get_contact(&id)? else {
                anyhow::bail!("Contact {id} not found");
            };

            let first_name =
                dialoguer::Input::<String>::new().with_prompt("First name").default(contact.first_name.clone()).interact_text()?;
            let last_name =
                dialoguer::Input::<String>::new().with_prompt("Last name").default(contact.last_name.clone()).interact_text()?;
            let email = dialoguer::Input::<String>::new()
                .with_prompt("Email")
                .default(contact.email.clone().unwrap_or_default())
                .interact_text()?;
            let phone = dialoguer::Input::<String>::new()
                .with_prompt("Phone")
                .default(contact.phone.clone().unwrap_or_default())
                .interact_text()?;
            let birthday = dialoguer::Input::<String>::new()
                .with_prompt("Birthday YYYY-MM-DD")
                .default(contact.birthday.clone().unwrap_or_default())
                .interact_text()?;

            let importance_options = ["low", "medium", "high", "veryhigh"];
            let current_idx = importance_options.iter().position(|&s| s == contact.importance.as_str()).unwrap_or(1);
            let importance_idx =
                dialoguer::Select::new().with_prompt("Importance").items(&importance_options).default(current_idx).interact()?;
            let importance = corre_db::Importance::from_str_loose(importance_options[importance_idx]);

            let mut updated = contact;
            updated.first_name = first_name;
            updated.last_name = last_name;
            updated.email = if email.is_empty() { None } else { Some(email) };
            updated.phone = if phone.is_empty() { None } else { Some(phone) };
            updated.birthday = if birthday.is_empty() { None } else { Some(birthday) };
            updated.importance = importance;

            db.update_contact(&updated)?;
            println!("Contact updated: {}", updated.full_name());
        }
        RolodexAction::Delete { id } => {
            let Some(contact) = db.get_contact(&id)? else {
                anyhow::bail!("Contact {id} not found");
            };
            let confirmed =
                dialoguer::Confirm::new().with_prompt(format!("Delete {} ({})?", contact.full_name(), id)).default(false).interact()?;
            if confirmed {
                db.delete_contact(&id)?;
                println!("Contact deleted.");
            } else {
                println!("Cancelled.");
            }
        }
        RolodexAction::Search { query } => {
            let results = db.search_contacts(&query)?;
            if results.is_empty() {
                println!("No contacts matching \"{query}\".");
            } else {
                println!("{:<36}  {:<20}  {:<30}  {:<10}", "ID", "Name", "Email", "Importance");
                println!("{}", "-".repeat(100));
                for c in &results {
                    println!("{:<36}  {:<20}  {:<30}  {:<10}", c.id, c.full_name(), c.email.as_deref().unwrap_or("-"), c.importance,);
                }
                println!("\n{} results", results.len());
            }
        }
    }

    Ok(())
}
