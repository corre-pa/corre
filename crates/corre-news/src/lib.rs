//! CorreNews — the newspaper-style web server and edition storage layer.
//!
//! Sits between the capability pipeline and the end user. After a capability produces
//! output, the CLI calls into this crate to persist the resulting edition to disk, index
//! its articles for search, and make it available on the web interface. The crate has no
//! knowledge of how editions are produced.
//!
//! Runs as a standalone Axum HTTP server (default port 5510) or can be merged into the
//! main `corre run` process alongside the dashboard.
//!
//! # Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`archive`] | Filesystem persistence under `{data_dir}/editions/YYYY-MM-DD/edition.json`. Merges editions from multiple capabilities on the same date and maintains a `_default` cache |
//! | [`cache`] | In-memory `BTreeMap<NaiveDate, Edition>` loaded at startup, also tracks seen URLs for cross-edition deduplication |
//! | [`config`] | [`NewsConfig`] — bind address, title, editor token, and data directory settings |
//! | [`edition`] | Re-exports the [`Edition`] type (date, headline, sections, tagline, content type) |
//! | [`render`] | Askama HTML templates: newspaper layout, edition pages, topics editor, and search results |
//! | [`search`] | Tantivy full-text search index over all archived articles (title, summary, body) |
//! | [`server`] | Axum route definitions and `serve()` / `serve_with_extra_routes()` entry points |
//!
//! # HTTP routes
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/` | Latest edition rendered as HTML |
//! | `GET` | `/edition/:date` | Specific edition with archive navigation |
//! | `GET` | `/api/dates` | JSON array of available edition dates |
//! | `GET` | `/search?q=...&limit=N` | Full-text article search (JSON) |
//! | `GET` | `/settings/topics` | Token-gated topics editor page |
//! | `GET/PUT` | `/api/topics` | Topics config API (requires editor token) |
//! | `GET` | `/plugin/:name/static/*path` | Plugin static assets |
//! | `GET` | `/static/*path` | Embedded CSS and assets |

pub mod archive;
pub mod cache;
pub mod config;
pub mod edition;
pub mod render;
pub mod search;
pub mod server;

pub use config::NewsConfig;
pub use edition::Edition;
