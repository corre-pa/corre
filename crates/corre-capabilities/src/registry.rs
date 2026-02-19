use corre_core::capability::Capability;
use corre_core::config::CapabilityConfig;
use std::collections::HashMap;
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

    /// Build a registry with all built-in capabilities, filtered by what's enabled in config.
    pub fn from_config(configs: &[CapabilityConfig]) -> Self {
        let mut registry = Self::new();

        for config in configs.iter().filter(|c| c.enabled) {
            match config.name.as_str() {
                "daily-brief" => {
                    let cap = crate::daily_brief::DailyBrief::from_config(config);
                    registry.register(Arc::new(cap));
                }
                name => {
                    tracing::warn!("Unknown capability `{name}`, skipping");
                }
            }
        }

        registry
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}
