use anyhow::Context;
use chrono::NaiveDate;
use corre_core::publish::Edition;
use std::path::{Path, PathBuf};

/// Filesystem-based archive for editions.
/// Layout: `{data_dir}/editions/{YYYY-MM-DD}/edition.json`
pub struct Archive {
    base_dir: PathBuf,
}

impl Archive {
    pub fn new(data_dir: &Path) -> Self {
        Self { base_dir: data_dir.join("editions") }
    }

    /// Store an edition as JSON.
    pub fn store(&self, edition: &Edition) -> anyhow::Result<PathBuf> {
        let dir = self.edition_dir(edition.date);
        std::fs::create_dir_all(&dir).with_context(|| format!("Failed to create edition directory: {}", dir.display()))?;

        let path = dir.join("edition.json");
        let json = serde_json::to_string_pretty(edition)?;
        std::fs::write(&path, json)?;

        tracing::info!("Stored edition for {} at {}", edition.date, path.display());
        Ok(path)
    }

    /// Load an edition by date.
    pub fn load(&self, date: NaiveDate) -> anyhow::Result<Option<Edition>> {
        let path = self.edition_dir(date).join("edition.json");
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let edition: Edition = serde_json::from_str(&content)?;
        Ok(Some(edition))
    }

    /// List all available edition dates, most recent first.
    pub fn list_dates(&self) -> anyhow::Result<Vec<NaiveDate>> {
        if !self.base_dir.exists() {
            return Ok(vec![]);
        }

        let mut dates: Vec<NaiveDate> = std::fs::read_dir(&self.base_dir)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                NaiveDate::parse_from_str(&name, "%Y-%m-%d").ok()
            })
            .collect();

        dates.sort_unstable_by(|a, b| b.cmp(a));
        Ok(dates)
    }

    /// Load the most recent edition.
    pub fn latest(&self) -> anyhow::Result<Option<Edition>> {
        let dates = self.list_dates()?;
        match dates.first() {
            Some(date) => self.load(*date),
            None => Ok(None),
        }
    }

    fn edition_dir(&self, date: NaiveDate) -> PathBuf {
        self.base_dir.join(date.format("%Y-%m-%d").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corre_core::publish::{Article, Section};
    use tempfile::tempdir;

    fn sample_edition() -> Edition {
        Edition::new(
            NaiveDate::from_ymd_opt(2026, 2, 19).unwrap(),
            vec![Section {
                title: "Tech".into(),
                articles: vec![Article {
                    title: "Test article".into(),
                    summary: "A test".into(),
                    body: "Body text".into(),
                    sources: vec![],
                    score: 0.8,
                }],
            }],
        )
    }

    #[test]
    fn store_and_load_edition() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());
        let edition = sample_edition();

        archive.store(&edition).unwrap();

        let loaded = archive.load(edition.date).unwrap().unwrap();
        assert_eq!(loaded.headline, "Test article");
        assert_eq!(loaded.article_count(), 1);
    }

    #[test]
    fn list_dates_sorted() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());

        let mut e1 = sample_edition();
        e1.date = NaiveDate::from_ymd_opt(2026, 2, 17).unwrap();
        archive.store(&e1).unwrap();

        let e2 = sample_edition();
        archive.store(&e2).unwrap();

        let dates = archive.list_dates().unwrap();
        assert_eq!(dates.len(), 2);
        assert_eq!(dates[0], NaiveDate::from_ymd_opt(2026, 2, 19).unwrap());
    }
}
