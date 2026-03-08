//! Full-text search index over archived editions, backed by Tantivy.
//!
//! Articles are indexed with `title`, `summary`, and `body` as text fields, plus `date`
//! and `section` as stored string fields. The index persists to
//! `{data_dir}/daily-brief/search_index/`.

use anyhow::Context;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Value};
use tantivy::{Index, IndexReader, ReloadPolicy};

/// Full-text search index over archived editions using tantivy.
pub struct SearchIndex {
    reader: IndexReader,
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
    /// Open an existing search index in read-only mode. Returns `None` if the
    /// index directory does not exist yet (no editions have been indexed).
    pub fn open_readonly(data_dir: &Path) -> anyhow::Result<Option<Self>> {
        let index_dir = data_dir.join("daily-brief").join("search_index");
        if !index_dir.is_dir() {
            return Ok(None);
        }

        let dir = tantivy::directory::MmapDirectory::open(&index_dir)?;
        let index = Index::open(dir).context("Failed to open search index")?;

        let schema = index.schema();
        let title_field = schema.get_field("title").context("missing title field")?;
        let summary_field = schema.get_field("summary").context("missing summary field")?;
        let body_field = schema.get_field("body").context("missing body field")?;
        let date_field = schema.get_field("date").context("missing date field")?;
        let section_field = schema.get_field("section").context("missing section field")?;

        let reader =
            index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into().context("Failed to create index reader")?;

        Ok(Some(Self { reader, index, title_field, summary_field, body_field, date_field, section_field }))
    }

    /// Search articles by query string. Returns top N results.
    pub fn search(&self, query_str: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();

        let query_parser = QueryParser::for_index(&self.index, vec![self.title_field, self.summary_field, self.body_field]);
        let query = query_parser.parse_query(query_str)?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        top_docs
            .into_iter()
            .map(|(_score, doc_address)| {
                let doc: tantivy::TantivyDocument = searcher.doc(doc_address)?;
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
