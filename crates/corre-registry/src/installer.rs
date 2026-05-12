//! MCP server and app installer/uninstaller.
//!
//! [`McpInstaller`] handles MCP servers (converting [`McpRegistryEntry`] into `McpServerConfig`)
//! and apps (downloading binaries to `{data_dir}/plugins/{id}/bin/`, generating
//! `manifest.toml`, and auto-installing MCP dependencies).

use crate::manifest::{AppEntry, InstallMethod, McpRegistryEntry};
use corre_core::config::McpServerConfig;
use corre_sdk::manifest::{PluginDefaults, PluginManifest, PluginMeta, PluginPermissions};
use corre_sdk::types::ContentType;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// Return `Ok(())` iff `s` parses as exactly one [`Component::Normal`] — non-empty,
/// no `.`, no `..`, no path separator, no absolute root, no Windows prefix, no NUL
/// or other ASCII control byte.
///
/// `field` is a static label used in the error message so the caller can tell the
/// user (and the logs) which input was rejected.
fn validate_path_component(field: &'static str, s: &str) -> Result<(), InstallError> {
    let reject = || InstallError::InvalidPathComponent { field, value: s.to_string() };
    // Reject NUL and other C0 control bytes (newline, tab, …). On Linux these are
    // valid filename bytes, so `Path::new("foo\nbar").components()` would yield a
    // single Normal component and pass — but a path with a control character in
    // it produces confusing log output and downstream-tool behaviour, so refuse
    // it here. (NUL must be rejected anyway: `Path::components()` swallows it.)
    if s.bytes().any(|b| b < 0x20) {
        return Err(reject());
    }
    let mut it = Path::new(s).components();
    let ok = matches!(
        (it.next(), it.next()),
        (Some(Component::Normal(part)), None) if !part.is_empty()
    );
    if ok { Ok(()) } else { Err(reject()) }
}

/// Strip the literal `{bin_dir}/` prefix from `command` if present and return
/// the remainder. The remainder is what gets stored in `McpServerConfig.command`
/// and later resolved against `bin_dir` at spawn time; it must be a single safe
/// path component, which the caller validates.
fn bare_install_command(command: &str) -> &str {
    command.strip_prefix("{bin_dir}/").unwrap_or(command)
}

/// Resolve a stored `McpServerConfig.command` to the binary it manages inside
/// `bin_dir`, if any. Accepts:
///   * a bare filename (the canonical post-install layout), or
///   * an absolute path whose direct parent is `bin_dir` (preserves the
///     uninstall path for older installs that stored an expanded absolute path
///     in the config).
///
/// Returns `None` for wrapper commands (`npx`, `uvx`, `pipx`) and for anything
/// that does not pass [`validate_path_component`] on the last segment.
fn binary_under_bin_dir(command: &str, bin_dir: &Path) -> Option<PathBuf> {
    let cmd_path = Path::new(command);
    if cmd_path.is_absolute() {
        let rel = cmd_path.strip_prefix(bin_dir).ok()?;
        let name = rel.to_str()?;
        validate_path_component("server.command", name).ok()?;
        Some(bin_dir.join(name))
    } else if validate_path_component("server.command", command).is_ok() {
        Some(bin_dir.join(command))
    } else {
        None
    }
}

/// Handles installing and uninstalling MCP servers and apps from registry entries.
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
        validate_path_component("registry_id", &entry.id)?;
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
                validate_path_component("binary_name", binary_name)?;
                // Strip the optional `{bin_dir}/` prefix and validate the remainder.
                // Anything containing `{bin_dir}` elsewhere (e.g. `{bin_dir}/../sh`)
                // collapses to a multi-component / traversal path here and is rejected.
                let bare_command = bare_install_command(command);
                validate_path_component("install.command", bare_command)?;

                let bin_dir = self.data_dir.join("bin");
                tokio::fs::create_dir_all(&bin_dir).await?;
                self.download_and_verify(download_url_template, sha256, &entry.version, &bin_dir, binary_name).await?;

                let bin_dir_str = bin_dir.to_string_lossy();
                let resolved_args: Vec<String> = args.iter().map(|a| a.replace("{bin_dir}", &bin_dir_str)).collect();

                // Store the bare binary name; `McpServerConfig::with_resolved_command`
                // (in corre-core) prepends `bin_dir` at spawn time, and `uninstall`
                // joins against `bin_dir` below.
                Ok(McpServerConfig {
                    command: bare_command.to_string(),
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
        validate_path_component("server_name", server_name)?;
        let bin_dir = self.data_dir.join("bin");
        // `server_config.command` may legitimately be a package-manager wrapper
        // (`npx`, `uvx`, `pipx`) or, for older installs, an absolute path that
        // expands to `{bin_dir}/{name}`. `binary_under_bin_dir` accepts both the
        // canonical bare-filename form and the legacy absolute-under-bin_dir
        // form, and rejects anything else; for wrappers / out-of-tree commands
        // it returns `None` and we fall through to the package-manager branches.
        let removed_binary = if let Some(binary_path) = binary_under_bin_dir(&server_config.command, &bin_dir) {
            if binary_path.exists() {
                tokio::fs::remove_file(&binary_path).await?;
                true
            } else {
                false
            }
        } else {
            tracing::warn!(
                "MCP server `{server_name}` command {:?} is not under bin_dir; skipping binary removal",
                server_config.command,
            );
            false
        };

        if !removed_binary {
            if server_config.command == "npx" {
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
        }

        tracing::info!("Uninstalled MCP server `{server_name}`");
        Ok(())
    }

    // =========================================================================
    // App install/uninstall
    // =========================================================================

    /// Install an app from a registry entry.
    ///
    /// 1. Downloads the binary using the existing `InstallMethod` logic
    /// 2. Places it at `{data_dir}/plugins/{id}/bin/app`
    /// 3. Generates `manifest.toml` from the inline manifest
    /// 4. Writes per-app config at `{data_dir}/config/apps/{id}.toml`
    ///
    /// Returns `(plugin_dir, mcp_deps)` where `mcp_deps` is the list of MCP server IDs
    /// that this app depends on. The caller should install any that are missing.
    pub async fn install_app(&self, entry: &AppEntry) -> Result<(PathBuf, Vec<String>), InstallError> {
        validate_path_component("app_id", &entry.id)?;
        if let InstallMethod::Binary { binary_name, .. } = &entry.install {
            validate_path_component("binary_name", binary_name)?;
        }
        let plugin_dir = self.data_dir.join("plugins").join(&entry.id);
        let bin_dir = plugin_dir.join("bin");
        tokio::fs::create_dir_all(&bin_dir).await?;

        match &entry.install {
            InstallMethod::Binary { download_url_template, binary_name, sha256, .. } => {
                self.download_and_verify(download_url_template, sha256, &entry.version, &bin_dir, binary_name).await?;
            }
            _ => {
                return Err(InstallError::Io("only binary install method is supported for apps".into()));
            }
        }

        // Generate manifest.toml from the inline manifest
        let manifest_toml = self.generate_app_manifest(entry);
        let manifest_path = plugin_dir.join("manifest.toml");
        tokio::fs::write(&manifest_path, manifest_toml).await?;

        // Write per-app config file
        let config_dir = self.data_dir.join("config").join("apps");
        tokio::fs::create_dir_all(&config_dir).await?;

        let cap_config = format!("installed = true\nregistry_id = \"{}\"\nversion = \"{}\"\n", entry.id, entry.version);
        let config_path = config_dir.join(format!("{}.toml", entry.id));
        tokio::fs::write(&config_path, cap_config).await?;

        let mcp_deps = entry.manifest.mcp_dependencies.clone();
        tracing::info!("Installed app `{}` v{}", entry.id, entry.version);
        Ok((plugin_dir, mcp_deps))
    }

    /// Uninstall an app: remove its plugin directory and config.
    ///
    /// Returns a list of MCP server IDs that were dependencies and may now be unused.
    pub async fn uninstall_app(&self, id: &str) -> Result<Vec<String>, InstallError> {
        validate_path_component("app_id", id)?;
        let plugin_dir = self.data_dir.join("plugins").join(id);

        // Read the manifest before deleting so we can report MCP dependencies
        let mcp_deps = self.read_app_mcp_deps(&plugin_dir).await;

        if plugin_dir.exists() {
            tokio::fs::remove_dir_all(&plugin_dir).await?;
        }

        let config_path = self.data_dir.join("config").join("apps").join(format!("{id}.toml"));
        if config_path.exists() {
            tokio::fs::remove_file(&config_path).await?;
        }

        tracing::info!("Uninstalled app `{id}`");
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
        validate_path_component("dest_name", dest_name)?;
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

    /// Generate a `manifest.toml` string from an app's inline manifest.
    ///
    /// Builds a [`PluginManifest`] and serializes it with `toml::to_string_pretty`
    /// so that all field values (descriptions, paths, URLs) are properly escaped.
    fn generate_app_manifest(&self, entry: &AppEntry) -> String {
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

    /// Read MCP dependency IDs from an app's manifest.toml (best-effort).
    async fn read_app_mcp_deps(&self, plugin_dir: &Path) -> Vec<String> {
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
    #[error("invalid path component {field}: {value:?}")]
    InvalidPathComponent { field: &'static str, value: String },
}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{AppDefaults, AppManifestInline, AppPermissions};
    use corre_sdk::manifest::{
        ConfigField, ConfigFieldType, ConfigSchema, OutputDeclaration, OutputType, PluginLink, SandboxPermissions, ServiceDeclaration,
    };

    /// Helper: build a minimal [`AppEntry`] with an optional config_schema.
    fn entry_with_schema(schema: Option<ConfigSchema>) -> AppEntry {
        AppEntry {
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
            manifest: AppManifestInline {
                content_type: "newspaper".into(),
                execution_mode: Default::default(),
                defaults: AppDefaults {
                    schedule: Some("0 0 5 * * *".into()),
                    config_path: Some("config/topics.yml".into()),
                    config_schema: schema,
                },
                permissions: AppPermissions::default(),
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
        let manifest_toml = installer.generate_app_manifest(&entry);

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
        let manifest_toml = installer.generate_app_manifest(&entry);

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

        let manifest_toml = installer.generate_app_manifest(&entry);
        let parsed: corre_sdk::manifest::PluginManifest =
            toml::from_str(&manifest_toml).unwrap_or_else(|e| panic!("generated manifest is invalid TOML:\n{manifest_toml}\nerror: {e}"));
        assert_eq!(parsed.plugin.description, "A \"quoted\" description with \\ backslashes\nand newlines");
    }

    #[test]
    fn manifest_round_trips_outputs_sandbox_links_services() {
        let data_dir = std::path::Path::new("/tmp/test-installer");
        let installer = McpInstaller::new(data_dir.to_path_buf(), "https://example.com".into());
        let mut entry = entry_with_schema(None);
        entry.manifest.permissions = AppPermissions {
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

        let manifest_toml = installer.generate_app_manifest(&entry);
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

    // -------------------------------------------------------------------------
    // validate_path_component — the sanitiser CodeQL alert #5 hangs the fix on.
    // -------------------------------------------------------------------------

    #[test]
    fn validator_accepts_simple_id() {
        assert!(validate_path_component("field", "daily-brief").is_ok());
    }

    #[test]
    fn validator_accepts_dotted_id() {
        assert!(validate_path_component("field", "some.app").is_ok());
    }

    #[test]
    fn validator_rejects_empty() {
        assert!(validate_path_component("field", "").is_err());
    }

    #[test]
    fn validator_rejects_parent_ref() {
        assert!(validate_path_component("field", "..").is_err());
    }

    #[test]
    fn validator_rejects_current_ref() {
        assert!(validate_path_component("field", ".").is_err());
    }

    #[test]
    fn validator_rejects_nested_parent() {
        assert!(validate_path_component("field", "foo/../bar").is_err());
    }

    #[test]
    fn validator_rejects_forward_slash() {
        assert!(validate_path_component("field", "foo/bar").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn validator_rejects_backslash() {
        assert!(validate_path_component("field", "foo\\bar").is_err());
    }

    #[test]
    fn validator_rejects_absolute_unix() {
        assert!(validate_path_component("field", "/etc/passwd").is_err());
    }

    #[test]
    fn validator_rejects_leading_double_slash() {
        assert!(validate_path_component("field", "//foo").is_err());
    }

    #[test]
    fn validator_rejects_nul_byte() {
        assert!(validate_path_component("field", "foo\0bar").is_err());
    }

    #[test]
    fn validator_error_carries_field_label() {
        let err = validate_path_component("registry_id", "..").unwrap_err();
        match err {
            InstallError::InvalidPathComponent { field, value } => {
                assert_eq!(field, "registry_id");
                assert_eq!(value, "..");
            }
            other => panic!("expected InvalidPathComponent, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // Entry-point rejection tests — drive each installer method against a
    // tempdir and assert that hostile inputs are rejected before any
    // filesystem-mutating call runs.
    // -------------------------------------------------------------------------

    use crate::manifest::McpRegistryEntry;
    use tempfile::TempDir;

    fn installer_with_tempdir() -> (McpInstaller, TempDir) {
        let tempdir = TempDir::new().expect("create tempdir");
        let installer = McpInstaller::new(tempdir.path().to_path_buf(), "https://example.com".into());
        (installer, tempdir)
    }

    fn assert_invalid_path_component(err: &InstallError, expected_field: &str) {
        match err {
            InstallError::InvalidPathComponent { field, .. } => {
                assert_eq!(*field, expected_field, "wrong field rejected: {err:?}");
            }
            other => panic!("expected InvalidPathComponent, got {other:?}"),
        }
    }

    fn assert_tempdir_empty(dir: &TempDir) {
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(entries.is_empty(), "tempdir should be empty but contains {} entries", entries.len());
    }

    fn hostile_mcp_entry(id: &str, binary_name: &str, command: &str) -> McpRegistryEntry {
        McpRegistryEntry {
            id: id.into(),
            name: "evil".into(),
            description: "evil".into(),
            version: "1.0.0".into(),
            install: InstallMethod::Binary {
                download_url_template: "https://example.invalid/{version}".into(),
                binary_name: binary_name.into(),
                sha256: HashMap::new(),
                command: command.into(),
                args: vec![],
            },
            config: vec![],
            homepage: None,
            tags: vec![],
            verified: false,
        }
    }

    #[tokio::test]
    async fn install_rejects_traversal_in_registry_id() {
        let (installer, tempdir) = installer_with_tempdir();
        let entry = hostile_mcp_entry("../../etc", "ok", "ok");
        let err = installer.install(&entry, &HashMap::new()).await.unwrap_err();
        assert_invalid_path_component(&err, "registry_id");
        assert_tempdir_empty(&tempdir);
    }

    #[tokio::test]
    async fn install_rejects_traversal_in_binary_name() {
        let (installer, tempdir) = installer_with_tempdir();
        let entry = hostile_mcp_entry("safe-id", "../../bin/sh", "safe-id");
        let err = installer.install(&entry, &HashMap::new()).await.unwrap_err();
        assert_invalid_path_component(&err, "binary_name");
        assert_tempdir_empty(&tempdir);
    }

    #[tokio::test]
    async fn install_rejects_traversal_in_command() {
        let (installer, tempdir) = installer_with_tempdir();
        let entry = hostile_mcp_entry("safe-id", "safe-bin", "../sh");
        let err = installer.install(&entry, &HashMap::new()).await.unwrap_err();
        assert_invalid_path_component(&err, "install.command");
        assert_tempdir_empty(&tempdir);
    }

    #[tokio::test]
    async fn install_rejects_bin_dir_template_traversal() {
        // `{bin_dir}/../sh` strips the `{bin_dir}/` prefix to leave `../sh`,
        // which fails component validation. The pre-fix code skipped
        // validation entirely whenever `{bin_dir}` appeared anywhere in the
        // command string, opening a command-execution path through a hostile
        // registry entry.
        let (installer, tempdir) = installer_with_tempdir();
        let entry = hostile_mcp_entry("safe-id", "safe-bin", "{bin_dir}/../sh");
        let err = installer.install(&entry, &HashMap::new()).await.unwrap_err();
        assert_invalid_path_component(&err, "install.command");
        assert_tempdir_empty(&tempdir);
    }

    #[test]
    fn validator_rejects_newline() {
        assert!(validate_path_component("field", "foo\nbar").is_err());
    }

    #[test]
    fn validator_rejects_tab() {
        assert!(validate_path_component("field", "foo\tbar").is_err());
    }

    #[tokio::test]
    async fn install_app_rejects_traversal_in_id() {
        let (installer, tempdir) = installer_with_tempdir();
        let mut entry = entry_with_schema(None);
        entry.id = "../../etc".into();
        let err = installer.install_app(&entry).await.unwrap_err();
        assert_invalid_path_component(&err, "app_id");
        assert_tempdir_empty(&tempdir);
    }

    #[tokio::test]
    async fn install_app_rejects_traversal_in_binary_name() {
        let (installer, tempdir) = installer_with_tempdir();
        let mut entry = entry_with_schema(None);
        if let InstallMethod::Binary { binary_name, .. } = &mut entry.install {
            *binary_name = "../../bin/sh".into();
        }
        let err = installer.install_app(&entry).await.unwrap_err();
        assert_invalid_path_component(&err, "binary_name");
        assert_tempdir_empty(&tempdir);
    }

    #[tokio::test]
    async fn uninstall_rejects_traversal_in_server_name() {
        let (installer, tempdir) = installer_with_tempdir();
        let cfg = McpServerConfig {
            registry_id: Some("brave".into()),
            command: "brave".into(),
            args: vec![],
            env: HashMap::new(),
            installed: true,
        };
        let err = installer.uninstall("../../etc", &cfg).await.unwrap_err();
        assert_invalid_path_component(&err, "server_name");
        assert_tempdir_empty(&tempdir);
    }

    #[tokio::test]
    async fn uninstall_removes_binary_for_bare_command() {
        // The canonical post-install layout: `McpServerConfig.command` holds
        // just the binary filename. `uninstall` must resolve it against
        // `bin_dir` and delete the file.
        let (installer, tempdir) = installer_with_tempdir();
        let bin_dir = tempdir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let binary = bin_dir.join("my-binary");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

        let cfg = McpServerConfig {
            registry_id: Some("my-server".into()),
            command: "my-binary".into(),
            args: vec![],
            env: HashMap::new(),
            installed: true,
        };
        installer.uninstall("my-server", &cfg).await.expect("uninstall should succeed");
        assert!(!binary.exists(), "binary should have been removed");
    }

    #[tokio::test]
    async fn uninstall_removes_binary_for_legacy_absolute_command() {
        // Older installs (before the bare-name change) wrote the resolved
        // absolute path into `McpServerConfig.command`. `binary_under_bin_dir`
        // accepts that form when the parent matches `bin_dir`, so `uninstall`
        // must still remove the file.
        let (installer, tempdir) = installer_with_tempdir();
        let bin_dir = tempdir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let binary = bin_dir.join("my-binary");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

        let cfg = McpServerConfig {
            registry_id: Some("my-server".into()),
            command: binary.to_string_lossy().into_owned(),
            args: vec![],
            env: HashMap::new(),
            installed: true,
        };
        installer.uninstall("my-server", &cfg).await.expect("uninstall should succeed");
        assert!(!binary.exists(), "legacy absolute-path binary should have been removed");
    }

    #[tokio::test]
    async fn uninstall_skips_binary_outside_bin_dir() {
        // An absolute path that does not live directly under `{data_dir}/bin/`
        // must not be touched. The function logs a warning and returns Ok.
        let (installer, tempdir) = installer_with_tempdir();
        let bin_dir = tempdir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let outside = tempdir.path().join("outside-binary");
        std::fs::write(&outside, b"#!/bin/sh\n").unwrap();

        let cfg = McpServerConfig {
            registry_id: Some("evil".into()),
            command: outside.to_string_lossy().into_owned(),
            args: vec![],
            env: HashMap::new(),
            installed: true,
        };
        installer.uninstall("evil", &cfg).await.expect("uninstall should succeed");
        assert!(outside.exists(), "binary outside bin_dir must not be removed");
    }

    #[tokio::test]
    async fn uninstall_skips_binary_removal_for_npx() {
        let (installer, tempdir) = installer_with_tempdir();
        // No binary at data_dir/bin/npx exists; the npm-uninstall branch
        // attempts to spawn `npm` which may or may not exist on the test
        // host. We only assert that the validator does not reject "npx"
        // as `server.command` (because that would mask the package-manager
        // branch entirely) — we sidestep the npm subprocess by passing no
        // package name in args, which makes the branch a no-op.
        let cfg = McpServerConfig {
            registry_id: Some("brave".into()),
            command: "npx".into(),
            args: vec!["--flag-only".into()],
            env: HashMap::new(),
            installed: true,
        };
        installer.uninstall("brave-search", &cfg).await.expect("uninstall should succeed");
        // Nothing should have been written under data_dir.
        let bin_dir = tempdir.path().join("bin");
        assert!(!bin_dir.exists(), "uninstall must not create bin_dir on a fresh data_dir");
    }

    #[tokio::test]
    async fn uninstall_app_rejects_traversal_in_id() {
        let (installer, tempdir) = installer_with_tempdir();
        let err = installer.uninstall_app("../../etc").await.unwrap_err();
        assert_invalid_path_component(&err, "app_id");
        assert_tempdir_empty(&tempdir);
    }
}
