//! Plugin discovery and manifest loading.

use corre_sdk::manifest::PluginManifest;
use std::path::{Path, PathBuf};

/// Information about a discovered plugin on disk.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// Parsed manifest.
    pub manifest: PluginManifest,
    /// Absolute path to the plugin directory.
    pub dir: PathBuf,
    /// Absolute path to the plugin binary.
    pub binary: PathBuf,
}

/// Scan `{data_dir}/plugins/` for directories containing a `manifest.toml`
/// and a `bin/capability` executable. Returns all successfully loaded plugins.
pub fn discover_plugins(data_dir: &Path) -> Vec<DiscoveredPlugin> {
    let plugins_dir = data_dir.join("plugins");
    if !plugins_dir.is_dir() {
        return vec![];
    }

    let entries = match std::fs::read_dir(&plugins_dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("Failed to read plugins directory {}: {e}", plugins_dir.display());
            return vec![];
        }
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let dir = entry.path();
            if !dir.is_dir() {
                return None;
            }
            match load_plugin(&dir) {
                Ok(plugin) => Some(plugin),
                Err(e) => {
                    tracing::warn!("Skipping plugin at {}: {e}", dir.display());
                    None
                }
            }
        })
        .collect()
}

/// Load a single plugin from a directory.
pub fn load_plugin(dir: &Path) -> anyhow::Result<DiscoveredPlugin> {
    let manifest_path = dir.join("manifest.toml");
    if !manifest_path.exists() {
        anyhow::bail!("no manifest.toml found");
    }

    let manifest = PluginManifest::load(&manifest_path)?;

    let bin_name = manifest.plugin.binary_name.as_deref().unwrap_or("capability");
    let binary = dir.join(format!("bin/{bin_name}"));
    if !binary.exists() {
        anyhow::bail!("no bin/{bin_name} executable found");
    }

    Ok(DiscoveredPlugin { manifest, dir: dir.to_path_buf(), binary })
}

/// Convert a discovered plugin into a `CapabilityConfig` entry, using
/// manifest defaults for any fields not already in the user's config.
pub fn plugin_to_capability_config(plugin: &DiscoveredPlugin) -> crate::config::CapabilityConfig {
    let meta = &plugin.manifest.plugin;
    crate::config::CapabilityConfig {
        name: meta.name.clone(),
        description: meta.description.clone(),
        schedule: meta.defaults.schedule.clone().unwrap_or_else(|| "0 0 5 * * *".into()),
        mcp_servers: meta.permissions.mcp_servers.clone(),
        config_path: meta.defaults.config_path.clone(),
        enabled: true,
        llm: None,
        plugin: Some(plugin.dir.to_string_lossy().into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discover_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = discover_plugins(tmp.path());
        assert!(plugins.is_empty());
    }

    #[test]
    fn discover_missing_dir() {
        let plugins = discover_plugins(Path::new("/nonexistent/path"));
        assert!(plugins.is_empty());
    }

    #[test]
    fn discover_valid_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("plugins/test-plugin");
        fs::create_dir_all(plugin_dir.join("bin")).unwrap();
        fs::write(
            plugin_dir.join("manifest.toml"),
            r#"
            [plugin]
            name = "test-plugin"
            version = "0.1.0"
            description = "A test plugin"
            "#,
        )
        .unwrap();
        fs::write(plugin_dir.join("bin/capability"), "#!/bin/sh\necho hello").unwrap();

        let plugins = discover_plugins(tmp.path());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].manifest.plugin.name, "test-plugin");
    }

    #[test]
    fn discover_skips_invalid_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("plugins/broken-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        // No manifest.toml — should be skipped
        fs::write(plugin_dir.join("README.md"), "not a plugin").unwrap();

        let plugins = discover_plugins(tmp.path());
        assert!(plugins.is_empty());
    }
}
