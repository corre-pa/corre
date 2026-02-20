mod providers;
mod state;
mod steps;
#[cfg(target_os = "linux")]
mod systemd;
mod templates;
mod validate;

use dialoguer::Confirm;
use state::SetupState;

/// Entry point for `corre setup`.
pub async fn run_setup() -> anyhow::Result<()> {
    let term = console::Term::stderr();

    // Check for existing state (resumability)
    let mut state = if let Some(existing) = SetupState::load() {
        if existing.completed_step > 0 {
            println!();
            let resume = Confirm::new()
                .with_prompt(format!("Found previous setup progress (completed step {}). Resume?", existing.completed_step))
                .default(true)
                .interact()?;
            if resume {
                existing
            } else {
                SetupState::cleanup();
                SetupState::default()
            }
        } else {
            SetupState::default()
        }
    } else {
        SetupState::default()
    };

    let start_step = state.completed_step + 1;

    if start_step <= 1 {
        steps::welcome(&term)?;
        state.completed_step = 1;
        state.save()?;
    }

    if start_step <= 2 {
        steps::llm_provider(&mut state, &term)?;
    }

    if start_step <= 3 {
        steps::brave_search(&mut state, &term)?;
    }

    if start_step <= 4 {
        steps::capabilities(&mut state, &term)?;
    }

    if start_step <= 5 {
        steps::topics(&mut state, &term)?;
    }

    if start_step <= 6 {
        steps::preferences(&mut state, &term)?;
    }

    if start_step <= 7 {
        steps::review_and_write(&mut state, &term)?;
    }

    if start_step <= 8 {
        steps::start(&mut state, &term)?;
    }

    Ok(())
}
