//! `AppRegistry`: maps app names to boxed `App` trait objects.
//!
//! Instantiates subprocess-backed plugin apps from `DiscoveredPlugin` entries
//! discovered at startup.

use corre_core::app::App;
use corre_core::config::AppConfig;
use corre_core::plugin::DiscoveredPlugin;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Registry mapping app names to their implementations.
pub struct AppRegistry {
    apps: HashMap<String, Arc<dyn App>>,
}

impl AppRegistry {
    pub fn new() -> Self {
        Self { apps: HashMap::new() }
    }

    pub fn register(&mut self, app: Arc<dyn App>) {
        let name = app.manifest().name.clone();
        self.apps.insert(name, app);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn App>> {
        self.apps.get(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.apps.keys().map(|s| s.as_str()).collect()
    }

    /// Build a registry with all discovered plugins, filtered by what's enabled in config.
    ///
    /// `global_log_level` is the fallback log level from `[general] log_level` —
    /// each app may override it via `AppConfig::log_level`.
    pub fn from_config(configs: &[AppConfig], plugins: &[DiscoveredPlugin], data_dir: &Path, global_log_level: &str) -> Self {
        let mut registry = Self::new();

        // Index plugins by name for quick lookup
        let plugin_map: HashMap<&str, &DiscoveredPlugin> = plugins.iter().map(|p| (p.manifest.plugin.name.as_str(), p)).collect();

        for config in configs.iter().filter(|c| c.enabled) {
            let resolved_log_level = config.log_level.clone().unwrap_or_else(|| global_log_level.to_owned());

            // Check if this app is backed by a plugin
            if config.plugin.is_some() || plugin_map.contains_key(config.name.as_str()) {
                if let Some(plugin) = plugin_map.get(config.name.as_str()) {
                    let manifest = corre_core::app::AppManifest::from(config);
                    let app = crate::subprocess::SubprocessApp::new(manifest, plugin.binary.clone(), plugin.dir.clone())
                        .with_outputs(plugin.manifest.plugin.permissions.outputs.clone())
                        .with_sandbox(plugin.manifest.plugin.permissions.sandbox.clone())
                        .with_data_dir(data_dir.to_path_buf())
                        .with_log_level(resolved_log_level);
                    registry.register(Arc::new(app));
                } else {
                    tracing::warn!("App `{}` has plugin set but no plugin directory found, skipping", config.name);
                }
                continue;
            }

            tracing::warn!("Unknown app `{}` and no plugin found, skipping", config.name);
        }

        registry
    }
}

impl Default for AppRegistry {
    fn default() -> Self {
        Self::new()
    }
}
