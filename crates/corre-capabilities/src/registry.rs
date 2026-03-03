//! `CapabilityRegistry`: maps capability names to boxed `Capability` trait objects.
//!
//! Instantiates subprocess-backed plugin capabilities from `DiscoveredPlugin` entries
//! discovered at startup.

use corre_core::capability::Capability;
use corre_core::config::CapabilityConfig;
use corre_core::plugin::DiscoveredPlugin;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Registry mapping capability names to their implementations.
pub struct CapabilityRegistry {
    capabilities: HashMap<String, Arc<dyn Capability>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self { capabilities: HashMap::new() }
    }

    pub fn register(&mut self, capability: Arc<dyn Capability>) {
        let name = capability.manifest().name.clone();
        self.capabilities.insert(name, capability);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Capability>> {
        self.capabilities.get(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.capabilities.keys().map(|s| s.as_str()).collect()
    }

    /// Build a registry with all built-in capabilities and discovered plugins,
    /// filtered by what's enabled in config.
    pub fn from_config(configs: &[CapabilityConfig], plugins: &[DiscoveredPlugin], data_dir: &Path, global_log_level: &str) -> Self {
        let mut registry = Self::new();

        // Index plugins by name for quick lookup
        let plugin_map: HashMap<&str, &DiscoveredPlugin> = plugins.iter().map(|p| (p.manifest.plugin.name.as_str(), p)).collect();

        for config in configs.iter().filter(|c| c.enabled) {
            let resolved_log_level = config.log_level.clone().unwrap_or_else(|| global_log_level.to_owned());

            // Check if this capability is backed by a plugin
            if config.plugin.is_some() || plugin_map.contains_key(config.name.as_str()) {
                if let Some(plugin) = plugin_map.get(config.name.as_str()) {
                    let manifest = corre_core::capability::CapabilityManifest::from(config);
                    let cap = crate::subprocess::SubprocessCapability::new(manifest, plugin.binary.clone(), plugin.dir.clone())
                        .with_outputs(plugin.manifest.plugin.permissions.outputs.clone())
                        .with_sandbox(plugin.manifest.plugin.permissions.sandbox.clone())
                        .with_data_dir(data_dir.to_path_buf())
                        .with_log_level(resolved_log_level);
                    registry.register(Arc::new(cap));
                } else {
                    tracing::warn!("Capability `{}` has plugin set but no plugin directory found, skipping", config.name);
                }
                continue;
            }

            tracing::warn!("Unknown capability `{}` and no plugin found, skipping", config.name);
        }

        registry
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}
