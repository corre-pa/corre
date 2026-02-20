use console::Style;
use dialoguer::{Confirm, Input, MultiSelect, Password, Select};

use super::providers::PROVIDERS;
use super::state::SetupState;
use super::validate;

/// Open a URL in the user's default browser.
fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "start";

    let _ = std::process::Command::new(cmd)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Ask the user if they'd like to open a signup page, and open it if yes.
fn offer_to_open_browser(url: &str, prompt: &str) -> anyhow::Result<()> {
    if Confirm::new().with_prompt(prompt).default(true).interact()? {
        open_browser(url);
        println!();
        println!("  Opened {url} in your browser.");
        println!("  Come back here when you have your API key.");
        println!();
    }
    Ok(())
}

/// Step 1: Welcome screen.
pub fn welcome(term: &console::Term) -> anyhow::Result<()> {
    let heading = Style::new().bold();
    let dim = Style::new().dim();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Welcome to Corre Setup"));
    println!();
    println!("Corre is a personal AI task scheduler that researches topics on a cron");
    println!("schedule and publishes the results as a newspaper-style web interface.");
    println!();
    println!("This wizard will walk you through:");
    println!();
    println!("  1. Choosing an LLM provider (Venice.ai, Ollama, OpenAI, or custom)");
    println!("  2. Setting up Brave Search for web queries");
    println!("  3. Selecting capabilities to enable");
    println!("  4. Configuring your topics and preferences");
    println!("  5. Writing corre.toml and starting the service");
    println!();
    println!("{}", dim.apply_to("Prerequisites: an internet connection and API keys for your chosen providers."));
    println!();

    Confirm::new().with_prompt("Ready to begin?").default(true).interact()?;
    Ok(())
}

/// Step 2: LLM provider selection and API key.
pub fn llm_provider(state: &mut SetupState, term: &console::Term) -> anyhow::Result<()> {
    let heading = Style::new().bold();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Step 2: LLM Provider"));
    println!();
    println!("Corre uses a large language model to score articles for newsworthiness and");
    println!("generate summaries. All providers use the OpenAI-compatible chat API format.");
    println!();

    let labels: Vec<&str> = PROVIDERS.iter().map(|p| p.label).collect();
    let selection = Select::new().with_prompt("Choose a provider").items(&labels).default(0).interact()?;

    let provider = &PROVIDERS[selection];

    println!();
    println!("{}", provider.guidance);
    println!();

    if let Some(url) = provider.signup_url {
        offer_to_open_browser(url, "Open the signup page in your browser?")?;
    }

    // Base URL
    let base_url: String = if provider.key == "custom" {
        Input::new().with_prompt("Base URL").validate_with(|input: &String| validate::url_like(input)).interact_text()?
    } else {
        Input::new().with_prompt("Base URL").default(provider.base_url.into()).interact_text()?
    };

    // Model
    let model: String = if provider.default_model.is_empty() {
        Input::new().with_prompt("Model name").validate_with(|input: &String| validate::non_empty(input)).interact_text()?
    } else {
        Input::new().with_prompt("Model name").default(provider.default_model.into()).interact_text()?
    };

    // API key env var name
    let api_key_env: String =
        Input::new().with_prompt("Environment variable for API key").default(provider.api_key_env.into()).interact_text()?;

    // Actual API key value (stored in state, written to .env later)
    if provider.needs_api_key {
        println!();
        let key: String =
            Password::new().with_prompt(format!("Paste your {api_key_env} value (hidden)")).allow_empty_password(false).interact()?;
        state.api_keys.insert(api_key_env.clone(), key);
    } else {
        println!();
        println!("No API key needed for this provider.");
    }

    state.llm_provider = Some(provider.key.into());
    state.llm_base_url = Some(base_url);
    state.llm_model = Some(model);
    state.llm_api_key_env = Some(api_key_env);
    state.completed_step = 2;
    state.save()?;

    Ok(())
}

/// Step 3: Brave Search API key.
pub fn brave_search(state: &mut SetupState, term: &console::Term) -> anyhow::Result<()> {
    let heading = Style::new().bold();
    let dim = Style::new().dim();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Step 3: Brave Search API Key"));
    println!();
    println!("Corre searches the web for your topics using the Brave Search API.");
    println!("{}", dim.apply_to("Free tier: 2,000 queries/month, no credit card required."));
    println!();
    println!("  1. Go to brave.com/search/api and click \"Get Started\"");
    println!("  2. Create an account");
    println!("  3. Copy your API key from the dashboard");
    println!();

    offer_to_open_browser("https://brave.com/search/api/", "Open the Brave Search signup page in your browser?")?;

    let env_name: String = Input::new().with_prompt("Environment variable name").default("BRAVE_API_KEY".into()).interact_text()?;

    let key: String =
        Password::new().with_prompt(format!("Paste your {env_name} value (hidden)")).allow_empty_password(false).interact()?;

    state.api_keys.insert(env_name.clone(), key);

    state.brave_api_key_env = Some(env_name);
    state.completed_step = 3;
    state.save()?;

    Ok(())
}

/// Step 4: Capability selection.
pub fn capabilities(state: &mut SetupState, term: &console::Term) -> anyhow::Result<()> {
    let heading = Style::new().bold();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Step 4: Capabilities"));
    println!();
    println!("Capabilities are modular tasks that run on a schedule. Select which to enable.");
    println!();

    // For now, only daily-brief exists. The UI supports future additions.
    let catalog = vec![("daily-brief", "Researches topics and produces a daily news briefing (requires Brave Search)")];

    let labels: Vec<String> = catalog.iter().map(|(name, desc)| format!("{name} — {desc}")).collect();
    let defaults: Vec<bool> = vec![true; catalog.len()];

    let selected = MultiSelect::new()
        .with_prompt("Enable capabilities (space to toggle, enter to confirm)")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;

    state.enabled_capabilities = selected.iter().map(|&i| catalog[i].0.into()).collect();

    if state.enabled_capabilities.is_empty() {
        println!();
        println!("No capabilities selected. You can enable them later in corre.toml.");
    }

    state.completed_step = 4;
    state.save()?;

    Ok(())
}

/// Step 5: Topics configuration (only if daily-brief is enabled).
pub fn topics(state: &mut SetupState, term: &console::Term) -> anyhow::Result<()> {
    if !state.enabled_capabilities.iter().any(|c| c == "daily-brief") {
        state.completed_step = 5;
        state.save()?;
        return Ok(());
    }

    let heading = Style::new().bold();
    let dim = Style::new().dim();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Step 5: Topics"));
    println!();
    println!("The daily brief searches for news on topics you define. Topics are organized");
    println!("into sections (e.g. \"Technology\", \"Science\"). Each section has a description");
    println!("of what to search for.");
    println!();
    println!("{}", dim.apply_to("You can always edit config/topics.md later."));
    println!();

    let use_defaults = Confirm::new().with_prompt("Use default topics (Technology, World News, Science)?").default(true).interact()?;

    if use_defaults {
        state.topics_md = Some(super::templates::DEFAULT_TOPICS.into());
    } else {
        let mut topics = String::from("# Daily Brief Topics\n\n");
        loop {
            println!();
            let section: String = Input::new()
                .with_prompt("Section name (e.g. \"Technology\")")
                .validate_with(|input: &String| validate::non_empty(input))
                .interact_text()?;

            let description: String = Input::new()
                .with_prompt(format!("What should Corre search for in \"{section}\"?"))
                .validate_with(|input: &String| validate::non_empty(input))
                .interact_text()?;

            topics.push_str(&format!("## {section}\n{description}\n\n"));

            if !Confirm::new().with_prompt("Add another section?").default(true).interact()? {
                break;
            }
        }
        state.topics_md = Some(topics);
    }

    state.completed_step = 5;
    state.save()?;

    Ok(())
}

/// Step 6: Preferences (schedule, port, title).
pub fn preferences(state: &mut SetupState, term: &console::Term) -> anyhow::Result<()> {
    let heading = Style::new().bold();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Step 6: Preferences"));
    println!();

    let hour: String = Input::new()
        .with_prompt("What hour should the daily brief run? (0-23, 24h format)")
        .default("5".into())
        .validate_with(|input: &String| validate::hour(input))
        .interact_text()?;
    state.schedule_hour = Some(hour.trim().parse().unwrap_or(5));

    let port: String = Input::new()
        .with_prompt("CorreNews web server port")
        .default("3200".into())
        .validate_with(|input: &String| validate::port_number(input))
        .interact_text()?;
    state.news_port = Some(port.trim().parse().unwrap_or(3200));

    let title: String = Input::new().with_prompt("Newspaper title").default("Corre News".into()).interact_text()?;
    state.news_title = Some(title);

    state.completed_step = 6;
    state.save()?;

    Ok(())
}

/// Step 7: Review and write configuration files.
pub fn review_and_write(state: &mut SetupState, term: &console::Term) -> anyhow::Result<()> {
    let heading = Style::new().bold();
    let dim = Style::new().dim();

    let config = super::templates::build_config(state);
    let config_toml = super::templates::format_config_toml(&config)?;

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Step 7: Review Configuration"));
    println!();
    println!("{}", dim.apply_to("--- corre.toml ---"));
    println!("{config_toml}");
    println!("{}", dim.apply_to("--- end ---"));
    println!();

    if !Confirm::new().with_prompt("Write this configuration?").default(true).interact()? {
        println!("Setup cancelled. Your progress is saved — run `corre setup` to resume.");
        return Ok(());
    }

    // Write corre.toml
    std::fs::write("corre.toml", &config_toml)?;
    println!("  Wrote corre.toml");

    // Write config/topics.md
    if let Some(ref topics) = state.topics_md {
        std::fs::create_dir_all("config")?;
        std::fs::write("config/topics.md", topics)?;
        println!("  Wrote config/topics.md");
    }

    // Write .env file from collected API keys
    let data_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join(".local/share/corre");
    std::fs::create_dir_all(&data_dir)?;

    let env_path = data_dir.join(".env");
    let mut env_content = String::from("# Corre API keys — generated by `corre setup`\n");

    for (name, value) in &state.api_keys {
        env_content.push_str(&format!("{name}={value}\n"));
    }

    std::fs::write(&env_path, &env_content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600))?;
    }
    println!("  Wrote {} (mode 600)", env_path.display());

    state.completed_step = 7;
    state.save()?;

    Ok(())
}

/// Step 8: Start / systemd / done.
pub fn start(state: &mut SetupState, term: &console::Term) -> anyhow::Result<()> {
    let heading = Style::new().bold();
    let green = Style::new().green().bold();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Step 8: Start Corre"));
    println!();

    let mut options = vec!["Run now (one-shot test of daily-brief)", "Save config only (start later)"];

    #[cfg(target_os = "linux")]
    options.insert(1, "Set up systemd service (run on boot)");

    let selection = Select::new().with_prompt("How would you like to proceed?").items(&options).default(0).interact()?;

    match options[selection] {
        "Run now (one-shot test of daily-brief)" => {
            println!();
            println!("Starting one-shot run of daily-brief...");
            println!("(This will take a few minutes while articles are scored and summarized)");
            println!();

            state.completed_step = 8;
            SetupState::cleanup();
            println!("{}", green.apply_to("Setup complete!"));
            print_summary(state);

            // Exec `corre run-now daily-brief` with API keys injected as env vars
            let exe = std::env::current_exe()?;
            let env_vars: Vec<(String, String)> = state.api_keys.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let status = std::process::Command::new(&exe).args(["run-now", "daily-brief"]).envs(env_vars).status()?;
            if !status.success() {
                println!();
                println!("The test run had errors. Check the output above and your API keys.");
            }
        }
        #[cfg(target_os = "linux")]
        "Set up systemd service (run on boot)" => {
            let exe = std::env::current_exe()?;
            let working_dir = std::env::current_dir()?;
            let env_file = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join(".local/share/corre/.env");

            let unit = super::systemd::generate_unit_file(&exe, &working_dir, &env_file);
            println!();
            println!("Installing systemd service (requires sudo)...");
            super::systemd::install_service(&unit)?;

            // Check status
            std::thread::sleep(std::time::Duration::from_secs(2));
            let output = std::process::Command::new("systemctl").args(["is-active", "corre"]).output()?;
            let active = String::from_utf8_lossy(&output.stdout).trim().to_string();

            println!();
            if active == "active" {
                println!("{}", green.apply_to("Corre service is running!"));
            } else {
                println!("Service status: {active}. Check: journalctl -u corre -e");
            }

            state.completed_step = 8;
            SetupState::cleanup();
            println!();
            println!("{}", green.apply_to("Setup complete!"));
            print_summary(state);
        }
        _ => {
            state.completed_step = 8;
            SetupState::cleanup();
            println!();
            println!("{}", green.apply_to("Setup complete!"));
            print_summary(state);
        }
    }

    Ok(())
}

fn print_summary(state: &SetupState) {
    let bold = Style::new().bold();
    let dim = Style::new().dim();

    let port = state.news_port.unwrap_or(3200);

    println!();
    println!("{}", bold.apply_to("Summary"));
    println!("{}", dim.apply_to("─────────────────────────────────────────────────"));
    println!("  Config:     corre.toml");
    println!("  Topics:     config/topics.md");
    println!("  Env file:   ~/.local/share/corre/.env");
    println!("  Data dir:   ~/.local/share/corre/");
    println!();
    println!("{}", bold.apply_to("Next steps"));
    println!("  Start daemon:    corre run");
    println!("  One-shot test:   corre run-now daily-brief");
    println!("  Web server:      corre serve");
    println!("  View newspaper:  http://127.0.0.1:{port}");
    println!();
}
