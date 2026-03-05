//! Standalone daily-brief app binary.
//!
//! Communicates with the host via the CCPP protocol over stdin/stdout using
//! [`corre_sdk::AppClient`]. This binary has no dependency on `corre-core`
//! — it uses only `corre-sdk` types and utilities.

use anyhow::Context as _;
use corre_sdk::html::{sanitize_html, sanitize_url};
use corre_sdk::tools::{
    SearchResultItem, extract_json, is_retryable_overload, normalize_freshness, parse_context_length_limit, parse_search_results,
};
use corre_sdk::types::{AppOutput, Article, Section, Source};
use corre_sdk::{AppClient, LlmMessage, LlmRequest, LlmRole};
use daily_brief::Edition;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// YAML config model
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct TopicsConfig {
    #[serde(rename = "daily-briefing")]
    daily_briefing: DailyBriefing,
}

#[derive(Debug, serde::Deserialize)]
struct DailyBriefing {
    sections: Vec<TopicSection>,
}

#[derive(Debug, serde::Deserialize)]
struct TopicSection {
    title: String,
    sources: Vec<TopicSource>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TopicSource {
    search: String,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(rename = "select-if", default)]
    select_if: String,
    #[serde(default = "default_freshness")]
    freshness: String,
}

fn default_freshness() -> String {
    "1d".into()
}

fn load_topics(content: &str) -> anyhow::Result<Vec<TopicSection>> {
    let config: TopicsConfig = serde_yaml_ng::from_str(content)?;
    Ok(config.daily_briefing.sections)
}

fn build_query(source: &TopicSource) -> String {
    let mut query = source.search.clone();
    for inc in &source.include {
        write!(query, " \"{inc}\"").unwrap();
    }
    for exc in &source.exclude {
        write!(query, " -{exc}").unwrap();
    }
    query
}

// ---------------------------------------------------------------------------
// Self-dedup: read seen URLs from previous editions on disk
// ---------------------------------------------------------------------------

/// Collect all article URLs from recent editions in the editions directory.
fn load_seen_urls(editions_dir: &Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(editions_dir) else {
        return HashSet::new();
    };

    entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path().join("edition.json");
            std::fs::read_to_string(&path).ok()
        })
        .filter_map(|content| serde_json::from_str::<Edition>(&content).ok())
        .flat_map(|ed| ed.sections)
        .flat_map(|section| section.articles)
        .flat_map(|article| article.sources)
        .map(|source| source.url)
        .collect()
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ERROR daily-brief failed: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let client = Arc::new(AppClient::from_stdio());
    let params = client.accept_initialize().await?;
    let _guard = corre_sdk::init_tracing(&params.app_name, params.log_dir.as_deref(), params.log_level.as_deref());

    let config_dir = PathBuf::from(&params.config_dir);
    let editions_dir = config_dir.join("editions");
    let max_concurrent_llm = params.max_concurrent_llm;

    // Self-dedup: read seen URLs from previous editions on disk
    let seen_urls = load_seen_urls(&editions_dir);
    tracing::info!("Loaded {} previously seen URLs from editions", seen_urls.len());

    // ── Step 1: Load and parse topics ────────────────────────────────────
    let config_path = params
        .config_path
        .as_ref()
        .map(|p| config_dir.join(p))
        .ok_or_else(|| anyhow::anyhow!("daily-brief requires a config_path pointing to topics.yml"))?;

    let topics_content =
        std::fs::read_to_string(&config_path).with_context(|| format!("failed to read topics file {}", config_path.display()))?;
    let sections = load_topics(&topics_content)?;
    tracing::info!("Parsed {} topic sections", sections.len());
    client.report_progress("parsed_topics", None, None).await?;

    // ── Step 2-3: Search via Brave web + news ────────────────────────────
    type SearchResult = (String, usize, Vec<SearchResultItem>);
    type SearchFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = SearchResult> + Send + 'a>>;
    let mut search_handles: Vec<SearchFuture<'_>> = Vec::new();

    for section in &sections {
        for (src_idx, source) in section.sources.iter().enumerate() {
            let query = build_query(source);
            let freshness = normalize_freshness(&source.freshness).to_string();
            let section_title = section.title.clone();

            for (tool, label) in [("brave_web_search", "Web"), ("brave_news_search", "News")] {
                let sec = section_title.clone();
                let q = query.clone();
                let f = freshness.clone();
                let client = client.clone();
                search_handles.push(Box::pin(async move {
                    tracing::info!("{label} searching: {q}");
                    let args = serde_json::json!({ "query": q, "freshness": f });
                    match client.call_tool("brave-search", tool, args).await {
                        Ok(results) => {
                            let items = parse_search_results(results);
                            tracing::info!("Got {} {label} results for: {q}", items.len());
                            (sec, src_idx, items)
                        }
                        Err(e) => {
                            tracing::warn!("{label} search failed for `{q}`: {e}");
                            (sec, src_idx, vec![])
                        }
                    }
                }));
            }
        }
    }

    let search_results = futures::future::join_all(search_handles).await;
    client.report_progress("searches_complete", None, None).await?;

    // Group results by (section_title, source_index)
    let mut source_results: HashMap<(String, usize), Vec<SearchResultItem>> = HashMap::new();
    for (section_title, src_idx, items) in search_results {
        source_results.entry((section_title, src_idx)).or_default().extend(items);
    }

    // Deduplicate by URL within each source
    for results in source_results.values_mut() {
        let mut seen = HashSet::new();
        results.retain(|r| seen.insert(r.url.clone()));
    }

    // Cross-edition dedup
    if !seen_urls.is_empty() {
        let before: usize = source_results.values().map(|r| r.len()).sum();
        for results in source_results.values_mut() {
            results.retain(|r| !seen_urls.contains(&r.url));
        }
        let after: usize = source_results.values().map(|r| r.len()).sum();
        if before != after {
            tracing::info!("Cross-edition dedup removed {} previously seen URLs", before - after);
        }
    }

    client.report_progress("dedup_complete", None, None).await?;

    // ── Step 4-6: Score and summarise (parallel LLM calls) ───────────────
    let semaphore = Arc::new(Semaphore::new(max_concurrent_llm));

    let total_sources: usize = sections
        .iter()
        .flat_map(|s| s.sources.iter().enumerate().map(move |(i, _)| (s.title.clone(), i)))
        .filter(|key| source_results.get(key).is_some_and(|r| !r.is_empty()))
        .count();

    client.report_progress("scoring_and_summarizing", None, Some(&format!("{total_sources} sources to process"))).await?;

    let mut handles = Vec::new();
    for section in &sections {
        for (src_idx, source) in section.sources.iter().enumerate() {
            let key = (section.title.clone(), src_idx);
            let results = match source_results.get(&key) {
                Some(r) if !r.is_empty() => r.clone(),
                _ => continue,
            };
            let section_name = section.title.clone();
            let select_if = source.select_if.clone();
            let sem = semaphore.clone();
            let client = client.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                score_and_summarize_source(&client, &section_name, &select_if, &results).await
            }));
        }
    }

    let batches: Vec<Vec<(String, Article)>> = futures::future::join_all(handles)
        .await
        .into_iter()
        .filter_map(|r| match r {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::error!("score+summarize task failed: {e}");
                None
            }
        })
        .collect();

    // ── Step 7: Group into sections ──────────────────────────────────────
    let mut article_map: HashMap<String, Vec<Article>> = HashMap::new();
    for batch in batches {
        for (section_name, article) in batch {
            article_map.entry(section_name).or_default().push(article);
        }
    }

    // Preserve section ordering from the YAML
    let output_sections: Vec<Section> =
        sections.iter().filter_map(|s| article_map.remove(&s.title).map(|articles| Section { title: s.title.clone(), articles })).collect();

    let output = AppOutput {
        app_name: "daily-brief".into(),
        produced_at: chrono::Utc::now(),
        sections: output_sections.clone(),
        content_type: Default::default(),
        custom_content: None,
    };

    // ── Step 8: Build Edition, generate tagline, write to disk ───────────
    let today = chrono::Utc::now().date_naive();
    let mut edition = Edition::new(today, output_sections);

    client.report_progress("generating_tagline", Some(90), None).await?;

    // Generate a dad joke tagline inspired by the headline
    let tagline_request = LlmRequest {
        messages: vec![
            LlmMessage {
                role: LlmRole::System,
                content: "You are a newspaper sub-editor who writes witty taglines. Write a single short dad joke or pun \
                          (max 15 words) inspired by the given headline. Just the joke, no quotes, no explanation."
                    .into(),
            },
            LlmMessage { role: LlmRole::User, content: edition.headline.clone() },
        ],
        temperature: Some(0.9),
        max_completion_tokens: Some(200),
        json_mode: false,
    };
    match client.llm_complete(tagline_request).await {
        Ok(resp) => {
            let tagline = resp.content.trim().trim_matches('"').to_string();
            if !tagline.is_empty() {
                edition.tagline = tagline;
            }
        }
        Err(e) => tracing::warn!("Failed to generate tagline, using default: {e}."),
    }

    // Write edition JSON via the host's output/write
    let edition_path = format!("editions/{}/edition.json", today.format("%Y-%m-%d"));
    let edition_json = serde_json::to_string_pretty(&edition)?;
    client.write_file(&edition_path, &edition_json, Some("application/json")).await?;
    tracing::info!("Edition written to {edition_path}");

    // Still send the AppOutput so the host can track it
    client.send_result(output).await?;
    Ok(())
}

/// Score and summarise a source's search results in a single LLM call.
async fn score_and_summarize_source(
    client: &AppClient<tokio::io::Stdout>,
    section_name: &str,
    select_if: &str,
    results: &[SearchResultItem],
) -> Vec<(String, Article)> {
    let results_json = serde_json::to_string(
        &results
            .iter()
            .enumerate()
            .map(|(i, r)| {
                serde_json::json!({
                    "index": i,
                    "title": r.title,
                    "url": r.url,
                    "description": r.description,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();

    let request = LlmRequest {
        messages: vec![
            LlmMessage {
                role: LlmRole::System,
                content: "You are a news editor and writer. For each search result:\n\
                    1. Score it for newsworthiness from 0.0 to 1.0.\n\
                    2. Write a one-sentence summary (max 30 words) with the main takeaway.\n\
                    3. Write a factual body (under 500 words) sticking to concrete facts, names, numbers, and dates.\n\n\
                    Respond with ONLY a raw JSON array, no markdown fencing, no explanation.\n\
                    Each element: {\"index\": <number>, \"score\": <number>, \"summary\": \"<string>\", \"body\": \"<string>\"}\n\
                    Include ALL results."
                    .into(),
            },
            LlmMessage {
                role: LlmRole::User,
                content: if select_if.is_empty() {
                    format!("Score and summarise these search results for the \"{section_name}\" section:\n{results_json}")
                } else {
                    format!(
                        "Score and summarise these search results for the \"{section_name}\" section.\n\
                        Editorial guidance: {select_if}\n{results_json}"
                    )
                },
            },
        ],
        temperature: Some(0.1),
        max_completion_tokens: None,
        json_mode: false,
    };

    #[derive(serde::Deserialize)]
    struct ScoredSummary {
        index: usize,
        score: f64,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        body: String,
    }

    let mut request = request;
    let mut parsed: Option<Vec<ScoredSummary>> = None;
    for attempt in 0..3u64 {
        let response = match client.llm_complete(request.clone()).await {
            Ok(r) => r,
            Err(e) => {
                let err_str = e.to_string();
                let backoff = 5 << attempt;
                if let Some(available) = parse_context_length_limit(&err_str) {
                    tracing::info!(
                        "Source `{section_name}` max_completion_tokens too large (attempt {}), reducing to {available}",
                        attempt + 1
                    );
                    request.max_completion_tokens = Some(available);
                } else if is_retryable_overload(&err_str) {
                    tracing::info!("Source `{section_name}` rate limited (attempt {}), backing off {backoff}s", attempt + 1);
                } else {
                    tracing::info!("LLM call for source `{section_name}` failed (attempt {}): {err_str}", attempt + 1);
                }
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                continue;
            }
        };

        let json_str = extract_json(&response.content);
        match serde_json::from_str::<Vec<ScoredSummary>>(json_str) {
            Ok(items) => {
                parsed = Some(items);
                break;
            }
            Err(e) => {
                let backoff = 5 << attempt;
                tracing::info!(
                    "JSON parse failed for source `{section_name}` (attempt {}): {e}. Raw: {}",
                    attempt + 1,
                    &response.content[..response.content.len().min(200)]
                );
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            }
        }
    }

    let mut items = match parsed {
        Some(items) => items,
        None => {
            tracing::warn!("Score+summarise failed for source `{section_name}` after 3 attempts");
            return vec![];
        }
    };

    // Filter out low-scoring items, sort by score desc, and keep top 10.
    items.retain(|i| i.score > 0.2);
    items.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    items.truncate(10);

    tracing::info!("Scored and summarised {} articles for source `{section_name}`", items.len());

    items
        .into_iter()
        .filter_map(|scored| {
            results.get(scored.index).map(|item| {
                let summary = if scored.summary.is_empty() { &item.description } else { &scored.summary };
                let body = if scored.body.is_empty() { &item.description } else { &scored.body };
                let article = Article {
                    title: item.title.clone(),
                    summary: sanitize_html(summary),
                    body: sanitize_html(body),
                    sources: vec![Source { title: item.title.clone(), url: sanitize_url(&item.url) }],
                    score: scored.score,
                };
                (section_name.to_string(), article)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_topics_yaml_round_trip() {
        let yaml = r#"
daily-briefing:
  sections:
    - title: "World News"
      sources:
        - search: "latest news"
          include:
            - "economics"
          exclude:
            - "politics"
          select-if: "General interest news from reputable sources."
          freshness: "1d"
        - search: "europe news"
          include: []
          exclude: []
          select-if: ""
    - title: "Sports"
      sources:
        - search: "rugby news"
          include:
            - "six nations"
          exclude:
            - "Planet Rugby"
          select-if: "Match reports and analysis."
          freshness: "1w"
"#;
        let sections = load_topics(yaml).unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].title, "World News");
        assert_eq!(sections[0].sources.len(), 2);
        assert_eq!(sections[0].sources[0].search, "latest news");
        assert_eq!(sections[0].sources[0].include, vec!["economics"]);
        assert_eq!(sections[0].sources[0].exclude, vec!["politics"]);
        assert_eq!(sections[0].sources[0].freshness, "1d");
        assert_eq!(sections[0].sources[1].freshness, "1d"); // default
        assert_eq!(sections[1].title, "Sports");
        assert_eq!(sections[1].sources[0].freshness, "1w");
    }

    #[test]
    fn build_query_includes_and_excludes() {
        let source = TopicSource {
            search: "rugby news".into(),
            include: vec!["six nations".into(), "world cup".into()],
            exclude: vec!["Planet Rugby".into()],
            select_if: String::new(),
            freshness: "1d".into(),
        };
        assert_eq!(build_query(&source), "rugby news \"six nations\" \"world cup\" -Planet Rugby");
    }

    #[test]
    fn build_query_no_modifiers() {
        let source = TopicSource {
            search: "international cricket news".into(),
            include: vec![],
            exclude: vec![],
            select_if: String::new(),
            freshness: "1w".into(),
        };
        assert_eq!(build_query(&source), "international cricket news");
    }
}
