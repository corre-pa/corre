//! Shared utility functions for app plugin authors.
//!
//! Covers search result parsing from MCP tool outputs, JSON extraction from LLM responses
//! that may include markdown fencing or prose, Brave Search freshness value normalisation,
//! and LLM error classification helpers (retryable overload detection, context-length parsing).

/// Shared search helpers used by multiple apps.

/// A single search result item from an MCP search tool.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SearchResultItem {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub description: String,
}

/// Parse MCP tool results into search result items. Handles JSON arrays,
/// single objects, and newline-delimited JSON text.
pub fn parse_search_results(value: serde_json::Value) -> Vec<SearchResultItem> {
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
        return vec![];
    }

    if let Ok(items) = serde_json::from_value::<Vec<SearchResultItem>>(value.clone()) {
        let items: Vec<_> = items.into_iter().filter(|i| !i.url.is_empty()).collect();
        if !items.is_empty() {
            return items;
        }
    }

    if let Ok(item) = serde_json::from_value::<SearchResultItem>(value) {
        if !item.url.is_empty() {
            return vec![item];
        }
    }

    vec![]
}

/// Extract a JSON substring from LLM output that may contain markdown fencing
/// or surrounding prose.
pub fn extract_json(text: &str) -> &str {
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

/// Map human-friendly freshness values (1d, 1w, 1m, 1y) to Brave API values (pd, pw, pm, py).
pub fn normalize_freshness(freshness: &str) -> &str {
    match freshness {
        "1d" => "pd",
        "1w" => "pw",
        "1m" => "pm",
        "1y" => "py",
        other => other,
    }
}

/// Check if an LLM error indicates a transient overload that warrants a longer backoff.
pub fn is_retryable_overload(err: &str) -> bool {
    err.contains("429") || err.contains("503") || err.contains("rate limited") || err.contains("overloaded")
}

/// Parse a context-length error to extract the maximum completion tokens the model can accept.
/// Returns `Some(available)` where `available = context_length - input_tokens`, or `None` if
/// the error doesn't match the expected format.
pub fn parse_context_length_limit(err: &str) -> Option<u32> {
    let ctx_marker = "maximum context length is ";
    let input_marker = "your request has ";
    let ctx_start = err.find(ctx_marker)? + ctx_marker.len();
    let ctx_end = err[ctx_start..].find(' ')? + ctx_start;
    let input_start = err.find(input_marker)? + input_marker.len();
    let input_end = err[input_start..].find(' ')? + input_start;
    let ctx: u32 = err[ctx_start..ctx_end].parse().ok()?;
    let input: u32 = err[input_start..input_end].parse().ok()?;
    ctx.checked_sub(input)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(parse_context_length_limit(err), Some(32768 - 2789));
    }

    #[test]
    fn parse_context_length_limit_returns_none_for_unrelated_errors() {
        assert_eq!(parse_context_length_limit("429 Too Many Requests"), None);
        assert_eq!(parse_context_length_limit("connection refused"), None);
    }
}
