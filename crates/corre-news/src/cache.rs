//! In-memory edition cache backed by the filesystem `Archive`.
//!
//! Loads all editions into a `BTreeMap` at startup so HTTP handlers never touch the filesystem
//! during a request. Also tracks all source URLs for cross-edition deduplication.

use crate::archive::Archive;
use crate::edition::Edition;
use chrono::NaiveDate;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use tokio::sync::RwLock;

/// In-memory cache of all editions, backed by the filesystem archive.
///
/// Keeps a sorted map of editions and a set of all source URLs ever published,
/// enabling cross-edition URL deduplication for capabilities.
pub struct EditionCache {
    inner: RwLock<CacheInner>,
    archive: Archive,
}

struct CacheInner {
    editions: BTreeMap<NaiveDate, Edition>,
    seen_urls: HashSet<String>,
}

impl EditionCache {
    /// Load all editions from the archive into memory.
    pub fn load(archive: Archive) -> Self {
        let mut editions = BTreeMap::new();
        let mut seen_urls = HashSet::new();

        if let Ok(dates) = archive.list_dates() {
            for date in dates {
                if let Ok(Some(edition)) = archive.load(date) {
                    collect_urls(&edition, &mut seen_urls);
                    editions.insert(date, edition);
                }
            }
        }

        tracing::info!("EditionCache loaded {} editions, {} unique URLs", editions.len(), seen_urls.len());

        Self { inner: RwLock::new(CacheInner { editions, seen_urls }), archive }
    }

    /// Return the most recent edition (no I/O).
    pub async fn latest(&self) -> Option<Edition> {
        self.inner.read().await.editions.values().next_back().cloned()
    }

    /// Return an edition by date (no I/O).
    pub async fn load_date(&self, date: NaiveDate) -> Option<Edition> {
        self.inner.read().await.editions.get(&date).cloned()
    }

    /// List all edition dates, most recent first (no I/O).
    pub async fn list_dates(&self) -> Vec<NaiveDate> {
        self.inner.read().await.editions.keys().rev().copied().collect()
    }

    /// Store an edition to disk and update the in-memory cache.
    /// If an edition already exists for the same date, merges sections:
    /// articles are appended into existing sections with matching titles,
    /// and new section titles create new sections. The headline is re-derived
    /// from the merged edition.
    pub async fn store(&self, edition: &Edition) -> anyhow::Result<PathBuf> {
        let mut inner = self.inner.write().await;
        let merged = match inner.editions.get(&edition.date) {
            Some(existing) => {
                let merged = merge_editions(existing, edition);
                tracing::debug!(
                    "Merged edition for {}: {} sections, {} articles",
                    merged.date,
                    merged.sections.len(),
                    merged.article_count()
                );
                merged
            }
            None => edition.clone(),
        };
        let path = self.archive.store(&merged)?;
        collect_urls(&merged, &mut inner.seen_urls);
        inner.editions.insert(merged.date, merged);
        Ok(path)
    }

    /// Rescan the filesystem archive and load any editions not already cached.
    pub async fn refresh(&self) {
        let Ok(dates) = self.archive.list_dates() else { return };
        let inner = self.inner.read().await;
        let new_dates: Vec<NaiveDate> = dates.into_iter().filter(|d| !inner.editions.contains_key(d)).collect();
        drop(inner);

        if new_dates.is_empty() {
            return;
        }

        let mut inner = self.inner.write().await;
        for date in new_dates {
            // Re-check under write lock to avoid TOCTOU
            if inner.editions.contains_key(&date) {
                continue;
            }
            if let Ok(Some(edition)) = self.archive.load(date) {
                tracing::info!("Discovered new edition for {date}");
                collect_urls(&edition, &mut inner.seen_urls);
                inner.editions.insert(date, edition);
            }
        }
    }

    /// Return a clone of all seen source URLs across all editions.
    pub async fn seen_urls(&self) -> HashSet<String> {
        self.inner.read().await.seen_urls.clone()
    }

    /// Check whether a URL has appeared in any previous edition.
    #[cfg(test)]
    pub async fn contains_url(&self, url: &str) -> bool {
        self.inner.read().await.seen_urls.contains(url)
    }
}

/// Merge two editions for the same date. Articles from `incoming` are appended
/// to sections in `existing` with matching titles; new section titles are added.
/// The headline is re-derived from the highest-scoring article.
fn merge_editions(existing: &Edition, incoming: &Edition) -> Edition {
    use std::collections::HashMap;
    let mut section_map: HashMap<String, Vec<crate::edition::Article>> = HashMap::new();
    let mut section_order: Vec<String> = Vec::new();

    // Collect existing sections in order
    for section in &existing.sections {
        section_order.push(section.title.clone());
        section_map.entry(section.title.clone()).or_default().extend(section.articles.clone());
    }

    // Merge incoming sections
    for section in &incoming.sections {
        if !section_order.contains(&section.title) {
            section_order.push(section.title.clone());
        }
        section_map.entry(section.title.clone()).or_default().extend(section.articles.clone());
    }

    let sections: Vec<crate::edition::Section> = section_order
        .into_iter()
        .filter_map(|title| section_map.remove(&title).map(|articles| crate::edition::Section { title, articles }))
        .collect();

    let mut edition = Edition::new(existing.date, sections);
    // Preserve the most recent tagline if the existing one was customized
    if existing.tagline != "All the news that's fit to pun" {
        edition.tagline = existing.tagline.clone();
    }
    edition
}

/// Extract all source URLs from an edition into the given set.
fn collect_urls(edition: &Edition, seen_urls: &mut HashSet<String>) {
    seen_urls.extend(
        edition.sections.iter().flat_map(|s| &s.articles).flat_map(|a| &a.sources).filter(|s| !s.url.is_empty()).map(|s| s.url.clone()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edition::{Article, Section, Source};
    use tempfile::tempdir;

    fn edition_with_urls(date: NaiveDate, urls: &[&str]) -> Edition {
        Edition::new(
            date,
            vec![Section {
                title: "Test".into(),
                articles: urls
                    .iter()
                    .map(|url| Article {
                        title: format!("Article from {url}"),
                        summary: "Summary".into(),
                        body: "Body".into(),
                        sources: vec![Source { title: "Source".into(), url: url.to_string() }],
                        score: 0.8,
                    })
                    .collect(),
            }],
        )
    }

    #[tokio::test]
    async fn seen_urls_union_across_editions() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());

        let e1 = edition_with_urls(NaiveDate::from_ymd_opt(2026, 2, 18).unwrap(), &["https://a.com", "https://b.com"]);
        let e2 = edition_with_urls(NaiveDate::from_ymd_opt(2026, 2, 19).unwrap(), &["https://b.com", "https://c.com"]);

        archive.store(&e1).unwrap();
        archive.store(&e2).unwrap();

        let cache = EditionCache::load(archive);
        let urls = cache.seen_urls().await;

        assert!(urls.contains("https://a.com"));
        assert!(urls.contains("https://b.com"));
        assert!(urls.contains("https://c.com"));
        assert_eq!(urls.len(), 3);
    }

    #[tokio::test]
    async fn store_updates_cache_and_seen_urls() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());
        let cache = EditionCache::load(archive);

        assert!(cache.latest().await.is_none());

        let edition = edition_with_urls(NaiveDate::from_ymd_opt(2026, 2, 20).unwrap(), &["https://new.com"]);
        cache.store(&edition).await.unwrap();

        assert!(cache.contains_url("https://new.com").await);
        assert!(cache.latest().await.is_some());
        assert_eq!(cache.list_dates().await.len(), 1);
    }

    #[tokio::test]
    async fn latest_returns_most_recent() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());

        let e1 = edition_with_urls(NaiveDate::from_ymd_opt(2026, 2, 17).unwrap(), &["https://old.com"]);
        let e2 = edition_with_urls(NaiveDate::from_ymd_opt(2026, 2, 19).unwrap(), &["https://new.com"]);

        archive.store(&e1).unwrap();
        archive.store(&e2).unwrap();

        let cache = EditionCache::load(archive);
        let latest = cache.latest().await.unwrap();
        assert_eq!(latest.date, NaiveDate::from_ymd_opt(2026, 2, 19).unwrap());
    }
}
