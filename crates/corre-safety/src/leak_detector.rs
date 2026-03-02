//! Stage 3 of the safety pipeline: detection and redaction of leaked credentials.
//!
//! Uses a two-phase approach -- Aho-Corasick prefix scanning followed by regex
//! confirmation -- to redact API keys, cloud credentials, tokens, PEM headers,
//! and high-entropy hex strings.

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use regex::Regex;
use std::sync::LazyLock;

use crate::report::SanitizationReport;

/// Secret type prefixes for Aho-Corasick fast filtering.
static SECRET_PREFIXES: &[&str] = &[
    "sk-ant-",    // Anthropic
    "sk-",        // OpenAI
    "AKIA",       // AWS access key
    "ghp_",       // GitHub personal token
    "gho_",       // GitHub OAuth token
    "ghs_",       // GitHub server token
    "ghr_",       // GitHub refresh token
    "sk_live_",   // Stripe live
    "sk_test_",   // Stripe test
    "pk_live_",   // Stripe public live
    "pk_test_",   // Stripe public test
    "xoxb-",      // Slack bot token
    "xoxp-",      // Slack user token
    "-----BEGIN", // PEM key
    "Bearer ",    // Bearer token in text
];

static PREFIX_AC: LazyLock<AhoCorasick> =
    LazyLock::new(|| AhoCorasickBuilder::new().match_kind(MatchKind::LeftmostFirst).build(SECRET_PREFIXES).unwrap());

/// Confirmation regexes for each secret type, indexed by the same order as SECRET_PREFIXES.
static SECRET_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"sk-ant-[A-Za-z0-9_-]{20,}").unwrap(),              // Anthropic
        Regex::new(r"sk-[A-Za-z0-9_-]{20,}").unwrap(),                  // OpenAI
        Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),                       // AWS
        Regex::new(r"ghp_[A-Za-z0-9]{36,}").unwrap(),                   // GitHub personal
        Regex::new(r"gho_[A-Za-z0-9]{36,}").unwrap(),                   // GitHub OAuth
        Regex::new(r"ghs_[A-Za-z0-9]{36,}").unwrap(),                   // GitHub server
        Regex::new(r"ghr_[A-Za-z0-9]{36,}").unwrap(),                   // GitHub refresh
        Regex::new(r"sk_live_[A-Za-z0-9]{24,}").unwrap(),               // Stripe live
        Regex::new(r"sk_test_[A-Za-z0-9]{24,}").unwrap(),               // Stripe test
        Regex::new(r"pk_live_[A-Za-z0-9]{24,}").unwrap(),               // Stripe pub live
        Regex::new(r"pk_test_[A-Za-z0-9]{24,}").unwrap(),               // Stripe pub test
        Regex::new(r"xoxb-[0-9]+-[0-9]+-[A-Za-z0-9]+").unwrap(),        // Slack bot
        Regex::new(r"xoxp-[0-9]+-[0-9]+-[0-9]+-[A-Za-z0-9]+").unwrap(), // Slack user
        Regex::new(r"-----BEGIN [A-Z ]+ KEY-----").unwrap(),            // PEM
        Regex::new(r"Bearer [A-Za-z0-9._\-]{20,}").unwrap(),            // Bearer
    ]
});

/// High-entropy hex string detector (40+ hex chars, like SHA tokens).
static HEX_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[0-9a-fA-F]{40,}\b").unwrap());

/// Check whether the byte position `pos` falls inside a URL by scanning backwards
/// for a protocol scheme (`://`) without hitting whitespace first.
fn appears_inside_url(text: &str, pos: usize) -> bool {
    let lookback = &text[pos.saturating_sub(256)..pos];
    // Walk backwards: if we hit `://` before any whitespace, the hex token is part of a URL.
    let mut chars = lookback.chars().rev().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_whitespace() || c == '"' || c == '\'' {
            return false;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            if chars.peek() == Some(&':') {
                return true;
            }
        }
    }
    false
}

/// Detect and redact leaked secrets/credentials in text.
pub fn detect_and_redact(input: &str, report: &mut SanitizationReport) -> String {
    let mut output = input.to_string();

    // Phase 1: Aho-Corasick prefix scan to identify candidate regions,
    // then confirm with corresponding regex
    let prefix_hits: Vec<_> = PREFIX_AC.find_iter(&output).collect();
    if !prefix_hits.is_empty() {
        // Collect all regex matches from the confirmation regexes for the matched prefix types
        let mut redactions: Vec<(usize, usize, &str)> = Vec::new();

        for hit in &prefix_hits {
            let pattern_idx = hit.pattern().as_usize();
            if let Some(re) = SECRET_REGEXES.get(pattern_idx) {
                // Search from the start of the prefix match
                let search_start = hit.start();
                if let Some(m) = re.find(&output[search_start..]) {
                    let abs_start = search_start + m.start();
                    let abs_end = search_start + m.end();
                    let label = match pattern_idx {
                        0 | 1 => "[REDACTED:api_key]",
                        2 => "[REDACTED:aws_key]",
                        3..=6 => "[REDACTED:github_token]",
                        7..=10 => "[REDACTED:stripe_key]",
                        11 | 12 => "[REDACTED:slack_token]",
                        13 => "[REDACTED:private_key]",
                        14 => "[REDACTED:bearer_token]",
                        _ => "[REDACTED:secret]",
                    };
                    redactions.push((abs_start, abs_end, label));
                }
            }
        }

        // Deduplicate overlapping ranges, sort by start desc for safe replacement
        redactions.sort_by(|a, b| b.0.cmp(&a.0));
        redactions.dedup_by(|a, b| a.0 >= b.0 && a.0 < b.1);

        for (start, end, label) in &redactions {
            let matched = &output[*start..*end];
            let redacted_preview =
                if matched.len() > 8 { format!("{}...{}", &matched[..4], &matched[matched.len() - 4..]) } else { "***".to_string() };
            report.details.push(crate::report::FindingDetail {
                kind: "secret_leak",
                matched: format!("{label} ({redacted_preview})"),
                context: crate::report::excerpt(&output, *start, *end, 30),
                byte_offset: *start,
            });
            output.replace_range(*start..*end, label);
            report.secrets_redacted += 1;
        }
    }

    // Phase 2: High-entropy hex tokens (skip those embedded in URLs, which are
    // common in search results and not actual secrets)
    let hex_matches: Vec<_> = HEX_TOKEN_RE.find_iter(&output).map(|m| (m.start(), m.end())).collect();
    for (start, end) in hex_matches.iter().rev() {
        if appears_inside_url(&output, *start) {
            continue;
        }
        let matched = &output[*start..*end];
        let redacted_preview = format!("{}...{}", &matched[..4], &matched[matched.len() - 4..]);
        report.details.push(crate::report::FindingDetail {
            kind: "hex_token",
            matched: redacted_preview,
            context: crate::report::excerpt(&output, *start, *end, 30),
            byte_offset: *start,
        });
        output.replace_range(*start..*end, "[REDACTED:hex_token]");
        report.secrets_redacted += 1;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_openai_key() {
        let mut report = SanitizationReport::default();
        let input = "my key is sk-abc123456789012345678901234567890123";
        let result = detect_and_redact(input, &mut report);
        assert!(result.contains("[REDACTED:api_key]"));
        assert!(!result.contains("sk-abc"));
        assert_eq!(report.secrets_redacted, 1);
    }

    #[test]
    fn redacts_anthropic_key() {
        let mut report = SanitizationReport::default();
        let input = "key: sk-ant-abcdef1234567890abcdef12";
        let result = detect_and_redact(input, &mut report);
        assert!(result.contains("[REDACTED:api_key]"));
        assert_eq!(report.secrets_redacted, 1);
    }

    #[test]
    fn redacts_aws_key() {
        let mut report = SanitizationReport::default();
        let input = "aws_access_key=AKIAIOSFODNN7EXAMPLE";
        let result = detect_and_redact(input, &mut report);
        assert!(result.contains("[REDACTED:aws_key]"));
    }

    #[test]
    fn redacts_github_token() {
        let mut report = SanitizationReport::default();
        let token = format!("ghp_{}", "a".repeat(40));
        let input = format!("token: {token}");
        let result = detect_and_redact(&input, &mut report);
        assert!(result.contains("[REDACTED:github_token]"));
    }

    #[test]
    fn redacts_pem_header() {
        let mut report = SanitizationReport::default();
        let input = "found -----BEGIN RSA PRIVATE KEY----- data";
        let result = detect_and_redact(input, &mut report);
        assert!(result.contains("[REDACTED:private_key]"));
    }

    #[test]
    fn redacts_hex_token() {
        let mut report = SanitizationReport::default();
        let hex = "a".repeat(40);
        let input = format!("commit {hex} is the latest");
        let result = detect_and_redact(&input, &mut report);
        assert!(result.contains("[REDACTED:hex_token]"));
    }

    #[test]
    fn passes_clean_text() {
        let mut report = SanitizationReport::default();
        let result = detect_and_redact("No secrets here, just normal text.", &mut report);
        assert_eq!(result, "No secrets here, just normal text.");
        assert_eq!(report.secrets_redacted, 0);
    }

    #[test]
    fn redacts_bearer_token() {
        let mut report = SanitizationReport::default();
        let input = "Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig";
        let result = detect_and_redact(input, &mut report);
        assert!(result.contains("[REDACTED:bearer_token]"));
    }

    #[test]
    fn skips_hex_in_url() {
        let mut report = SanitizationReport::default();
        let hex = "a1b2c3d4e5f6".repeat(4); // 48 hex chars
        let input = format!("found https://example.com/page/{hex}?q=1 in results");
        let result = detect_and_redact(&input, &mut report);
        assert!(result.contains(&hex), "hex token inside a URL should not be redacted");
        assert_eq!(report.secrets_redacted, 0);
    }

    #[test]
    fn redacts_standalone_hex_not_in_url() {
        let mut report = SanitizationReport::default();
        let hex = "a".repeat(40);
        let input = format!("the secret is {hex} here");
        let result = detect_and_redact(&input, &mut report);
        assert!(result.contains("[REDACTED:hex_token]"));
        assert_eq!(report.secrets_redacted, 1);
    }
}
