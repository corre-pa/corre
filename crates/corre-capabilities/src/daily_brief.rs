use corre_core::capability::{
    Capability, CapabilityContext, CapabilityManifest, CapabilityOutput, LlmMessage, LlmRequest, LlmRole, ProgressStatus, ProgressTracker,
};
use corre_core::config::CapabilityConfig;
use corre_core::publish::{Article, Section, Source, sanitize_html, sanitize_url};
use std::fmt::Write;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Daily Research Brief capability.
///
/// Pipeline:
/// 1. Parse topics.yml -> sections of sources with per-source search config
/// 2. Build one query per source (search + include phrases + exclude operators)
/// 3. Search each query via brave_web_search AND brave_news_search in parallel
/// 4. Deduplicate results by URL within each source, then cross-edition dedup
/// 5. Score and summarise results in a single LLM call per source (parallel, semaphore-bounded)
/// 6. Keep top 10 per source by score
/// 7. Group into sections -> CapabilityOutput
pub struct DailyBrief {
    manifest: CapabilityManifest,
    tracker: ProgressTracker,
}

impl DailyBrief {
    pub fn from_config(config: &CapabilityConfig) -> Self {
        Self {
            tracker: ProgressTracker::new(&config.name),
            manifest: CapabilityManifest {
                name: config.name.clone(),
                description: config.description.clone(),
                schedule: config.schedule.clone(),
                mcp_servers: config.mcp_servers.clone(),
                config_path: config.config_path.clone(),
            },
        }
    }
}

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

/// Check if an LLM error indicates a transient overload that warrants a longer backoff.
fn is_retryable_overload(err: &str) -> bool {
    err.contains("429") || err.contains("503") || err.contains("rate limited") || err.contains("overloaded")
}

/// Parse a context-length error to extract the maximum completion tokens the model can accept.
/// Returns `Some(available)` where `available = context_length - input_tokens`, or `None` if
/// the error doesn't match the expected format.
fn parse_context_length_limit(err: &str) -> Option<u32> {
    // Pattern: "maximum context length is {ctx} tokens and your request has {input} input tokens"
    let ctx_marker = "maximum context length is ";
    let input_marker = "your request has ";
    let ctx_start = err.find(ctx_marker)? + ctx_marker.len();
    let ctx_end = err[ctx_start..].find(' ')? + ctx_start;
    let input_start = err.find(input_marker)? + input_marker.len();
    let input_end = err[input_start..].find(' ')? + input_start;
    let ctx: u32 = err[ctx_start..ctx_end].parse().ok()?;
    let input: u32 = err[input_start..input_end].parse().ok()?;
    ctx.checked_sub(input)?.checked_sub(100) // leave a safety margin of 100 tokens for the response
}

/// Map human-friendly freshness values (1d, 1w, 1m, 1y) to Brave API values (pd, pw, pm, py).
fn normalize_freshness(freshness: &str) -> &str {
    match freshness {
        "1d" => "pd",
        "1w" => "pw",
        "1m" => "pm",
        "1y" => "py",
        other => other,
    }
}

fn load_topics(content: &str) -> anyhow::Result<Vec<TopicSection>> {
    let config: TopicsConfig = serde_yaml_ng::from_str(content)?;
    Ok(config.daily_briefing.sections)
}

/// Build a single Brave search query from a source entry.
/// Include terms are quoted must-haves, exclude terms are negated.
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
// Search result parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct SearchResultItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    #[allow(dead_code)]
    extra_snippets: Vec<String>,
}

/// Extract a JSON substring from LLM output that may contain markdown fencing
/// or surrounding prose.
fn extract_json(text: &str) -> &str {
    let trimmed = text.trim();
    // Strip ```json ... ``` fences
    if let Some(start) = trimmed.find("```json") {
        let after_fence = &trimmed[start + 7..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }
    // Find outermost [ ] or { }
    if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            return &trimmed[start..=end];
        }
    }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }
    trimmed
}

/// Parse MCP tool results into search result items. Handles JSON arrays,
/// single objects, and newline-delimited JSON text.
fn parse_search_results(value: &serde_json::Value) -> Vec<SearchResultItem> {
    if let Ok(items) = serde_json::from_value::<Vec<SearchResultItem>>(value.clone()) {
        let items: Vec<_> = items.into_iter().filter(|i| !i.url.is_empty()).collect();
        if !items.is_empty() {
            return items;
        }
    }

    if let Ok(item) = serde_json::from_value::<SearchResultItem>(value.clone()) {
        if !item.url.is_empty() {
            return vec![item];
        }
    }

    if let Some(text) = value.as_str() {
        if let Ok(items) = serde_json::from_str::<Vec<SearchResultItem>>(text) {
            return items.into_iter().filter(|i| !i.url.is_empty()).collect();
        }
        let items: Vec<SearchResultItem> =
            text.lines().filter_map(|line| serde_json::from_str::<SearchResultItem>(line).ok()).filter(|i| !i.url.is_empty()).collect();
        if !items.is_empty() {
            return items;
        }
        tracing::debug!("Could not parse search results from text ({} chars)", text.len());
    } else {
        tracing::debug!("Unexpected search result shape: {value}");
    }

    vec![]
}

// ---------------------------------------------------------------------------
// Capability implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl Capability for DailyBrief {
    fn manifest(&self) -> &CapabilityManifest {
        &self.manifest
    }

    async fn execute(&self, ctx: &CapabilityContext) -> anyhow::Result<CapabilityOutput> {
        self.tracker.reset();

        let config_path = self
            .manifest
            .config_path
            .as_ref()
            .map(|p| ctx.config_dir.join(p))
            .ok_or_else(|| anyhow::anyhow!("daily-brief requires a config_path pointing to topics.yml"))?;

        let topics_content = std::fs::read_to_string(&config_path)?;
        let sections = load_topics(&topics_content)?;
        tracing::info!("Parsed {} topic sections", sections.len());
        self.tracker.touch("parsed_topics");

        // ------------------------------------------------------------------
        // Step 2-3: Build one query per source and search via web + news
        // ------------------------------------------------------------------
        // Each future returns (section_title, source_index, results) so we
        // can track which source produced which results.
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
                    search_handles.push(Box::pin(async move {
                        tracing::info!("{label} searching: {q}");
                        let args = serde_json::json!({ "query": q, "freshness": f });
                        match ctx.mcp.call_tool("brave-search", tool, args).await {
                            Ok(results) => {
                                let items = parse_search_results(&results);
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
        self.tracker.touch("searches_complete");

        // Group results by (section_title, source_index)
        let mut source_results: std::collections::HashMap<(String, usize), Vec<SearchResultItem>> = std::collections::HashMap::new();
        for (section_title, src_idx, items) in search_results {
            source_results.entry((section_title, src_idx)).or_default().extend(items);
        }

        // Deduplicate by URL within each source
        for results in source_results.values_mut() {
            let mut seen = std::collections::HashSet::new();
            results.retain(|r| seen.insert(r.url.clone()));
        }

        // Cross-edition dedup: remove URLs that appeared in previous editions
        if !ctx.seen_urls.is_empty() {
            let before: usize = source_results.values().map(|r| r.len()).sum();
            for results in source_results.values_mut() {
                results.retain(|r| !ctx.seen_urls.contains(&r.url));
            }
            let after: usize = source_results.values().map(|r| r.len()).sum();
            if before != after {
                tracing::info!("Cross-edition dedup removed {} previously seen URLs", before - after);
            }
        }

        self.tracker.touch("dedup_complete");

        // ------------------------------------------------------------------
        // Step 4-6: Score and summarise each source (parallel LLM calls)
        // ------------------------------------------------------------------
        let semaphore = Arc::new(Semaphore::new(ctx.max_concurrent_llm));

        let total_sources: usize = sections
            .iter()
            .enumerate()
            .flat_map(|(_, s)| s.sources.iter().enumerate().map(move |(i, _)| (s.title.clone(), i)))
            .filter(|key| source_results.get(key).is_some_and(|r| !r.is_empty()))
            .count();
        self.tracker.set_expected(total_sources);
        self.tracker.touch("scoring_and_summarizing");
        let tracker = &self.tracker;

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
                handles.push(async move {
                    let _permit = sem.acquire().await.unwrap();
                    let articles = score_and_summarize_source(&ctx.llm, &section_name, &select_if, &results).await;
                    for (sec, article) in &articles {
                        tracker.add_article(sec.clone(), article.clone());
                    }
                    articles
                });
            }
        }

        let batches = futures::future::join_all(handles).await;

        // ------------------------------------------------------------------
        // Step 7: Group into sections
        // ------------------------------------------------------------------
        let mut article_map: std::collections::HashMap<String, Vec<Article>> = std::collections::HashMap::new();
        for batch in batches {
            for (section_name, article) in batch {
                article_map.entry(section_name).or_default().push(article);
            }
        }

        // Preserve section ordering from the YAML
        let output_sections: Vec<Section> = sections
            .iter()
            .filter_map(|s| article_map.remove(&s.title).map(|articles| Section { title: s.title.clone(), articles }))
            .collect();

        Ok(CapabilityOutput { capability_name: self.manifest.name.clone(), produced_at: chrono::Utc::now(), sections: output_sections })
    }

    async fn in_progress(&self) -> ProgressStatus {
        // 120s exceeds worst-case single LLM call with full rate-limit retries (~90s = 3 attempts x 30s backoff)
        const STALENESS_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(120);
        self.tracker.evaluate(STALENESS_THRESHOLD)
    }
}

/// Score and summarise a source's search results in a single LLM call.
/// Returns the top 10 results as fully-built Articles.
async fn score_and_summarize_source(
    llm: &Box<dyn corre_core::capability::LlmProvider>,
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
        let response = match llm.complete(request.clone()).await {
            Ok(r) => r,
            Err(e) => {
                let err_str = e.to_string();
                let backoff = 5 << attempt;
                if let Some(available) = parse_context_length_limit(&err_str) {
                    tracing::info!("Source `{section_name}` max_completion_tokens too large (attempt {}), reducing to {available}",attempt + 1);
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
    items = items.into_iter().filter(|i| i.score > 0.2).collect();
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
        assert_eq!(sections[0].sources[0].select_if, "General interest news from reputable sources.");
        assert_eq!(sections[0].sources[0].freshness, "1d");

        // Second source uses defaults
        assert_eq!(sections[0].sources[1].search, "europe news");
        assert!(sections[0].sources[1].include.is_empty());
        assert!(sections[0].sources[1].exclude.is_empty());
        assert_eq!(sections[0].sources[1].select_if, "");
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

    #[test]
    fn build_query_excludes_only() {
        let source = TopicSource {
            search: "astronomy".into(),
            include: vec![],
            exclude: vec!["Avi Loeb".into(), "UFO".into()],
            select_if: String::new(),
            freshness: "1d".into(),
        };
        assert_eq!(build_query(&source), "astronomy -Avi Loeb -UFO");
    }

    #[test]
    fn extract_json_from_fenced_block() {
        let input = "Here is the result:\n```json\n[{\"x\": 1}]\n```\nDone.";
        assert_eq!(extract_json(input), "[{\"x\": 1}]");
    }

    #[test]
    fn extract_json_bare_array() {
        let input = "Some preamble [{\"a\":1}] trailing";
        assert_eq!(extract_json(input), "[{\"a\":1}]");
    }

    #[test]
    fn parse_context_length_limit_extracts_available_tokens() {
        let err = "LLM API returned 400 Bad Request: {\"error\":\"'max_tokens' or 'max_completion_tokens' is too large: 65536. \
            This model's maximum context length is 32768 tokens and your request has 2789 input tokens (65536 > 32768 - 2789). \
            None\",\"request_id\":\"DERYTCex_4QMLyruMM8UV\"}";
        assert_eq!(parse_context_length_limit(err), Some(32768 - 2789 - 100));
    }

    #[test]
    fn parse_context_length_limit_returns_none_for_unrelated_errors() {
        assert_eq!(parse_context_length_limit("429 Too Many Requests"), None);
        assert_eq!(parse_context_length_limit("connection refused"), None);
    }
}
