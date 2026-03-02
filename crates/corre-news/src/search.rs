//! Full-text search index over archived editions, backed by Tantivy.
//!
//! Articles are indexed with `title`, `summary`, and `body` as text fields, plus `date`
//! and `section` as stored string fields. The index persists to `{data_dir}/search_index/`.

use anyhow::Context;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{Index, ReloadPolicy};

/// Full-text search index over archived editions using tantivy.
pub struct SearchIndex {
    index: Index,
    title_field: Field,
    summary_field: Field,
    body_field: Field,
    date_field: Field,
    section_field: Field,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub title: String,
    pub summary: String,
    pub date: String,
    pub section: String,
}

impl SearchIndex {
    fn build_schema() -> (Schema, Field, Field, Field, Field, Field) {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let summary_field = schema_builder.add_text_field("summary", TEXT | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT);
        let date_field = schema_builder.add_text_field("date", STRING | STORED);
        let section_field = schema_builder.add_text_field("section", STRING | STORED);
        (schema_builder.build(), title_field, summary_field, body_field, date_field, section_field)
    }

    /// Open an existing search index in read-only mode. Returns `None` if the
    /// index directory does not exist yet (no editions have been indexed).
    pub fn open_readonly(data_dir: &Path) -> anyhow::Result<Option<Self>> {
        let index_dir = data_dir.join("search_index");
        if !index_dir.is_dir() {
            return Ok(None);
        }

        let (schema, title_field, summary_field, body_field, date_field, section_field) = Self::build_schema();

        let index = Index::open_or_create(tantivy::directory::MmapDirectory::open(&index_dir)?, schema)
            .context("Failed to open search index read-only")?;

        Ok(Some(Self { index, title_field, summary_field, body_field, date_field, section_field }))
    }

    /// Search articles by query string. Returns top N results.
    pub fn search(&self, query_str: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        let reader = self.index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into()?;
        let searcher = reader.searcher();

        let query_parser = QueryParser::for_index(&self.index, vec![self.title_field, self.summary_field, self.body_field]);
        let query = query_parser.parse_query(query_str)?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        top_docs
            .into_iter()
            .map(|(_score, doc_address)| {
                let doc: TantivyDocument = searcher.doc(doc_address)?;
                Ok(SearchResult {
                    title: doc.get_first(self.title_field).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    summary: doc.get_first(self.summary_field).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    date: doc.get_first(self.date_field).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    section: doc.get_first(self.section_field).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                })
            })
            .collect()
    }
}
