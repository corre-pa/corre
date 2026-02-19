use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// A single news article produced by a capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub title: String,
    pub summary: String,
    pub body: String,
    #[serde(default)]
    pub sources: Vec<Source>,
    /// Newsworthiness score from 0.0 to 1.0.
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub title: String,
    pub url: String,
}

/// A section groups related articles under a heading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub articles: Vec<Article>,
}

/// A full newspaper edition for a given date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edition {
    pub date: NaiveDate,
    pub headline: String,
    pub sections: Vec<Section>,
    pub produced_at: DateTime<Utc>,
}

impl Edition {
    pub fn new(date: NaiveDate, sections: Vec<Section>) -> Self {
        let headline = sections
            .iter()
            .flat_map(|s| s.articles.iter())
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
            .map(|a| a.title.clone())
            .unwrap_or_else(|| "No news today".into());

        Self { date, headline, sections, produced_at: Utc::now() }
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
