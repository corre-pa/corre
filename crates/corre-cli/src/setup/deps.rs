use console::Style;
use dialoguer::{Confirm, Select};
use std::process::Command;

/// A required external dependency.
struct Dependency {
    name: &'static str,
    commands_to_check: &'static [&'static str],
    description: &'static str,
    install_instructions: InstallInstructions,
}

/// Platform-specific installation instructions and auto-install commands.
struct InstallInstructions {
    linux: &'static [InstallOption],
    macos: &'static [InstallOption],
    windows: &'static [InstallOption],
}

struct InstallOption {
    label: &'static str,
    command: &'static str,
    args: &'static [&'static str],
}

/// Result of checking a single dependency.
struct DepStatus {
    name: &'static str,
    found: bool,
    found_via: Option<String>,
}

/// Dependencies that can be installed via system package managers.
const DEPS: &[Dependency] = &[
    Dependency {
        name: "Node.js",
        commands_to_check: &["node"],
        description: "Required to run MCP servers via npx (e.g. Brave Search)",
        install_instructions: InstallInstructions {
            linux: &[
                InstallOption { label: "apt (Debian/Ubuntu)", command: "sudo", args: &["apt", "install", "-y", "nodejs", "npm"] },
                InstallOption { label: "dnf (Fedora/RHEL)", command: "sudo", args: &["dnf", "install", "-y", "nodejs", "npm"] },
                InstallOption { label: "pacman (Arch)", command: "sudo", args: &["pacman", "-S", "--noconfirm", "nodejs", "npm"] },
            ],
            macos: &[InstallOption { label: "Homebrew", command: "brew", args: &["install", "node"] }],
            windows: &[
                InstallOption { label: "winget", command: "winget", args: &["install", "-e", "--id", "OpenJS.NodeJS"] },
                InstallOption { label: "choco", command: "choco", args: &["install", "nodejs", "-y"] },
            ],
        },
    },
    Dependency {
        name: "npx",
        commands_to_check: &["npx"],
        description: "Included with Node.js; used to launch MCP server packages",
        install_instructions: InstallInstructions {
            // Same as Node.js — npx is bundled with npm which ships with Node.js
            linux: &[
                InstallOption { label: "apt (Debian/Ubuntu)", command: "sudo", args: &["apt", "install", "-y", "nodejs", "npm"] },
                InstallOption { label: "dnf (Fedora/RHEL)", command: "sudo", args: &["dnf", "install", "-y", "nodejs", "npm"] },
                InstallOption { label: "pacman (Arch)", command: "sudo", args: &["pacman", "-S", "--noconfirm", "nodejs", "npm"] },
            ],
            macos: &[InstallOption { label: "Homebrew", command: "brew", args: &["install", "node"] }],
            windows: &[
                InstallOption { label: "winget", command: "winget", args: &["install", "-e", "--id", "OpenJS.NodeJS"] },
                InstallOption { label: "choco", command: "choco", args: &["install", "nodejs", "-y"] },
            ],
        },
    },
];

/// Check whether a command is available on PATH.
fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Get current platform's install options for a dependency.
fn platform_options(dep: &Dependency) -> &'static [InstallOption] {
    if cfg!(target_os = "macos") {
        dep.install_instructions.macos
    } else if cfg!(target_os = "windows") {
        dep.install_instructions.windows
    } else {
        dep.install_instructions.linux
    }
}

/// Check all dependencies and return their status.
fn check_all() -> Vec<DepStatus> {
    DEPS.iter()
        .map(|dep| {
            let found_cmd = dep.commands_to_check.iter().find(|cmd| command_exists(cmd));
            DepStatus { name: dep.name, found: found_cmd.is_some(), found_via: found_cmd.map(|c| (*c).to_string()) }
        })
        .collect()
}

/// Attempt to install a system dependency using the user's chosen package manager.
fn try_install(dep: &Dependency) -> anyhow::Result<bool> {
    let options = platform_options(dep);
    if options.is_empty() {
        println!("  No automatic installation available for this platform.");
        println!("  Please install {} manually and re-run `corre setup`.", dep.name);
        return Ok(false);
    }

    let labels: Vec<String> = options
        .iter()
        .map(|opt| format!("{} — {} {}", opt.label, opt.command, opt.args.join(" ")))
        .chain(std::iter::once("Skip (install manually later)".into()))
        .collect();

    let selection = Select::new().with_prompt(format!("Install {} using", dep.name)).items(&labels).default(0).interact()?;

    if selection >= options.len() {
        return Ok(false);
    }

    let opt = &options[selection];
    println!();
    println!("  Running: {} {}", opt.command, opt.args.join(" "));
    println!();

    let status = Command::new(opt.command).args(opt.args).status()?;

    if status.success() {
        let now_found = dep.commands_to_check.iter().any(|cmd| command_exists(cmd));
        if now_found {
            println!("  {} installed successfully.", dep.name);
            Ok(true)
        } else {
            println!("  Command completed but {} is still not found on PATH.", dep.name);
            println!("  You may need to restart your shell or add it to your PATH.");
            Ok(false)
        }
    } else {
        println!("  Installation command failed. Please install {} manually.", dep.name);
        Ok(false)
    }
}

/// Run the full dependency check. Returns Ok(true) if all deps are satisfied,
/// Ok(false) if the user chose to continue despite missing deps.
/// Called early in the setup wizard before configuration steps.
pub fn check_dependencies(term: &console::Term) -> anyhow::Result<bool> {
    let heading = Style::new().bold();
    let green = Style::new().green();
    let red = Style::new().red();
    let dim = Style::new().dim();

    term.clear_screen()?;
    println!();
    println!("{}", heading.apply_to("Checking dependencies..."));
    println!();

    let mut statuses = check_all();

    // Display initial status
    for st in &statuses {
        if st.found {
            println!("  {} {} {}", green.apply_to("✓"), st.name, dim.apply_to(format!("({})", st.found_via.as_deref().unwrap_or("found"))));
        } else {
            println!("  {} {}", red.apply_to("✗"), st.name);
        }
    }

    let missing: Vec<usize> = statuses.iter().enumerate().filter(|(_, s)| !s.found).map(|(i, _)| i).collect();

    if missing.is_empty() {
        println!();
        println!("{}", green.apply_to("All dependencies found."));
        println!();
        return Ok(true);
    }

    println!();
    println!("{} missing: {}", missing.len(), missing.iter().map(|&i| statuses[i].name).collect::<Vec<_>>().join(", "));
    println!();

    // Deduplicate: if both "Node.js" and "npx" are missing, only offer to install Node.js
    // (npx is bundled with npm which comes with Node.js)
    let node_missing = statuses.iter().any(|s| s.name == "Node.js" && !s.found);
    let npx_missing = statuses.iter().any(|s| s.name == "npx" && !s.found);
    let skip_npx_install = node_missing && npx_missing;

    let install = Confirm::new().with_prompt("Would you like to install missing dependencies now?").default(true).interact()?;

    if install {
        for &idx in &missing {
            let name = statuses[idx].name;

            // npx will be installed alongside Node.js
            if skip_npx_install && name == "npx" {
                println!();
                println!("  {} — will be installed with Node.js", dim.apply_to("npx"));
                continue;
            }

            // System package dependency (Node.js, npx standalone)
            if let Some(dep) = DEPS.iter().find(|d| d.name == name) {
                println!();
                println!("{}", heading.apply_to(format!("Installing {name}")));
                println!("  {}", dep.description);
                println!();
                try_install(dep)?;
            }
        }

        // Re-check after installation
        println!();
        println!("{}", heading.apply_to("Re-checking dependencies..."));
        println!();
        statuses = check_all();

        for st in &statuses {
            if st.found {
                println!("  {} {}", green.apply_to("✓"), st.name);
            } else {
                println!("  {} {}", red.apply_to("✗"), st.name);
            }
        }

        let still_missing: Vec<&str> = statuses.iter().filter(|s| !s.found).map(|s| s.name).collect();
        if still_missing.is_empty() {
            println!();
            println!("{}", green.apply_to("All dependencies found."));
            println!();
            return Ok(true);
        }

        println!();
        println!("Still missing: {}", still_missing.join(", "));
        println!("Corre may not work correctly without these dependencies.");
        println!();

        let proceed = Confirm::new().with_prompt("Continue setup anyway?").default(false).interact()?;
        if !proceed {
            println!("Install the missing dependencies and re-run `corre setup`.");
            return Ok(false);
        }
    } else {
        println!();
        println!("You can install them later. Corre may not work correctly without:");
        for &idx in &missing {
            let name = statuses[idx].name;
            if let Some(dep) = DEPS.iter().find(|d| d.name == name) {
                println!("  - {name} — {}", dep.description);
            }
        }
        println!();

        let proceed = Confirm::new().with_prompt("Continue setup anyway?").default(false).interact()?;
        if !proceed {
            println!("Install the missing dependencies and re-run `corre setup`.");
            return Ok(false);
        }
    }

    Ok(true)
}
