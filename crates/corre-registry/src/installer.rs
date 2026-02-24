//! MCP server and capability installer/uninstaller.
//!
//! [`McpInstaller`] handles MCP servers (converting [`McpRegistryEntry`] into `McpServerConfig`)
//! and capabilities (downloading binaries to `{data_dir}/plugins/{id}/bin/`, generating
//! `manifest.toml`, and auto-installing MCP dependencies).

use crate::manifest::{CapabilityEntry, InstallMethod, McpRegistryEntry};
use corre_core::config::McpServerConfig;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Handles installing and uninstalling MCP servers and capabilities from registry entries.
pub struct McpInstaller {
    data_dir: PathBuf,
    registry_base_url: String,
}

impl McpInstaller {
    pub fn new(data_dir: PathBuf, registry_base_url: String) -> Self {
        Self { data_dir, registry_base_url }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    // =========================================================================
    // MCP server install/uninstall
    // =========================================================================

    /// Install an MCP server from a registry entry. Returns the `McpServerConfig` to
    /// add to `corre.toml`. The `env_values` map provides the *env var names* (not secrets)
    /// for each required env var.
    pub async fn install(&self, entry: &McpRegistryEntry, env_values: &HashMap<String, String>) -> Result<McpServerConfig, InstallError> {
        let env: HashMap<String, String> =
            entry.config.iter().filter_map(|spec| env_values.get(&spec.name).map(|v| (spec.name.clone(), v.clone()))).collect();
        match &entry.install {
            InstallMethod::Npx { command, args, .. } => Ok(McpServerConfig {
                command: command.clone(),
                args: args.clone(),
                env,
                registry_id: Some(entry.id.clone()),
                installed: true,
            }),
            InstallMethod::Pip { command, args, .. } => Ok(McpServerConfig {
                command: command.clone(),
                args: args.clone(),
                env,
                registry_id: Some(entry.id.clone()),
                installed: true,
            }),
            InstallMethod::Binary { download_url_template, binary_name, sha256, command, args } => {
                let bin_dir = self.data_dir.join("bin");
                tokio::fs::create_dir_all(&bin_dir).await.map_err(|e| InstallError::Io(e.to_string()))?;
                self.download_and_verify(download_url_template, sha256, &entry.version, &bin_dir, binary_name).await?;

                let bin_dir_str = bin_dir.to_string_lossy();
                let resolved_command = if command.contains("{bin_dir}") {
                    command.replace("{bin_dir}", &bin_dir_str)
                } else {
                    bin_dir.join(command).to_string_lossy().into_owned()
                };
                let resolved_args: Vec<String> = args.iter().map(|a| a.replace("{bin_dir}", &bin_dir_str)).collect();

                Ok(McpServerConfig {
                    command: resolved_command,
                    args: resolved_args,
                    env,
                    registry_id: Some(entry.id.clone()),
                    installed: true,
                })
            }
        }
    }

    /// Uninstall an MCP server: remove binary/package artifacts.
    /// The per-MCP config file is handled by the caller (set `installed = false`).
    pub async fn uninstall(&self, server_name: &str, server_config: &McpServerConfig) -> Result<(), InstallError> {
        let bin_dir = self.data_dir.join("bin");
        let binary_path = bin_dir.join(&server_config.command);

        if binary_path.exists() {
            tokio::fs::remove_file(&binary_path).await.map_err(|e| InstallError::Io(e.to_string()))?;
        } else if server_config.command == "npx" {
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
        } else if ["uvx", "pipx"].contains(&server_config.command.as_str())
            && let Some(pkg) = server_config.args.first()
        {
            let output = tokio::process::Command::new("pip")
                .args(["uninstall", "-y", pkg])
                .output()
                .await
                .map_err(|e| InstallError::Io(e.to_string()))?;
            if !output.status.success() {
                tracing::warn!("pip uninstall -y {pkg} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
            }
        }

        tracing::info!("Uninstalled MCP server `{server_name}`");
        Ok(())
    }

    // =========================================================================
    // Capability install/uninstall
    // =========================================================================

    /// Install a capability from a registry entry.
    ///
    /// 1. Downloads the binary using the existing `InstallMethod` logic
    /// 2. Places it at `{data_dir}/plugins/{id}/bin/capability`
    /// 3. Generates `manifest.toml` from the inline manifest
    /// 4. Writes per-capability config at `{data_dir}/config/capabilities/{id}.toml`
    ///
    /// Returns `(plugin_dir, mcp_deps)` where `mcp_deps` is the list of MCP server IDs
    /// that this capability depends on. The caller should install any that are missing.
    pub async fn install_capability(&self, entry: &CapabilityEntry) -> Result<(PathBuf, Vec<String>), InstallError> {
        let plugin_dir = self.data_dir.join("plugins").join(&entry.id);
        let bin_dir = plugin_dir.join("bin");
        tokio::fs::create_dir_all(&bin_dir).await.map_err(|e| InstallError::Io(e.to_string()))?;

        match &entry.install {
            InstallMethod::Binary { download_url_template, binary_name, sha256, .. } => {
                self.download_and_verify(download_url_template, sha256, &entry.version, &bin_dir, binary_name).await?;
            }
            _ => {
                return Err(InstallError::Io("only binary install method is supported for capabilities".into()));
            }
        }

        // Generate manifest.toml from the inline manifest
        let manifest_toml = self.generate_capability_manifest(entry);
        let manifest_path = plugin_dir.join("manifest.toml");
        tokio::fs::write(&manifest_path, manifest_toml).await.map_err(|e| InstallError::Io(e.to_string()))?;

        // Write per-capability config file
        let config_dir = self.data_dir.join("config").join("capabilities");
        tokio::fs::create_dir_all(&config_dir).await.map_err(|e| InstallError::Io(e.to_string()))?;

        let cap_config = format!("installed = true\nregistry_id = \"{}\"\nversion = \"{}\"\n", entry.id, entry.version);
        let config_path = config_dir.join(format!("{}.toml", entry.id));
        tokio::fs::write(&config_path, cap_config).await.map_err(|e| InstallError::Io(e.to_string()))?;

        let mcp_deps = entry.manifest.mcp_dependencies.clone();
        tracing::info!("Installed capability `{}` v{}", entry.id, entry.version);
        Ok((plugin_dir, mcp_deps))
    }

    /// Uninstall a capability: remove its plugin directory and config.
    ///
    /// Returns a list of MCP server IDs that were dependencies and may now be unused.
    pub async fn uninstall_capability(&self, id: &str) -> Result<Vec<String>, InstallError> {
        let plugin_dir = self.data_dir.join("plugins").join(id);

        // Read the manifest before deleting so we can report MCP dependencies
        let mcp_deps = self.read_capability_mcp_deps(&plugin_dir).await;

        if plugin_dir.exists() {
            tokio::fs::remove_dir_all(&plugin_dir).await.map_err(|e| InstallError::Io(e.to_string()))?;
        }

        let config_path = self.data_dir.join("config").join("capabilities").join(format!("{id}.toml"));
        if config_path.exists() {
            tokio::fs::remove_file(&config_path).await.map_err(|e| InstallError::Io(e.to_string()))?;
        }

        tracing::info!("Uninstalled capability `{id}`");
        Ok(mcp_deps)
    }

    // =========================================================================
    // Internal helpers
    // =========================================================================

    /// Download a binary, verify its SHA-256, and write it to `dest_dir/dest_name`.
    async fn download_and_verify(
        &self,
        url_template: &str,
        sha256_map: &HashMap<String, String>,
        version: &str,
        dest_dir: &Path,
        dest_name: &str,
    ) -> Result<(), InstallError> {
        let (platform, arch) = platform_arch();
        let platform_key = format!("{platform}-{arch}");
        let expected_sha256 = sha256_map.get(&platform_key).ok_or_else(|| InstallError::UnsupportedPlatform(platform_key.clone()))?;

        let expanded = url_template
            .replace("{version}", version)
            .replace("{platform}", platform)
            .replace("{arch}", arch)
            .replace("{bin_dir}", &dest_dir.to_string_lossy());

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

        use sha2::Digest;
        let digest = sha2::Sha256::digest(&bytes);
        let hex_digest = format!("{digest:x}");
        if hex_digest != *expected_sha256 {
            return Err(InstallError::ChecksumMismatch { expected: expected_sha256.clone(), actual: hex_digest });
        }

        let dest = dest_dir.join(dest_name);
        tokio::fs::write(&dest, &bytes).await.map_err(|e| InstallError::Io(e.to_string()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&dest, perms).map_err(|e| InstallError::Io(e.to_string()))?;
        }

        Ok(())
    }

    /// Generate a `manifest.toml` string from a capability's inline manifest.
    fn generate_capability_manifest(&self, entry: &CapabilityEntry) -> String {
        let m = &entry.manifest;
        let binary_name = match &entry.install {
            InstallMethod::Binary { binary_name, .. } => binary_name.clone(),
            _ => entry.id.clone(),
        };
        let mut lines = vec![
            "[plugin]".into(),
            format!("name = \"{}\"", entry.id),
            format!("version = \"{}\"", entry.version),
            format!("description = \"{}\"", entry.description),
            format!("protocol_version = \"{}\"", entry.protocol_version),
            format!("binary_name = \"{binary_name}\""),
            format!("content_type = \"{}\"", m.content_type),
        ];

        if m.execution_mode != corre_sdk::manifest::ExecutionMode::Oneshot {
            lines.push("execution_mode = \"daemon\"".into());
        }

        if m.defaults.schedule.is_some() || m.defaults.config_path.is_some() || m.defaults.config_schema.is_some() {
            lines.push(String::new());
            lines.push("[plugin.defaults]".into());
            if let Some(ref sched) = m.defaults.schedule {
                lines.push(format!("schedule = \"{sched}\""));
            }
            if let Some(ref path) = m.defaults.config_path {
                lines.push(format!("config_path = \"{path}\""));
            }
        }

        // config_schema is a nested structure — serialize it via toml to avoid
        // hand-building deeply nested TOML arrays-of-tables.
        if let Some(ref schema) = m.defaults.config_schema {
            #[derive(serde::Serialize)]
            struct Wrapper<'a> {
                plugin: WrapperPlugin<'a>,
            }
            #[derive(serde::Serialize)]
            struct WrapperPlugin<'a> {
                defaults: WrapperDefaults<'a>,
            }
            #[derive(serde::Serialize)]
            struct WrapperDefaults<'a> {
                config_schema: &'a corre_sdk::manifest::ConfigSchema,
            }
            let wrapper = Wrapper { plugin: WrapperPlugin { defaults: WrapperDefaults { config_schema: schema } } };
            if let Ok(fragment) = toml::to_string(&wrapper) {
                // toml::to_string produces keys under [plugin.defaults.config_schema]
                // which merges cleanly with the [plugin.defaults] block above.
                lines.push(String::new());
                lines.push(fragment.trim_end().to_string());
            }
        }

        lines.push(String::new());
        lines.push("[plugin.permissions]".into());
        if !m.permissions.mcp_servers.is_empty() {
            let servers: Vec<String> = m.permissions.mcp_servers.iter().map(|s| format!("\"{s}\"")).collect();
            lines.push(format!("mcp_servers = [{}]", servers.join(", ")));
        }
        lines.push(format!("llm_access = {}", m.permissions.llm_access));
        lines.push(format!("max_concurrent_llm = {}", m.permissions.max_concurrent_llm));

        for output in &m.permissions.outputs {
            lines.push(String::new());
            lines.push("[[plugin.permissions.outputs]]".into());
            lines.push(format!("output_type = \"{}\"", serde_json::to_value(&output.output_type).unwrap().as_str().unwrap()));
            lines.push(format!("target = \"{}\"", output.target));
            if let Some(ref ct) = output.content_type {
                lines.push(format!("content_type = \"{ct}\""));
            }
        }

        if let Some(ref sandbox) = m.permissions.sandbox {
            lines.push(String::new());
            lines.push("[plugin.permissions.sandbox]".into());
            if !sandbox.network.is_empty() {
                let vals: Vec<String> = sandbox.network.iter().map(|s| format!("\"{s}\"")).collect();
                lines.push(format!("network = [{}]", vals.join(", ")));
            }
            if !sandbox.filesystem_read.is_empty() {
                let vals: Vec<String> = sandbox.filesystem_read.iter().map(|s| format!("\"{s}\"")).collect();
                lines.push(format!("filesystem_read = [{}]", vals.join(", ")));
            }
            if !sandbox.filesystem_write.is_empty() {
                let vals: Vec<String> = sandbox.filesystem_write.iter().map(|s| format!("\"{s}\"")).collect();
                lines.push(format!("filesystem_write = [{}]", vals.join(", ")));
            }
            if let Some(dns) = sandbox.dns {
                lines.push(format!("dns = {dns}"));
            }
            if let Some(mem) = sandbox.max_memory_mb {
                lines.push(format!("max_memory_mb = {mem}"));
            }
            if let Some(cpu) = sandbox.max_cpu_secs {
                lines.push(format!("max_cpu_secs = {cpu}"));
            }
        }

        for link in &m.links {
            lines.push(String::new());
            lines.push("[[plugin.links]]".into());
            lines.push(format!("label = \"{}\"", link.label));
            lines.push(format!("url = \"{}\"", link.url));
            if let Some(ref icon) = link.icon {
                lines.push(format!("icon = \"{icon}\""));
            }
        }

        for svc in &m.services {
            lines.push(String::new());
            lines.push("[[plugin.services]]".into());
            lines.push(format!("name = \"{}\"", svc.name));
            lines.push(format!("description = \"{}\"", svc.description));
            lines.push(format!("image = \"{}\"", svc.image));
            if !svc.ports.is_empty() {
                let ports: Vec<String> = svc.ports.iter().map(|p| format!("\"{p}\"")).collect();
                lines.push(format!("ports = [{}]", ports.join(", ")));
            }
            if !svc.volumes.is_empty() {
                let vols: Vec<String> = svc.volumes.iter().map(|v| format!("\"{v}\"")).collect();
                lines.push(format!("volumes = [{}]", vols.join(", ")));
            }
            if svc.optional {
                lines.push("optional = true".into());
            }
        }

        lines.push(String::new());
        lines.join("\n")
    }

    /// Read MCP dependency IDs from a capability's manifest.toml (best-effort).
    async fn read_capability_mcp_deps(&self, plugin_dir: &Path) -> Vec<String> {
        let manifest_path = plugin_dir.join("manifest.toml");
        let Ok(content) = tokio::fs::read_to_string(&manifest_path).await else { return Vec::new() };
        let Ok(manifest) = toml::from_str::<corre_sdk::manifest::PluginManifest>(&content) else { return Vec::new() };
        manifest.plugin.permissions.mcp_servers
    }
}

fn platform_arch() -> (&'static str, &'static str) {
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
    (platform, arch)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{CapabilityDefaults, CapabilityManifestInline, CapabilityPermissions};
    use corre_sdk::manifest::{ConfigField, ConfigFieldType, ConfigSchema};

    /// Helper: build a minimal `CapabilityEntry` with an optional config_schema.
    fn entry_with_schema(schema: Option<ConfigSchema>) -> CapabilityEntry {
        CapabilityEntry {
            id: "daily-brief".into(),
            name: "Daily Brief".into(),
            description: "test".into(),
            version: "1.0.0".into(),
            protocol_version: "1.0".into(),
            install: InstallMethod::Binary {
                download_url_template: String::new(),
                binary_name: "daily-brief".into(),
                sha256: HashMap::new(),
                command: "daily-brief".into(),
                args: vec![],
            },
            manifest: CapabilityManifestInline {
                content_type: "newspaper".into(),
                execution_mode: Default::default(),
                defaults: CapabilityDefaults {
                    schedule: Some("0 0 5 * * *".into()),
                    config_path: Some("config/topics.yml".into()),
                    config_schema: schema,
                },
                permissions: CapabilityPermissions::default(),
                mcp_dependencies: vec![],
                services: vec![],
                links: vec![],
            },
            tags: vec![],
            verified: false,
        }
    }

    #[test]
    fn generated_manifest_includes_config_schema() {
        let schema = ConfigSchema {
            root_key: Some("daily-briefing".into()),
            format: "yaml".into(),
            fields: vec![ConfigField {
                key: "sections".into(),
                field_type: ConfigFieldType::List,
                label: Some("Sections".into()),
                options: vec![],
                default: None,
                fields: vec![
                    ConfigField {
                        key: "title".into(),
                        field_type: ConfigFieldType::Text,
                        label: Some("Section title".into()),
                        options: vec![],
                        default: None,
                        fields: vec![],
                    },
                    ConfigField {
                        key: "sources".into(),
                        field_type: ConfigFieldType::List,
                        label: Some("Sources".into()),
                        options: vec![],
                        default: None,
                        fields: vec![ConfigField {
                            key: "search".into(),
                            field_type: ConfigFieldType::Text,
                            label: Some("Search query".into()),
                            options: vec![],
                            default: None,
                            fields: vec![],
                        }],
                    },
                ],
            }],
        };

        let data_dir = std::path::Path::new("/tmp/test-installer");
        let installer = McpInstaller::new(data_dir.to_path_buf(), "https://example.com".into());
        let entry = entry_with_schema(Some(schema));
        let manifest_toml = installer.generate_capability_manifest(&entry);

        // Must parse back as a valid PluginManifest
        let parsed: corre_sdk::manifest::PluginManifest =
            toml::from_str(&manifest_toml).unwrap_or_else(|e| panic!("generated manifest is invalid TOML:\n{manifest_toml}\nerror: {e}"));

        let s = parsed.plugin.defaults.config_schema.expect("config_schema should be present after round-trip");
        assert_eq!(s.root_key.as_deref(), Some("daily-briefing"));
        assert_eq!(s.format, "yaml");
        assert_eq!(s.fields.len(), 1);
        assert_eq!(s.fields[0].key, "sections");
        assert_eq!(s.fields[0].fields.len(), 2);
        assert_eq!(s.fields[0].fields[1].key, "sources");
        assert_eq!(s.fields[0].fields[1].fields.len(), 1);
    }

    #[test]
    fn generated_manifest_without_schema_still_valid() {
        let data_dir = std::path::Path::new("/tmp/test-installer");
        let installer = McpInstaller::new(data_dir.to_path_buf(), "https://example.com".into());
        let entry = entry_with_schema(None);
        let manifest_toml = installer.generate_capability_manifest(&entry);

        let parsed: corre_sdk::manifest::PluginManifest =
            toml::from_str(&manifest_toml).unwrap_or_else(|e| panic!("generated manifest is invalid TOML:\n{manifest_toml}\nerror: {e}"));
        assert!(parsed.plugin.defaults.config_schema.is_none());
    }
}
