use corre_core::capability::{Capability, CapabilityContext, CapabilityManifest, CapabilityOutput, LlmMessage, LlmRequest, LlmRole};
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
/// 5. Score results for newsworthiness (one LLM call per source, parallel)
/// 6. Keep top 10 per source by score
/// 8. Summarise each top result (parallel LLM calls, semaphore-bounded)
/// 9. Group into sections -> CapabilityOutput
pub struct DailyBrief {
    manifest: CapabilityManifest,
}

impl DailyBrief {
    pub fn from_config(config: &CapabilityConfig) -> Self {
        Self {
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

/// A scored, deduplicated search result ready for summarisation.
struct RankedResult {
    section_name: String,
    item: SearchResultItem,
    score: f64,
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
        let config_path = self
            .manifest
            .config_path
            .as_ref()
            .map(|p| ctx.config_dir.join(p))
            .ok_or_else(|| anyhow::anyhow!("daily-brief requires a config_path pointing to topics.yml"))?;

        let topics_content = std::fs::read_to_string(&config_path)?;
        let sections = load_topics(&topics_content)?;
        tracing::info!("Parsed {} topic sections", sections.len());

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

        // ------------------------------------------------------------------
        // Step 4-5: Score each source's results (parallel LLM calls)
        // ------------------------------------------------------------------
        let semaphore = Arc::new(Semaphore::new(ctx.max_concurrent_llm));

        let mut scoring_handles = Vec::new();
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
                scoring_handles.push(async move {
                    let _permit = sem.acquire().await.unwrap();
                    score_source(&ctx.llm, &section_name, &select_if, &results).await
                });
            }
        }

        let scored_batches = futures::future::join_all(scoring_handles).await;

        let ranked: Vec<RankedResult> = scored_batches.into_iter().flatten().collect();
        tracing::info!("{} articles to summarise across all sections", ranked.len());

        // ------------------------------------------------------------------
        // Step 6: Summarise each top result (parallel, semaphore-bounded)
        // ------------------------------------------------------------------
        let mut summary_handles = Vec::new();
        for result in &ranked {
            let section_name = result.section_name.clone();
            let item = result.item.clone();
            let score = result.score;
            let sem = semaphore.clone();
            summary_handles.push(async move {
                let _permit = sem.acquire().await.unwrap();
                let summary_request = LlmRequest::simple(
                    "You are a precise news writer. You respond ONLY with raw JSON, no markdown fencing.\n\
                    Given a news item, produce a JSON object with two fields:\n\
                    - \"summary\": One sentence (max 30 words) delivering the single main takeaway. No filler.\n\
                    - \"body\": A factual precis of the article in under 500 words. Stick strictly to concrete facts, \
                    names, numbers, and dates from the source. Do not editorialize or pad with generic context. \
                    The reader will decide whether to click through to the full article.",
                    format!(
                        "Title: {title}\nDescription: {desc}\nURL: {url}",
                        title = item.title,
                        desc = item.description,
                        url = item.url,
                    ),
                );

                let mut response = None;
                for attempt in 0..3 {
                    match ctx.llm.complete(summary_request.clone()).await {
                        Ok(r) => {
                            response = Some(r);
                            break;
                        }
                        Err(e) => {
                            let err_str = e.to_string();
                            let is_rate_limit = err_str.contains("rate limited") || err_str.contains("429");
                            let backoff = if is_rate_limit { 10 * (attempt as u64 + 1) } else { 1u64 << attempt };
                            tracing::info!("Summary LLM call for `{}` failed (attempt {}): {err_str}", item.title, attempt + 1);
                            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                        }
                    }
                }

                let Some(response) = response else {
                    tracing::warn!("Failed to summarise `{}` after 3 attempts", item.title);
                    return None;
                };

                tracing::info!("Summarised: {}", item.title);
                let json_str = extract_json(&response.content);
                let (summary, body) = match serde_json::from_str::<serde_json::Value>(json_str) {
                    Ok(obj) => {
                        let s = obj.get("summary").and_then(|v| v.as_str()).unwrap_or(&item.description);
                        let b = obj.get("body").and_then(|v| v.as_str()).unwrap_or(&response.content);
                        (s.to_string(), b.to_string())
                    }
                    Err(_) => {
                        tracing::warn!("Failed to parse summary JSON for `{}`, using raw response", item.title);
                        (item.description.clone(), response.content.clone())
                    }
                };
                Some((
                    section_name,
                    Article {
                        title: item.title.clone(),
                        summary: sanitize_html(&summary),
                        body: sanitize_html(&body),
                        sources: vec![Source { title: item.title, url: sanitize_url(&item.url) }],
                        score,
                    },
                ))
            });
        }

        let summaries = futures::future::join_all(summary_handles).await;

        // ------------------------------------------------------------------
        // Step 7: Group into sections
        // ------------------------------------------------------------------
        let mut article_map: std::collections::HashMap<String, Vec<Article>> = std::collections::HashMap::new();
        for pair in summaries.into_iter().flatten() {
            article_map.entry(pair.0).or_default().push(pair.1);
        }

        // Preserve section ordering from the YAML
        let output_sections: Vec<Section> = sections
            .iter()
            .filter_map(|s| article_map.remove(&s.title).map(|articles| Section { title: s.title.clone(), articles }))
            .collect();

        Ok(CapabilityOutput { capability_name: self.manifest.name.clone(), produced_at: chrono::Utc::now(), sections: output_sections })
    }
}

/// Score a source's search results for newsworthiness via a single LLM call.
/// Returns the top 10 results above the threshold.
async fn score_source(
    llm: &Box<dyn corre_core::capability::LlmProvider>,
    section_name: &str,
    select_if: &str,
    results: &[SearchResultItem],
) -> Vec<RankedResult> {
    let results_json = serde_json::to_string(
        &results
            .iter()
            .enumerate()
            .map(|(i, r)| {
                serde_json::json!({
                    "index": i,
                    "title": r.title,
                    "description": r.description,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();

    let scoring_request = LlmRequest {
        messages: vec![
            LlmMessage {
                role: LlmRole::System,
                content: "You are a news editor. Score each result for newsworthiness from 0.0 to 1.0.\n\
                    Respond with ONLY a raw JSON array, no markdown fencing, no explanation.\n\
                    Each element: {\"index\": <number>, \"score\": <number>, \"reasoning\": \"<string>\"}\n\
                    Include ALL results in the response."
                    .into(),
            },
            LlmMessage {
                role: LlmRole::User,
                content: if select_if.is_empty() {
                    format!("Score these search results for the \"{section_name}\" section:\n{results_json}")
                } else {
                    format!(
                        "Score these search results for the \"{section_name}\" section.\nEditorial guidance: {select_if}\n{results_json}"
                    )
                },
            },
        ],
        temperature: Some(0.1),
        max_tokens: Some(2048),
        json_mode: false,
    };

    #[derive(serde::Deserialize)]
    struct ScoredItem {
        index: usize,
        score: f64,
        #[allow(dead_code)]
        reasoning: String,
    }

    let mut scored: Option<Vec<ScoredItem>> = None;
    for attempt in 0..3 {
        let response = match llm.complete(scoring_request.clone()).await {
            Ok(r) => r,
            Err(e) => {
                let err_str = e.to_string();
                let is_rate_limit = err_str.contains("rate limited") || err_str.contains("429");
                let backoff = if is_rate_limit {
                    let secs = 10 * (attempt as u64 + 1);
                    tracing::info!("Scoring for section `{section_name}` rate limited (attempt {}), backing off {secs}s", attempt + 1);
                    secs
                } else {
                    let secs = 1u64 << attempt;
                    tracing::info!("Scoring LLM call for section `{section_name}` failed (attempt {}): {err_str}", attempt + 1);
                    secs
                };
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                continue;
            }
        };

        let json_str = extract_json(&response.content);
        match serde_json::from_str::<Vec<ScoredItem>>(json_str) {
            Ok(items) => {
                scored = Some(items);
                break;
            }
            Err(e) => {
                tracing::info!(
                    "Scoring JSON parse failed for section `{section_name}` (attempt {}): {e}. Raw: {}",
                    attempt + 1,
                    &response.content[..response.content.len().min(200)]
                );
                tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            }
        }
    }

    let scored = match scored {
        Some(items) => items,
        None => {
            tracing::warn!("Scoring failed for section `{section_name}` after 3 attempts");
            return vec![];
        }
    };
    scored.iter().for_each(|scored_item| {
        tracing::debug!(
            "Scoring result. {section_name}#{} -> {:0.2}. Reasoning: {}",
            scored_item.index,
            scored_item.score,
            scored_item.reasoning
        );
    });
    tracing::info!("Scored {} results above threshold for section `{section_name}`", scored.len());

    let mut top: Vec<_> = scored.into_iter().map(|s| (s.index, s.score)).collect();
    top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    top.truncate(10);

    top.into_iter()
        .filter_map(|(idx, score)| {
            results.get(idx).map(|item| RankedResult { section_name: section_name.to_string(), item: item.clone(), score })
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
}
