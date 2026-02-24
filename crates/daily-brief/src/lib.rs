//! Shared types for the daily-brief capability.
//!
//! The `Edition` struct lives here so that both the `daily-brief` binary
//! (which writes editions) and `corre-news` (which reads and serves them)
//! can share the same serialisation format without either depending on the other.

pub use corre_sdk::types::{Article, ContentType, CustomContent, Section, Source};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// A full newspaper edition for a given date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edition {
    pub date: NaiveDate,
    pub headline: String,
    pub sections: Vec<Section>,
    pub produced_at: DateTime<Utc>,
    #[serde(default = "default_tagline")]
    pub tagline: String,
    #[serde(default)]
    pub content_type: ContentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_content: Option<CustomContent>,
}

fn default_tagline() -> String {
    "All the news that's fit to pun".to_string()
}

impl Edition {
    pub fn new(date: NaiveDate, sections: Vec<Section>) -> Self {
        let headline = sections
            .iter()
            .flat_map(|s| s.articles.iter())
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
            .map(|a| a.title.clone())
            .unwrap_or_else(|| "No news today".into());

        Self {
            date,
            headline,
            sections,
            produced_at: Utc::now(),
            tagline: default_tagline(),
            content_type: ContentType::default(),
            custom_content: None,
        }
    }

    pub fn article_count(&self) -> usize {
        self.sections.iter().map(|s| s.articles.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn edition_picks_top_headline() {
        let sections = vec![Section {
            title: "Tech".into(),
            articles: vec![
                Article { title: "Low score".into(), summary: String::new(), body: String::new(), sources: vec![], score: 0.3 },
                Article { title: "High score".into(), summary: String::new(), body: String::new(), sources: vec![], score: 0.9 },
            ],
        }];
        let edition = Edition::new(NaiveDate::from_ymd_opt(2026, 2, 19).unwrap(), sections);
        assert_eq!(edition.headline, "High score");
        assert_eq!(edition.article_count(), 2);
    }
}
