use crate::report::SanitizationReport;

const MAX_IDENTICAL_RUN: usize = 500;

/// Structural validation: length limits, null bytes, whitespace anomalies, character stuffing.
pub fn validate(input: &str, max_bytes: usize, report: &mut SanitizationReport) -> String {
    let mut output = input.to_string();

    // Truncate if over byte limit
    if output.len() > max_bytes {
        // Find a valid char boundary at or before max_bytes
        let truncate_at = (0..=max_bytes).rev().find(|&i| output.is_char_boundary(i)).unwrap_or(0);
        output.truncate(truncate_at);
        output.push_str(" [TRUNCATED]");
        report.truncated = true;
    }

    // Strip null bytes
    let before_len = output.len();
    output = output.replace('\0', "");
    let nulls_removed = before_len - output.len();
    if nulls_removed > 0 {
        report.null_bytes_removed = nulls_removed;
    }

    // Flag high whitespace ratio (>90%) as suspicious; collapse to single spaces
    let total_chars = output.chars().count();
    if total_chars > 100 {
        let ws_chars = output.chars().filter(|c| c.is_whitespace()).count();
        if ws_chars as f64 / total_chars as f64 > 0.9 {
            // Collapse runs of whitespace to single space
            let collapsed: String = output.split_whitespace().collect::<Vec<_>>().join(" ");
            output = collapsed;
            report.injections_found.push("high whitespace ratio (obfuscation attempt)".into());
        }
    }

    // Truncate runs of 500+ identical characters (token stuffing)
    output = truncate_identical_runs(&output);

    output
}

fn truncate_identical_runs(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        result.push(ch);
        let mut run_len = 1usize;
        while chars.peek() == Some(&ch) {
            chars.next();
            run_len += 1;
            if run_len <= MAX_IDENTICAL_RUN {
                result.push(ch);
            }
        }
        if run_len > MAX_IDENTICAL_RUN {
            result.push_str(" [TRUNCATED:repeated]");
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_clean_text() {
        let mut report = SanitizationReport::default();
        let result = validate("Hello, world!", 1000, &mut report);
        assert_eq!(result, "Hello, world!");
        assert!(!report.had_findings());
    }

    #[test]
    fn truncates_long_output() {
        let mut report = SanitizationReport::default();
        let input = "A".repeat(200);
        let result = validate(&input, 100, &mut report);
        assert!(result.len() <= 112); // 100 + " [TRUNCATED]"
        assert!(result.ends_with("[TRUNCATED]"));
        assert!(report.truncated);
    }

    #[test]
    fn strips_null_bytes() {
        let mut report = SanitizationReport::default();
        let result = validate("hello\0world\0", 1000, &mut report);
        assert_eq!(result, "helloworld");
        assert_eq!(report.null_bytes_removed, 2);
    }

    #[test]
    fn collapses_whitespace_obfuscation() {
        let mut report = SanitizationReport::default();
        // 95% whitespace
        let input = format!("cmd{}", " ".repeat(200));
        let result = validate(&input, 100_000, &mut report);
        assert!(!result.contains("   "));
        assert!(!report.injections_found.is_empty());
    }

    #[test]
    fn truncates_identical_runs() {
        let mut report = SanitizationReport::default();
        let input = "A".repeat(1000);
        let result = validate(&input, 100_000, &mut report);
        assert!(result.contains("[TRUNCATED:repeated]"));
        assert!(result.len() < 1000);
    }
}
