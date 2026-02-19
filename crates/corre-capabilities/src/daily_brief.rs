use corre_core::capability::{Capability, CapabilityContext, CapabilityManifest, CapabilityOutput, LlmMessage, LlmRequest, LlmRole};
use corre_core::config::CapabilityConfig;
use corre_core::publish::{Article, Source};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Daily Research Brief capability.
///
/// Pipeline:
/// 1. Parse topics.md -> sections of search queries
/// 2. Search all queries in parallel via brave-search MCP
/// 3. Deduplicate results by URL within each section
/// 4. Score results for newsworthiness (one LLM call per section, parallel)
/// 5. Filter to top 5 per section above score threshold
/// 6. Summarise each top result (parallel LLM calls, semaphore-bounded)
/// 7. Group into sections -> CapabilityOutput
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

#[derive(Debug)]
struct TopicSection {
    name: String,
    queries: Vec<String>,
}

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

fn parse_topics(content: &str) -> Vec<TopicSection> {
    let mut sections = Vec::new();
    let mut current_section: Option<TopicSection> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            if let Some(section) = current_section.take() {
                if !section.queries.is_empty() {
                    sections.push(section);
                }
            }
            current_section = Some(TopicSection { name: heading.trim().to_string(), queries: Vec::new() });
        } else if let Some(query) = trimmed.strip_prefix("- ") {
            if let Some(ref mut section) = current_section {
                section.queries.push(query.trim().to_string());
            }
        }
    }

    if let Some(section) = current_section {
        if !section.queries.is_empty() {
            sections.push(section);
        }
    }

    sections
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
        let items: Vec<SearchResultItem> = text
            .lines()
            .filter_map(|line| serde_json::from_str::<SearchResultItem>(line).ok())
            .filter(|i| !i.url.is_empty())
            .collect();
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
            .ok_or_else(|| anyhow::anyhow!("daily-brief requires a config_path pointing to topics.md"))?;

        let topics_content = std::fs::read_to_string(&config_path)?;
        let sections = parse_topics(&topics_content);
        tracing::info!("Parsed {} topic sections", sections.len());

        // ------------------------------------------------------------------
        // Step 2: Search all queries in parallel (MCP calls are I/O-bound)
        // ------------------------------------------------------------------
        let mut search_handles = Vec::new();
        for section in &sections {
            for query in &section.queries {
                let section_name = section.name.clone();
                let query = query.clone();
                search_handles.push(async move {
                    tracing::info!("Searching: {query}");
                    let search_args = serde_json::json!({ "query": query });
                    match ctx.mcp.call_tool("brave-search", "brave_web_search", search_args).await {
                        Ok(results) => {
                            let items = parse_search_results(&results);
                            tracing::info!("Got {} results for query: {query}", items.len());
                            (section_name, items)
                        }
                        Err(e) => {
                            tracing::warn!("Search failed for query `{query}`: {e}");
                            (section_name, vec![])
                        }
                    }
                });
            }
        }

        let search_results = futures::future::join_all(search_handles).await;

        // Group by section and deduplicate by URL
        let mut section_results: std::collections::HashMap<String, Vec<SearchResultItem>> = std::collections::HashMap::new();
        for (section_name, items) in search_results {
            section_results.entry(section_name).or_default().extend(items);
        }
        for results in section_results.values_mut() {
            let mut seen = std::collections::HashSet::new();
            results.retain(|r| seen.insert(r.url.clone()));
        }

        // ------------------------------------------------------------------
        // Step 4: Score each section's results (parallel LLM calls)
        // ------------------------------------------------------------------
        let semaphore = Arc::new(Semaphore::new(ctx.max_concurrent_llm));

        let mut scoring_handles = Vec::new();
        for (section_name, results) in &section_results {
            if results.is_empty() {
                continue;
            }
            let section_name = section_name.clone();
            let results = results.clone();
            let sem = semaphore.clone();
            scoring_handles.push(async move {
                let _permit = sem.acquire().await.unwrap();
                score_section(&ctx.llm, &section_name, &results).await
            });
        }

        let scored_sections = futures::future::join_all(scoring_handles).await;

        // Flatten into ranked results
        let mut ranked: Vec<RankedResult> = Vec::new();
        for section_ranked in scored_sections {
            ranked.extend(section_ranked);
        }

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
                    "You are a concise news writer. Write a 2-3 paragraph summary of the news item. Be factual and neutral.",
                    format!(
                        "Write a summary for this news item:\nTitle: {title}\nDescription: {desc}\nURL: {url}",
                        title = item.title,
                        desc = item.description,
                        url = item.url,
                    ),
                );

                match ctx.llm.complete(summary_request).await {
                    Ok(response) => {
                        tracing::info!("Summarised: {}", item.title);
                        Some((
                            section_name,
                            Article {
                                title: item.title.clone(),
                                summary: item.description.clone(),
                                body: response.content,
                                sources: vec![Source { title: item.title, url: item.url }],
                                score,
                            },
                        ))
                    }
                    Err(e) => {
                        tracing::warn!("Failed to summarise `{}`: {e}", item.title);
                        None
                    }
                }
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

        let articles: Vec<Article> = article_map.values().flatten().cloned().collect();

        Ok(CapabilityOutput { capability_name: self.manifest.name.clone(), produced_at: chrono::Utc::now(), articles })
    }
}

/// Score a section's search results for newsworthiness via a single LLM call.
/// Returns the top 5 results above the threshold.
async fn score_section(
    llm: &Box<dyn corre_core::capability::LlmProvider>,
    section_name: &str,
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
                    Only include results with score > 0.4."
                    .into(),
            },
            LlmMessage {
                role: LlmRole::User,
                content: format!("Score these search results for the \"{section_name}\" section:\n{results_json}"),
            },
        ],
        temperature: Some(0.1),
        max_tokens: Some(2048),
        json_mode: false,
    };

    let scoring_response = match llm.complete(scoring_request).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Scoring failed for section `{section_name}`: {e}");
            return vec![];
        }
    };

    #[derive(serde::Deserialize)]
    struct ScoredItem {
        index: usize,
        score: f64,
        #[allow(dead_code)]
        reasoning: String,
    }

    let json_str = extract_json(&scoring_response.content);
    let scored: Vec<ScoredItem> = serde_json::from_str(json_str).unwrap_or_default();
    tracing::info!("Scored {} results above threshold for section `{section_name}`", scored.len());

    let mut top: Vec<_> = scored.into_iter().map(|s| (s.index, s.score)).collect();
    top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    top.truncate(5);

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
    fn parse_topics_md() {
        let content = r#"# Daily Brief Topics

Each section becomes a section in CorreNews.

## Technology
- Rust programming language news
- AI and machine learning developments

## World News
- Geopolitics and international relations
"#;
        let sections = parse_topics(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "Technology");
        assert_eq!(sections[0].queries.len(), 2);
        assert_eq!(sections[1].name, "World News");
        assert_eq!(sections[1].queries.len(), 1);
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
