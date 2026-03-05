# daily-brief

A standalone subprocess app that researches topics from a user-configured YAML file,
searches the web via the Brave Search MCP server, scores and summarises the results with an
LLM, and produces a newspaper-style edition for CorreNews.

## Role in the Corre project

`daily-brief` is the reference implementation for Corre's subprocess app model. It
runs as its own binary, communicating with the Corre host over stdin/stdout using the CCPP
protocol. The host spawns it, provides access to MCP tools and LLM completions, and persists
the output.

The crate produces both a library (shared `Edition` type consumed by `corre-news`) and a
binary (the app itself).

## Pipeline

The daily brief runs an 8-step deterministic pipeline:

1. **Parse config** — load `topics.yml`, extract sections and search sources
2. **Search** — parallel Brave web + news searches via `call_tool("brave-search", ...)`
3. **Deduplicate** — remove duplicate URLs within each source and filter against previously
   seen URLs from earlier editions
4. **Score + summarise** — parallel LLM calls (semaphore-bounded) requesting structured JSON
   scores and 2-3 paragraph summaries for each result
5. **Filter** — drop items with score <= 0.2, keep top 10 per source
6. **Group** — collect articles into sections preserving the YAML ordering
7. **Generate tagline** — LLM call at high temperature for a witty headline-inspired tagline
8. **Persist** — write `edition.json` via `output/write`, send `AppOutput` to host

## Configuration

Topics are defined in `config/topics.yml` (path configurable via `config_path`):

```yaml
daily_briefing:
  sections:
    - title: "Technology"
      sources:
        - search: "Rust programming language"
          include: ["async", "tokio"]
          exclude: ["game"]
          select-if: "focuses on systems programming"
          freshness: "1d"
    - title: "Science"
      sources:
        - search: "space exploration news"
```

### Source fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `search` | string | — | Base search query |
| `include` | list | `[]` | Terms that must appear (quoted in query) |
| `exclude` | list | `[]` | Terms to exclude (negated in query) |
| `select-if` | string | `""` | Editorial guidance passed to the scoring LLM |
| `freshness` | string | `"1d"` | Time filter: `1d`, `1w`, `1m` |

## Library

The `daily-brief` library crate exports the `Edition` struct so that `corre-news` can
deserialise and render editions without depending on the binary. It also re-exports the core
output types (`Article`, `Section`, `Source`) from `corre-sdk`.

## Building

```sh
cargo build --release -p daily-brief
```

The binary is at `target/release/daily-brief`. Install it as a plugin by placing it alongside
a `manifest.toml` in `~/.local/share/corre/plugins/daily-brief/`.
