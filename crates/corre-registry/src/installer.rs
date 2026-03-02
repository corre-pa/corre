//! MCP server and capability installer/uninstaller.
//!
//! [`McpInstaller`] handles MCP servers (converting [`McpRegistryEntry`] into `McpServerConfig`)
//! and capabilities (downloading binaries to `{data_dir}/plugins/{id}/bin/`, generating
//! `manifest.toml`, and auto-installing MCP dependencies).

use crate::manifest::{CapabilityEntry, InstallMethod, McpRegistryEntry};
use corre_core::config::McpServerConfig;
use corre_sdk::manifest::{PluginDefaults, PluginManifest, PluginMeta, PluginPermissions};
use corre_sdk::types::ContentType;
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
                tokio::fs::create_dir_all(&bin_dir).await?;
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
            tokio::fs::remove_file(&binary_path).await?;
        } else if server_config.command == "npx" {
            if let Some(pkg) = server_config.args.iter().find(|a| !a.starts_with('-')) {
                let output = tokio::process::Command::new("npm").args(["uninstall", "-g", pkg]).output().await?;
                if !output.status.success() {
                    tracing::warn!("npm uninstall -g {pkg} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
                }
            }
        } else if ["uvx", "pipx"].contains(&server_config.command.as_str())
            && let Some(pkg) = server_config.args.first()
        {
            let output = tokio::process::Command::new("pip").args(["uninstall", "-y", pkg]).output().await?;
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
        tokio::fs::create_dir_all(&bin_dir).await?;

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
        tokio::fs::write(&manifest_path, manifest_toml).await?;

        // Write per-capability config file
        let config_dir = self.data_dir.join("config").join("capabilities");
        tokio::fs::create_dir_all(&config_dir).await?;

        let cap_config = format!("installed = true\nregistry_id = \"{}\"\nversion = \"{}\"\n", entry.id, entry.version);
        let config_path = config_dir.join(format!("{}.toml", entry.id));
        tokio::fs::write(&config_path, cap_config).await?;

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
            tokio::fs::remove_dir_all(&plugin_dir).await?;
        }

        let config_path = self.data_dir.join("config").join("capabilities").join(format!("{id}.toml"));
        if config_path.exists() {
            tokio::fs::remove_file(&config_path).await?;
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
        tokio::fs::write(&dest, &bytes).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&dest, perms)?;
        }

        Ok(())
    }

    /// Generate a `manifest.toml` string from a capability's inline manifest.
    ///
    /// Builds a [`PluginManifest`] and serializes it with `toml::to_string_pretty`
    /// so that all field values (descriptions, paths, URLs) are properly escaped.
    fn generate_capability_manifest(&self, entry: &CapabilityEntry) -> String {
        let m = &entry.manifest;
        let binary_name = match &entry.install {
            InstallMethod::Binary { binary_name, .. } => binary_name.clone(),
            _ => entry.id.clone(),
        };

        let content_type = serde_json::from_value::<ContentType>(serde_json::Value::String(m.content_type.clone())).unwrap_or_default();

        let manifest = PluginManifest {
            plugin: PluginMeta {
                name: entry.id.clone(),
                version: entry.version.clone(),
                description: entry.description.clone(),
                min_host_version: None,
                protocol_version: entry.protocol_version.clone(),
                binary_name: Some(binary_name),
                content_type,
                execution_mode: m.execution_mode.clone(),
                defaults: PluginDefaults {
                    schedule: m.defaults.schedule.clone(),
                    config_path: m.defaults.config_path.clone(),
                    config_schema: m.defaults.config_schema.clone(),
                },
                permissions: PluginPermissions {
                    mcp_servers: m.permissions.mcp_servers.clone(),
                    llm_access: m.permissions.llm_access,
                    max_concurrent_llm: m.permissions.max_concurrent_llm,
                    outputs: m.permissions.outputs.clone(),
                    sandbox: m.permissions.sandbox.clone(),
                },
                services: m.services.clone(),
                links: m.links.clone(),
            },
            mcp_dependencies: HashMap::new(),
        };

        toml::to_string_pretty(&manifest).expect("PluginManifest serialization should not fail")
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

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{CapabilityDefaults, CapabilityManifestInline, CapabilityPermissions};
    use corre_sdk::manifest::{
        ConfigField, ConfigFieldType, ConfigSchema, OutputDeclaration, OutputType, PluginLink, SandboxPermissions, ServiceDeclaration,
    };

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

    #[test]
    fn manifest_handles_special_characters() {
        let data_dir = std::path::Path::new("/tmp/test-installer");
        let installer = McpInstaller::new(data_dir.to_path_buf(), "https://example.com".into());
        let mut entry = entry_with_schema(None);
        entry.description = "A \"quoted\" description with \\ backslashes\nand newlines".into();

        let manifest_toml = installer.generate_capability_manifest(&entry);
        let parsed: corre_sdk::manifest::PluginManifest =
            toml::from_str(&manifest_toml).unwrap_or_else(|e| panic!("generated manifest is invalid TOML:\n{manifest_toml}\nerror: {e}"));
        assert_eq!(parsed.plugin.description, "A \"quoted\" description with \\ backslashes\nand newlines");
    }

    #[test]
    fn manifest_round_trips_outputs_sandbox_links_services() {
        let data_dir = std::path::Path::new("/tmp/test-installer");
        let installer = McpInstaller::new(data_dir.to_path_buf(), "https://example.com".into());
        let mut entry = entry_with_schema(None);
        entry.manifest.permissions = CapabilityPermissions {
            mcp_servers: vec!["brave-search".into()],
            llm_access: true,
            max_concurrent_llm: 5,
            outputs: vec![OutputDeclaration {
                output_type: OutputType::Filesystem,
                target: "{data_dir}/editions/{date}/edition.json".into(),
                content_type: Some("application/json".into()),
            }],
            sandbox: Some(SandboxPermissions {
                network: vec!["api.example.com:443".into()],
                filesystem_read: vec!["{config_dir}".into()],
                filesystem_write: vec!["{data_dir}/editions".into()],
                dns: Some(true),
                max_memory_mb: Some(512),
                max_cpu_secs: Some(60),
            }),
        };
        entry.manifest.links = vec![PluginLink { label: "Homepage".into(), url: "https://example.com".into(), icon: Some("home".into()) }];
        entry.manifest.services = vec![ServiceDeclaration {
            name: "web-ui".into(),
            description: "A service with \"quotes\" in its description".into(),
            image: "ghcr.io/example/web:latest".into(),
            ports: vec!["8080:80".into()],
            volumes: vec!["{data_dir}/editions:/data:ro".into()],
            env: HashMap::new(),
            optional: true,
            health_check: None,
        }];

        let manifest_toml = installer.generate_capability_manifest(&entry);
        let parsed: corre_sdk::manifest::PluginManifest =
            toml::from_str(&manifest_toml).unwrap_or_else(|e| panic!("generated manifest is invalid TOML:\n{manifest_toml}\nerror: {e}"));

        assert_eq!(parsed.plugin.permissions.mcp_servers, vec!["brave-search"]);
        assert_eq!(parsed.plugin.permissions.max_concurrent_llm, 5);
        assert_eq!(parsed.plugin.permissions.outputs.len(), 1);
        assert_eq!(parsed.plugin.permissions.outputs[0].output_type, OutputType::Filesystem);
        assert_eq!(parsed.plugin.permissions.outputs[0].content_type.as_deref(), Some("application/json"));

        let sandbox = parsed.plugin.permissions.sandbox.as_ref().expect("sandbox should be present");
        assert_eq!(sandbox.network, vec!["api.example.com:443"]);
        assert_eq!(sandbox.dns, Some(true));
        assert_eq!(sandbox.max_memory_mb, Some(512));

        assert_eq!(parsed.plugin.links.len(), 1);
        assert_eq!(parsed.plugin.links[0].icon.as_deref(), Some("home"));

        assert_eq!(parsed.plugin.services.len(), 1);
        assert_eq!(parsed.plugin.services[0].description, "A service with \"quotes\" in its description");
        assert!(parsed.plugin.services[0].optional);
    }
}
