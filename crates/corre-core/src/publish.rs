use ammonia::Builder;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Sanitize HTML content, allowing only safe formatting tags.
/// Strips scripts, event handlers, iframes, and other dangerous elements.
pub fn sanitize_html(input: &str) -> String {
    Builder::new()
        .tags(HashSet::from(["p", "br", "b", "strong", "i", "em", "a", "ul", "ol", "li", "blockquote"]))
        .link_rel(Some("noopener"))
        .url_schemes(HashSet::from(["http", "https"]))
        .clean(input)
        .to_string()
}

/// Validate a URL, returning `"#"` for anything that isn't http or https.
pub fn sanitize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") { trimmed.to_string() } else { "#".to_string() }
}

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
    fn sanitize_html_strips_scripts_preserves_formatting() {
        let dangerous = r#"<p>Hello</p><script>alert('xss')</script><b>bold</b>"#;
        let result = sanitize_html(dangerous);
        assert!(result.contains("<p>Hello</p>"));
        assert!(result.contains("<b>bold</b>"));
        assert!(!result.contains("<script>"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn sanitize_html_strips_event_handlers() {
        let dangerous = r#"<img onerror="alert(1)" src="x"><p>safe</p>"#;
        let result = sanitize_html(dangerous);
        assert!(!result.contains("onerror"));
        assert!(result.contains("<p>safe</p>"));
    }

    #[test]
    fn sanitize_html_preserves_safe_links() {
        let input = r#"<a href="https://example.com">link</a>"#;
        let result = sanitize_html(input);
        assert!(result.contains("https://example.com"));
        assert!(result.contains("<a"));
    }

    #[test]
    fn sanitize_html_strips_javascript_links() {
        let input = r#"<a href="javascript:alert(1)">click</a>"#;
        let result = sanitize_html(input);
        assert!(!result.contains("javascript:"));
    }

    #[test]
    fn sanitize_url_allows_http_and_https() {
        assert_eq!(sanitize_url("https://example.com"), "https://example.com");
        assert_eq!(sanitize_url("http://example.com"), "http://example.com");
    }

    #[test]
    fn sanitize_url_rejects_javascript_protocol() {
        assert_eq!(sanitize_url("javascript:alert(1)"), "#");
        assert_eq!(sanitize_url("data:text/html,<script>"), "#");
        assert_eq!(sanitize_url(""), "#");
    }

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
