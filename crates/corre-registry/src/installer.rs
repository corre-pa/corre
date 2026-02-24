use crate::manifest::{InstallMethod, RegistryEntry};
use corre_core::config::McpServerConfig;
use std::collections::HashMap;
use std::path::PathBuf;

/// Handles installing and uninstalling MCP servers from registry entries.
pub struct McpInstaller {
    data_dir: PathBuf,
    registry_base_url: String,
}

impl McpInstaller {
    pub fn new(data_dir: PathBuf, registry_base_url: String) -> Self {
        Self { data_dir, registry_base_url }
    }

    /// Install an MCP server from a registry entry. Returns the `McpServerConfig` to
    /// add to `corre.toml`. The `env_values` map provides the *env var names* (not secrets)
    /// for each required env var.
    pub async fn install(&self, entry: &RegistryEntry, env_values: &HashMap<String, String>) -> Result<McpServerConfig, InstallError> {
        match &entry.install {
            InstallMethod::Npx { command, args, .. } => {
                // npx auto-downloads on first run; nothing to pre-install.
                let env: HashMap<String, String> =
                    entry.config.iter().filter_map(|spec| env_values.get(&spec.name).map(|v| (spec.name.clone(), v.clone()))).collect();

                Ok(McpServerConfig { command: command.clone(), args: args.clone(), env, registry_id: Some(entry.id.clone()) })
            }
            InstallMethod::Pip { command, args, .. } => {
                // uvx / pipx auto-downloads; nothing to pre-install.
                let env: HashMap<String, String> =
                    entry.config.iter().filter_map(|spec| env_values.get(&spec.name).map(|v| (spec.name.clone(), v.clone()))).collect();

                Ok(McpServerConfig { command: command.clone(), args: args.clone(), env, registry_id: Some(entry.id.clone()) })
            }
            InstallMethod::Binary { download_url_template, binary_name, sha256, command, args } => {
                self.install_binary(download_url_template, binary_name, sha256, command, args, entry, env_values).await
            }
        }
    }

    async fn install_binary(
        &self,
        url_template: &str,
        binary_name: &str,
        sha256_map: &HashMap<String, String>,
        command: &str,
        args: &[String],
        entry: &RegistryEntry,
        env_values: &HashMap<String, String>,
    ) -> Result<McpServerConfig, InstallError> {
        let bin_dir = self.data_dir.join("bin");
        tokio::fs::create_dir_all(&bin_dir).await.map_err(|e| InstallError::Io(e.to_string()))?;

        let platform = if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "unknown"
        };
        let arch = if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "unknown"
        };

        let platform_key = format!("{platform}-{arch}");
        let expected_sha256 = sha256_map.get(&platform_key).ok_or_else(|| InstallError::UnsupportedPlatform(platform_key.clone()))?;

        let expanded = url_template
            .replace("{version}", &entry.version)
            .replace("{platform}", platform)
            .replace("{arch}", arch)
            .replace("{bin_dir}", &bin_dir.to_string_lossy());

        // Resolve relative URLs against the registry base URL
        let url = if expanded.starts_with("http://") || expanded.starts_with("https://") {
            expanded
        } else {
            format!("{}/{}", self.registry_base_url.trim_end_matches('/'), expanded.trim_start_matches('/'))
        };

        tracing::info!("Downloading binary from {url}");
        let resp = reqwest::get(&url).await.map_err(|e| InstallError::Download(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(InstallError::Download(format!("HTTP {}", resp.status())));
        }

        let bytes = resp.bytes().await.map_err(|e| InstallError::Download(e.to_string()))?;

        // Verify SHA-256
        use sha2::Digest;
        let digest = sha2::Sha256::digest(&bytes);
        let hex_digest = format!("{digest:x}");
        if hex_digest != *expected_sha256 {
            return Err(InstallError::ChecksumMismatch { expected: expected_sha256.clone(), actual: hex_digest });
        }

        let dest = bin_dir.join(binary_name);
        tokio::fs::write(&dest, &bytes).await.map_err(|e| InstallError::Io(e.to_string()))?;

        // chmod +x on unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&dest, perms).map_err(|e| InstallError::Io(e.to_string()))?;
        }

        // Resolve command: replace {bin_dir} in the command template, or
        // default to the full path inside bin_dir when the command is a bare name.
        let bin_dir_str = bin_dir.to_string_lossy();
        let resolved_command = if command.contains("{bin_dir}") {
            command.replace("{bin_dir}", &bin_dir_str)
        } else {
            bin_dir.join(command).to_string_lossy().into_owned()
        };
        let resolved_args: Vec<String> = args.iter().map(|a| a.replace("{bin_dir}", &bin_dir_str)).collect();

        let env: HashMap<String, String> =
            entry.config.iter().filter_map(|spec| env_values.get(&spec.name).map(|v| (spec.name.clone(), v.clone()))).collect();

        Ok(McpServerConfig { command: resolved_command, args: resolved_args, env, registry_id: Some(entry.id.clone()) })
    }

    /// Uninstall an MCP server: remove binary/package artifacts.
    /// The per-MCP config file is handled by the caller (set `installed = false`).
    pub async fn uninstall(&self, server_name: &str, server_config: &McpServerConfig) -> Result<(), InstallError> {
        let bin_dir = self.data_dir.join("bin");
        let binary_path = bin_dir.join(&server_config.command);

        if binary_path.exists() {
            // Binary install — delete the binary
            tokio::fs::remove_file(&binary_path).await.map_err(|e| InstallError::Io(e.to_string()))?;
        } else if server_config.command == "npx" {
            // npx-based — try global uninstall of the package
            if let Some(pkg) = server_config.args.iter().find(|a| !a.starts_with('-')) {
                let output = tokio::process::Command::new("npm")
                    .args(["uninstall", "-g", pkg])
                    .output()
                    .await
                    .map_err(|e| InstallError::Io(e.to_string()))?;
                if !output.status.success() {
                    tracing::warn!("npm uninstall -g {pkg} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
                }
            }
        } else if ["uvx", "pipx"].contains(&server_config.command.as_str()) {
            // pip-based — try pip uninstall
            if let Some(pkg) = server_config.args.first() {
                let output = tokio::process::Command::new("pip")
                    .args(["uninstall", "-y", pkg])
                    .output()
                    .await
                    .map_err(|e| InstallError::Io(e.to_string()))?;
                if !output.status.success() {
                    tracing::warn!("pip uninstall -y {pkg} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
                }
            }
        }

        tracing::info!("Uninstalled MCP server `{server_name}`");
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("download failed: {0}")]
    Download(String),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("unsupported platform: no checksum for {0}")]
    UnsupportedPlatform(String),
    #[error("IO error: {0}")]
    Io(String),
}
