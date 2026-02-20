use regex::Regex;
use std::sync::LazyLock;

use crate::config::PolicyAction;
use crate::report::SanitizationReport;

/// Severity of a policy violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// A single policy rule: a regex pattern with severity and action.
struct PolicyRule {
    name: &'static str,
    pattern: Regex,
    severity: Severity,
    action: PolicyAction,
}

/// Built-in policy rules for common attack vectors.
static BUILTIN_RULES: LazyLock<Vec<PolicyRule>> = LazyLock::new(|| {
    vec![
        PolicyRule {
            name: "shell_injection_backtick",
            pattern: Regex::new(r"`[^`]{2,}`").unwrap(),
            severity: Severity::Medium,
            action: PolicyAction::Warn,
        },
        PolicyRule {
            name: "shell_injection_subshell",
            pattern: Regex::new(r"\$\([^)]+\)").unwrap(),
            severity: Severity::High,
            action: PolicyAction::Sanitize,
        },
        PolicyRule {
            name: "sql_injection",
            pattern: Regex::new(r"(?i)(?:UNION\s+SELECT|DROP\s+TABLE|DELETE\s+FROM|INSERT\s+INTO|UPDATE\s+\w+\s+SET|;\s*--)").unwrap(),
            severity: Severity::High,
            action: PolicyAction::Sanitize,
        },
        PolicyRule {
            name: "path_traversal",
            pattern: Regex::new(r"(?:\.\./){2,}|(?:\\\.\\\.\\){2,}").unwrap(),
            severity: Severity::High,
            action: PolicyAction::Sanitize,
        },
        PolicyRule {
            name: "script_tag",
            pattern: Regex::new(r"(?i)<script[^>]*>").unwrap(),
            severity: Severity::High,
            action: PolicyAction::Sanitize,
        },
        PolicyRule {
            name: "data_uri_base64",
            pattern: Regex::new(r"data:[^;]+;base64,").unwrap(),
            severity: Severity::Medium,
            action: PolicyAction::Warn,
        },
        PolicyRule {
            name: "hex_escape_sequence",
            pattern: Regex::new(r"(?:\\x[0-9a-fA-F]{2}){8,}").unwrap(),
            severity: Severity::Medium,
            action: PolicyAction::Sanitize,
        },
    ]
});

/// Result of evaluating all policy rules against content.
pub struct PolicyResult {
    pub action: PolicyAction,
    pub violations: Vec<String>,
}

/// Evaluate content against built-in policy rules and optional custom block patterns.
pub fn evaluate(input: &str, custom_patterns: &[String], high_action: PolicyAction, report: &mut SanitizationReport) -> PolicyResult {
    let mut max_action = PolicyAction::Warn;
    let mut violations = Vec::new();

    // Check built-in rules
    for rule in BUILTIN_RULES.iter() {
        if let Some(m) = rule.pattern.find(input) {
            let effective_action = if rule.severity >= Severity::High { high_action } else { rule.action };
            violations.push(format!("{} (severity: {:?})", rule.name, rule.severity));
            report.details.push(crate::report::FindingDetail {
                kind: "policy_violation",
                matched: format!("{} [{:?}]", rule.name, rule.severity),
                context: crate::report::excerpt(input, m.start(), m.end(), 60),
                byte_offset: m.start(),
            });
            if effective_action > max_action {
                max_action = effective_action;
            }
        }
    }

    // Check custom block patterns
    for pattern_str in custom_patterns {
        if let Ok(re) = Regex::new(pattern_str) {
            if let Some(m) = re.find(input) {
                violations.push(format!("custom_pattern: {pattern_str}"));
                report.details.push(crate::report::FindingDetail {
                    kind: "custom_block_pattern",
                    matched: pattern_str.clone(),
                    context: crate::report::excerpt(input, m.start(), m.end(), 60),
                    byte_offset: m.start(),
                });
                max_action = PolicyAction::Block;
            }
        }
    }

    report.policy_violations.extend(violations.clone());

    PolicyResult { action: max_action, violations }
}

/// Apply a policy action to content. Returns the (possibly replaced) content.
pub fn apply_action(action: PolicyAction, content: &str, report: &mut SanitizationReport) -> String {
    match action {
        PolicyAction::Warn => content.to_string(),
        PolicyAction::Sanitize => {
            // The sanitizer module handles actual replacement; policy just confirms the action
            content.to_string()
        }
        PolicyAction::Block => {
            report.blocked = true;
            "[BLOCKED: content violated safety policy]".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sql_injection() {
        let mut report = SanitizationReport::default();
        let result = evaluate("some UNION SELECT * FROM users", &[], PolicyAction::Sanitize, &mut report);
        assert!(!result.violations.is_empty());
        assert!(result.violations.iter().any(|v| v.contains("sql_injection")));
    }

    #[test]
    fn detects_path_traversal() {
        let mut report = SanitizationReport::default();
        let result = evaluate("read file ../../etc/passwd", &[], PolicyAction::Sanitize, &mut report);
        assert!(result.violations.iter().any(|v| v.contains("path_traversal")));
    }

    #[test]
    fn detects_script_tag() {
        let mut report = SanitizationReport::default();
        let result = evaluate("try <script>alert(1)</script>", &[], PolicyAction::Sanitize, &mut report);
        assert!(result.violations.iter().any(|v| v.contains("script_tag")));
    }

    #[test]
    fn custom_pattern_blocks() {
        let mut report = SanitizationReport::default();
        let customs = vec!["forbidden_word".to_string()];
        let result = evaluate("this has forbidden_word inside", &customs, PolicyAction::Sanitize, &mut report);
        assert_eq!(result.action, PolicyAction::Block);
    }

    #[test]
    fn clean_text_passes() {
        let mut report = SanitizationReport::default();
        let result = evaluate("Rust 1.85 brings exciting improvements.", &[], PolicyAction::Sanitize, &mut report);
        assert!(result.violations.is_empty());
        // Warn is the minimum action even when nothing matches
        assert_eq!(result.action, PolicyAction::Warn);
    }

    #[test]
    fn block_action_replaces_content() {
        let mut report = SanitizationReport::default();
        let result = apply_action(PolicyAction::Block, "malicious content", &mut report);
        assert!(result.contains("[BLOCKED"));
        assert!(report.blocked);
    }

    #[test]
    fn high_action_propagates() {
        let mut report = SanitizationReport::default();
        let result = evaluate("DROP TABLE users;--", &[], PolicyAction::Block, &mut report);
        assert_eq!(result.action, PolicyAction::Block);
    }
}
