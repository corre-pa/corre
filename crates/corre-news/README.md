# corre-news

The web server and edition storage layer for Corre. Exposes archived capability output as a
newspaper-style HTML interface (CorreNews), provides Tantivy full-text search over all indexed
articles, and serves a token-protected settings page for editing capability topics.

## Role in the Corre project

`corre-news` sits between the capability pipeline and the end user. After a capability produces
output, the CLI calls `corre-news` to persist the resulting `Edition` to disk and make it
visible on the web interface. The crate has no knowledge of how editions are produced.

## Key types

| Type | Purpose |
|------|---------|
| `Archive` | Filesystem persistence under `{data_dir}/editions/YYYY-MM-DD/edition.json` |
| `EditionCache` | In-memory `BTreeMap<NaiveDate, Edition>` loaded at startup; also tracks seen URLs |
| `SearchIndex` | Tantivy full-text index over all archived articles |
| `AppState` | Shared Axum state: cache, search index, config, data directory |
| `NewspaperTemplate` | Askama template for the standard newspaper layout |

## HTTP routes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Latest edition as HTML |
| `GET` | `/edition/:date` | Specific edition |
| `GET` | `/api/dates` | JSON array of available edition dates |
| `GET` | `/search?q=...&limit=N` | Full-text search (JSON) |
| `GET` | `/settings/topics` | Token-gated topics editor |
| `GET/PUT` | `/api/topics` | Topics file API (requires token) |
| `GET` | `/plugin/:name/static/*path` | Plugin static assets |
| `GET` | `/static/*path` | Embedded CSS and assets |

## Entry points

```rust
pub async fn serve(state: Arc<AppState>, addr: SocketAddr) -> anyhow::Result<()>
pub async fn serve_with_extra_routes(state: Arc<AppState>, extra: Router, addr: SocketAddr) -> anyhow::Result<()>
```
