//! Core output types shared between the host and capability plugins.
//!
//! Defines [`CapabilityOutput`], [`Section`], [`Article`], [`Source`], and [`ContentType`] —
//! the data structures that a plugin serialises into a `capability/result` notification and
//! that the host stores as an edition in CorreNews.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Metadata describing a capability's identity, schedule, and dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub name: String,
    pub description: String,
    /// Cron expression with seconds field (e.g. "0 0 5 * * *" for 05:00 daily).
    pub schedule: String,
    /// Names of MCP servers this capability requires (references `[mcp.servers.*]` in config).
    pub mcp_servers: Vec<String>,
    /// Optional path to a user-editable config file (relative to project root).
    pub config_path: Option<String>,
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

/// The output produced by a capability after execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityOutput {
    pub capability_name: String,
    pub produced_at: DateTime<Utc>,
    pub sections: Vec<Section>,
    #[serde(default)]
    pub content_type: ContentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_content: Option<CustomContent>,
}

/// The type of content a capability produces.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    /// Standard newspaper layout rendered by the host.
    #[default]
    Newspaper,
    /// Plugin-provided HTML/CSS/JS rendered in a host wrapper.
    Custom,
}

/// Custom content provided by a plugin with `content_type = "custom"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomContent {
    /// Pre-rendered HTML (sanitized by host before serving).
    pub html: String,
    /// Optional scoped CSS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css: Option<String>,
    /// Optional client-side JS (sandboxed iframe if enabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub js: Option<String>,
    /// Plain text for Tantivy full-text indexing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub searchable_text: Option<String>,
}
