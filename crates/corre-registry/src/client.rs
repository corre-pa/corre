//! HTTP client for fetching and caching the remote registry manifest.
//!
//! [`RegistryClient`] fetches `{url}/mcp/registry.json`, caches the result in memory
//! for a configurable TTL, and exposes search and single-entry lookup helpers.

use crate::manifest::{CapabilityEntry, McpRegistryEntry, RegistryManifest};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Fetches and caches the registry manifest from a remote URL.
pub struct RegistryClient {
    url: String,
    cache_ttl: Duration,
    http: reqwest::Client,
    cache: Arc<RwLock<Option<CacheEntry>>>,
}

struct CacheEntry {
    manifest: RegistryManifest,
    fetched_at: Instant,
}

impl RegistryClient {
    pub fn new(url: String, cache_ttl_secs: u64) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { url, cache_ttl: Duration::from_secs(cache_ttl_secs), http, cache: Arc::new(RwLock::new(None)) }
    }

    /// Fetch the manifest, using the in-memory cache if still valid.
    pub async fn get_manifest(&self) -> Result<RegistryManifest, RegistryError> {
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.as_ref() {
                if entry.fetched_at.elapsed() < self.cache_ttl {
                    return Ok(entry.manifest.clone());
                }
            }
        }
        self.refresh().await
    }

    /// Force-refresh the cached manifest from the remote URL.
    pub async fn refresh(&self) -> Result<RegistryManifest, RegistryError> {
        if self.url.is_empty() {
            return Err(RegistryError::NotConfigured);
        }

        let url = format!("{}/mcp/registry.json", self.url.trim_end_matches('/'));
        tracing::info!("Fetching registry manifest from {url}");

        let resp = self.http.get(&url).send().await.map_err(|e| RegistryError::Fetch(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(RegistryError::Fetch(format!("HTTP {}", resp.status())));
        }

        let manifest: RegistryManifest = resp.json().await.map_err(|e| RegistryError::Parse(e.to_string()))?;

        let mut cache = self.cache.write().await;
        *cache = Some(CacheEntry { manifest: manifest.clone(), fetched_at: Instant::now() });

        Ok(manifest)
    }

    /// Search entries by matching query against name, description, and tags (case-insensitive).
    pub async fn search(&self, query: &str) -> Result<Vec<McpRegistryEntry>, RegistryError> {
        let manifest = self.get_manifest().await?;
        let q = query.to_lowercase();
        let results = manifest
            .servers
            .into_iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&q)
                    || entry.description.to_lowercase().contains(&q)
                    || entry.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect();
        Ok(results)
    }

    /// Look up a single MCP server entry by ID.
    pub async fn get_entry(&self, id: &str) -> Result<Option<McpRegistryEntry>, RegistryError> {
        let manifest = self.get_manifest().await?;
        Ok(manifest.servers.into_iter().find(|e| e.id == id))
    }

    // ── Capability methods ──────────────────────────────────────────────

    /// Fetch all capability entries from the registry.
    pub async fn fetch_capabilities(&self) -> Result<Vec<CapabilityEntry>, RegistryError> {
        let manifest = self.get_manifest().await?;
        Ok(manifest.capabilities)
    }

    /// Look up a single capability entry by ID.
    pub async fn lookup_capability(&self, id: &str) -> Result<Option<CapabilityEntry>, RegistryError> {
        let manifest = self.get_manifest().await?;
        Ok(manifest.capabilities.into_iter().find(|e| e.id == id))
    }

    /// Search capability entries by matching query against name, description, and tags.
    pub async fn search_capabilities(&self, query: &str) -> Result<Vec<CapabilityEntry>, RegistryError> {
        let manifest = self.get_manifest().await?;
        let q = query.to_lowercase();
        let results = manifest
            .capabilities
            .into_iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&q)
                    || entry.description.to_lowercase().contains(&q)
                    || entry.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect();
        Ok(results)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("registry URL not configured")]
    NotConfigured,
    #[error("failed to fetch registry: {0}")]
    Fetch(String),
    #[error("failed to parse registry manifest: {0}")]
    Parse(String),
}

impl RegistryError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::NotConfigured => 503,
            Self::Fetch(_) | Self::Parse(_) => 502,
        }
    }
}
