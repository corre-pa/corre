//! Publishing types for CorreNews editions.
//!
//! Re-exports `Edition` from the `daily-brief` crate so that downstream
//! consumers (archive, cache, search, server) keep their imports unchanged.

pub use corre_sdk::html::{sanitize_custom_html, sanitize_html, sanitize_url};
pub use corre_sdk::types::{Article, ContentType, CustomContent, Section, Source};
pub use daily_brief::Edition;
