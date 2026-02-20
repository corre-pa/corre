use crate::archive::Archive;
use chrono::NaiveDate;
use corre_core::publish::Edition;
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
    pub async fn store(&self, edition: &Edition) -> anyhow::Result<PathBuf> {
        let path = self.archive.store(edition)?;
        let mut inner = self.inner.write().await;
        collect_urls(edition, &mut inner.seen_urls);
        inner.editions.insert(edition.date, edition.clone());
        Ok(path)
    }

    /// Return a clone of all seen source URLs across all editions.
    pub async fn seen_urls(&self) -> HashSet<String> {
        self.inner.read().await.seen_urls.clone()
    }

    /// Check whether a URL has appeared in any previous edition.
    #[allow(dead_code)]
    pub async fn contains_url(&self, url: &str) -> bool {
        self.inner.read().await.seen_urls.contains(url)
    }
}

/// Extract all source URLs from an edition into the given set.
fn collect_urls(edition: &Edition, seen_urls: &mut HashSet<String>) {
    for section in &edition.sections {
        for article in &section.articles {
            for source in &article.sources {
                if !source.url.is_empty() {
                    seen_urls.insert(source.url.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corre_core::publish::{Article, Section, Source};
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
