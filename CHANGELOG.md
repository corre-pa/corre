# Changelog

All notable changes to Corre are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/).

## [0.20.0] - 2026-03-10

### Added
- About modal to the operator dashboard.

### Changed
- Updated README.
- Changed project license.

## [0.19.0] - 2026-03-08

### Fixed
- Populate and serve the Tantivy search index end-to-end.

### Changed
- Added container publish script, removed old deploy/install scripts.
- Updated README.

## [0.18.0] - 2026-03-06

### Changed
- Renamed "Capabilities" to "Apps" across the entire project (types, config,
  documentation, CLI output, and dashboard UI).
- Updated README and APP_GUIDE to reflect the new terminology.

## [0.17.0] - 2026-03-03

### Added
- **extra_body pass-through** for provider-specific LLM parameters, enabling
  direct control over vendor extensions (e.g. Venice.ai model parameters).
- **Plugin-owned log files** with daily rotation, giving each plugin its own
  dedicated log output.
- **corre-host crate** extracted from subprocess hosting logic, with debug
  logging and fatal error handling.
- `update_models` maintenance script.
- 35x speedup in daily-brief run times.

### Changed
- `SubprocessCapability::execute()` restructured for clarity and safety.

## [0.16.0] - 2026-03-02

### Fixed
- Dispatch plugin requests concurrently to prevent LLM scoring hangs.
- Stop safety layer from breaking MCP search results.
- Upgrade plugin link URLs to HTTPS when dashboard is served over TLS.
- Use protocol-relative URLs for links.
- Replace hand-built TOML generation with `toml::to_string_pretty`.
- Harden panic and error-handling paths across the codebase.
- Deduplicate plugin stderr logs and strip ANSI escape sequences.
- Wire boundary wrapping and harden role marker sanitization.
- Demultiplex SDK codec to enable concurrent RPC calls.
- Resolve three concurrency bugs (deadlock, TOCTOU race, blocking I/O).
- Use modern Tailscale serve syntax and document remote access.

### Changed
- Apply idiomatic Rust patterns across 14 files.
- Consolidate DRY violations across 10 crates.
- Remove dead code across 8 crates.
- Remove 12 unused dependency declarations across 6 crates.
- Upgrade base image to Trixie.

## [0.15.0] - 2026-03-01

### Added
- **Capability plugin system** (CCPP v1) for loading capabilities as
  external plugins.
- **MCP registry** with per-MCP config files and unified MCP Store,
  generated from individual JSON definitions.
- **Rolodex capability** for contact management.
- **Docker image build** triggered after registry generation.

### Changed
- Focused or restored dashboard windows are brought to front via
  z-index management.
- MCP tool-level errors are now surfaced into a dedicated
  `McpCallError` enum.
- Embedded assets removed from bundle in favour of external serving.
- Reverted to beta versioning scheme.

### Fixed
- More detailed logs on search parse errors.

## [0.4.0] - 2026-02-24

### Fixed
- Removed irrelevant articles from daily brief output.
- Fixed `build-all` script.

### Changed
- Environment variables are now loaded from `.env` file automatically.

## [0.3.0] - 2026-02-23

### Added
- **Dashboard for capability management** with a themed UI for monitoring
  and controlling capabilities.
- **System monitor** displaying host resource usage in the dashboard.
- **Historical log viewer** with date picker for browsing past capability
  run logs from the dashboard.
- **Per-capability LLM configuration overrides** allowing each capability
  to specify its own model, temperature, and token limits.
- **In-progress polling** for capability timeouts so the scheduler can
  detect and report stuck runs.

### Changed
- Scoring and summarization merged into a single LLM call per source,
  reducing API usage and latency.
- Capability timeout increased from 5 to 10 minutes.
- Updated default daily-brief model.

### Fixed
- Reduced false positives in safety layer base64 detection.

## [0.2.1] - 2026-02-22

### Fixed
- **Brave web search was returning zero results.** The topics config used
  freshness values like `1d`/`1w` but Brave's `brave_web_search` MCP tool
  expects `pd`/`pw`/`pm`/`py`. A mapping layer now normalises these before
  calling the API.
- **MCP pool dropped valid JSON when non-JSON text blocks were present.**
  `brave_web_search` can return a mix of JSON result objects and plain-text
  metadata (e.g. `"Summarizer key: ..."`). The pool now keeps whichever
  blocks parse as JSON instead of discarding everything.
- **LLM scoring failures were silent.** `unwrap_or_default()` swallowed
  JSON parse errors, producing zero results with no log output. Scoring
  and summary calls now retry up to 3 times with exponential backoff.
- **Rate limits and transient errors are handled properly.** The LLM
  provider now surfaces 429 (rate limited) and 503 (model overloaded)
  errors with response bodies. Retries use a longer backoff schedule
  (10s, 20s, 30s) for these cases.
- **Truncated LLM responses are detected.** The provider now reads
  `finish_reason` from the API response and returns an error when the
  completion was cut short (`finish_reason=length`).
- **Empty and null LLM responses are treated as errors** instead of
  being silently returned as empty strings.

### Changed
- Scoring prompt no longer pre-filters results below a threshold. All
  results are scored and the top 10 per source are kept, which produces
  significantly more articles per edition.
- Scoring `max_tokens` increased from 2048 to 16384 to accommodate
  scoring all results in a single response.

## [0.2.0] - 2026-02-22

### Fixed
- Static files (CSS, templates) are now embedded in the binary instead
  of loaded from the filesystem at runtime.
- Topics config loading fixed after the YAML migration.
- Removed post-search exclude filtering that was redundant with query-level
  exclusion operators, and increased the per-source result cap.

## [0.1.0] - 2026-02-22

### Added
- `corre install-deps` CLI command to install runtime dependencies
  (Node.js, npm, MCP servers).
- `bundle.sh` script to package platform-specific distribution archives.
- Structured HTML form for editing topics in the web UI, replacing the
  raw YAML editor.

### Changed
- Daily brief config switched from `topics.md` (Markdown) to
  `topics.yml` (YAML) for structured per-source configuration including
  freshness, include/exclude terms, and editorial guidance.
- All runtime configuration now uses the platform data directory
  (`~/.local/share/corre/` on Linux, `~/Library/Application Support/corre/`
  on macOS).

## [0.0.2] - 2026-02-22

### Added
- Binary cross-compilation support for Linux and macOS (aarch64/x86_64).
- `corre setup` interactive installation wizard that creates the data
  directory, writes default config, and validates API keys.
- OS-specific default configurations with platform-aware setup.

## [0.0.1] - 2026-02-22

Initial release.

### Added
- **Core framework**: cron-scheduled capability execution with isolated
  tokio tasks, 5-minute timeout per capability, and MCP server pool with
  lazy stdio process management.
- **Daily Research Brief capability**: multi-step pipeline that parses
  topics, searches via Brave Web + News Search MCP, deduplicates by URL,
  scores for newsworthiness via LLM, summarises top results, and emits
  structured editions.
- **CorreNews web server**: Axum-based newspaper UI with askama templates,
  edition archive with date navigation, section tabs, and calendar picker.
- **Full-text search**: Tantivy index over all archived articles, exposed
  at `GET /search?q=...`.
- **Safety layer** (`corre-safety`): prompt injection detection via
  Aho-Corasick pattern matching, credential leak scanning, policy
  enforcement, and LLM response scanning. Enabled by default, wraps
  MCP and LLM providers transparently.
- **Web UI features**: settings page, topics editor, archive search
  toolbar, XSS sanitisation, justified text columns, and the
  Manufacturing Consent masthead font.
- Cross-edition URL deduplication via `EditionCache`.
- Contextual query building with include phrases and exclusion operators.
- Docker Compose deployment configuration.
- Debian/Ubuntu deployment script.
- Rolling capability log files for external ingestion.
- LLM-powered dad joke taglines for each edition.
