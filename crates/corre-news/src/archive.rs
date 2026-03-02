//! Filesystem-backed archive for `Edition` values.
//!
//! Editions are stored per-capability under
//! `{data_dir}/{capability}/editions/YYYY-MM-DD/edition.json`.
//! The archive scans all `{data_dir}/*/editions/` directories so that
//! editions from any capability are discovered automatically.

use crate::edition::Edition;
use anyhow::Context;
use chrono::NaiveDate;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Filesystem-based archive for editions.
///
/// Scans `{data_dir}/*/editions/` for date-named subdirectories containing
/// `edition.json` files. Multiple capabilities may produce editions for the
/// same date; in that case the sections are merged into a single edition.
pub struct Archive {
    data_dir: PathBuf,
}

impl Archive {
    pub fn new(data_dir: &Path) -> Self {
        Self { data_dir: data_dir.to_path_buf() }
    }

    /// Collect all `*/editions/` directories under the data root.
    fn edition_roots(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.data_dir) else {
            return vec![];
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let editions = e.path().join("editions");
                editions.is_dir().then_some(editions)
            })
            .collect()
    }

    /// Load an edition by date, merging across capability directories.
    pub fn load(&self, date: NaiveDate) -> anyhow::Result<Option<Edition>> {
        let date_str = date.format("%Y-%m-%d").to_string();
        let mut merged: Option<Edition> = None;

        for root in self.edition_roots() {
            let path = root.join(&date_str).join("edition.json");
            if !path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let edition: Edition = serde_json::from_str(&content)?;
            merged = Some(match merged {
                None => edition,
                Some(mut base) => {
                    base.sections.extend(edition.sections);
                    base
                }
            });
        }

        Ok(merged)
    }

    /// List all available edition dates, most recent first.
    pub fn list_dates(&self) -> anyhow::Result<Vec<NaiveDate>> {
        let mut dates = BTreeSet::new();

        for root in self.edition_roots() {
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if let Ok(d) = NaiveDate::parse_from_str(&name, "%Y-%m-%d") {
                        dates.insert(d);
                    }
                }
            }
        }

        Ok(dates.into_iter().rev().collect())
    }

    /// Store an edition as JSON under `{data_dir}/_default/editions/`.
    ///
    /// Primarily used by the edition cache when merging incoming editions.
    pub fn store(&self, edition: &Edition) -> anyhow::Result<PathBuf> {
        let dir = self.data_dir.join("_default").join("editions").join(edition.date.format("%Y-%m-%d").to_string());
        std::fs::create_dir_all(&dir).with_context(|| format!("failed to create edition directory: {}", dir.display()))?;

        let path = dir.join("edition.json");
        let json = serde_json::to_string_pretty(edition)?;
        std::fs::write(&path, json)?;

        tracing::info!("Stored edition for {} at {}", edition.date, path.display());
        Ok(path)
    }

    /// Load the most recent edition.
    pub fn latest(&self) -> anyhow::Result<Option<Edition>> {
        let dates = self.list_dates()?;
        match dates.first() {
            Some(date) => self.load(*date),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edition::{Article, Section};
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

    /// Write an edition into the per-capability layout expected by Archive.
    fn write_edition(data_dir: &Path, capability: &str, edition: &Edition) {
        let dir = data_dir.join(capability).join("editions").join(edition.date.format("%Y-%m-%d").to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let json = serde_json::to_string_pretty(edition).unwrap();
        std::fs::write(dir.join("edition.json"), json).unwrap();
    }

    #[test]
    fn load_edition_from_capability_dir() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());
        let edition = sample_edition();

        write_edition(dir.path(), "daily-brief", &edition);

        let loaded = archive.load(edition.date).unwrap().unwrap();
        assert_eq!(loaded.headline, "Test article");
        assert_eq!(loaded.article_count(), 1);
    }

    #[test]
    fn list_dates_across_capabilities() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());

        let mut e1 = sample_edition();
        e1.date = NaiveDate::from_ymd_opt(2026, 2, 17).unwrap();
        write_edition(dir.path(), "daily-brief", &e1);

        let e2 = sample_edition();
        write_edition(dir.path(), "daily-brief", &e2);

        // Same date from a different capability
        write_edition(dir.path(), "other-cap", &e2);

        let dates = archive.list_dates().unwrap();
        // Dates are deduplicated
        assert_eq!(dates.len(), 2);
        assert_eq!(dates[0], NaiveDate::from_ymd_opt(2026, 2, 19).unwrap());
    }

    #[test]
    fn merge_editions_from_multiple_capabilities() {
        let dir = tempdir().unwrap();
        let archive = Archive::new(dir.path());

        let e1 = sample_edition();
        write_edition(dir.path(), "daily-brief", &e1);

        let e2 = Edition::new(
            e1.date,
            vec![Section {
                title: "Science".into(),
                articles: vec![Article {
                    title: "Science article".into(),
                    summary: "Discovery".into(),
                    body: "Details".into(),
                    sources: vec![],
                    score: 0.9,
                }],
            }],
        );
        write_edition(dir.path(), "science-brief", &e2);

        let loaded = archive.load(e1.date).unwrap().unwrap();
        assert_eq!(loaded.sections.len(), 2);
    }
}
