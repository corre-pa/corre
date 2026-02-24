//! systemd service integration for Corre on Linux.
//!
//! Generates a `corre.service` unit file and installs it via `sudo systemctl`
//! during setup step 9 when the user opts for automatic startup on boot.

use std::path::Path;

/// Generate a systemd unit file for the Corre daemon.
pub fn generate_unit_file(binary_path: &Path, working_dir: &Path, env_file: &Path) -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let home = dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_else(|| "/root".into());

    format!(
        "\
[Unit]
Description=Corre — personal AI task scheduler and newspaper
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={user}
WorkingDirectory={working_dir}
EnvironmentFile={env_file}
Environment=PATH={home}/.cargo/bin:/usr/local/bin:/usr/bin:/bin
ExecStart={binary_path} run
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
",
        working_dir = working_dir.display(),
        env_file = env_file.display(),
        binary_path = binary_path.display(),
    )
}

/// Write the systemd unit and enable/start the service. Requires sudo.
pub fn install_service(unit_content: &str) -> anyhow::Result<()> {
    let service_path = "/etc/systemd/system/corre.service";

    // Write via sudo tee
    let mut child = std::process::Command::new("sudo")
        .args(["tee", service_path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()?;

    if let Some(ref mut stdin) = child.stdin {
        use std::io::Write;
        stdin.write_all(unit_content.as_bytes())?;
    }
    child.wait()?;

    let status = std::process::Command::new("sudo").args(["systemctl", "daemon-reload"]).status()?;
    if !status.success() {
        anyhow::bail!("systemctl daemon-reload failed");
    }

    let status = std::process::Command::new("sudo").args(["systemctl", "enable", "corre"]).status()?;
    if !status.success() {
        anyhow::bail!("systemctl enable failed");
    }

    let status = std::process::Command::new("sudo").args(["systemctl", "restart", "corre"]).status()?;
    if !status.success() {
        anyhow::bail!("systemctl restart failed");
    }

    Ok(())
}
