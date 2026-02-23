/// Extract a context window around a byte offset, clamped to the input bounds.
/// Returns up to `radius` chars either side of the matched span.
pub fn excerpt(input: &str, start: usize, end: usize, radius: usize) -> String {
    let ctx_start = input.floor_char_boundary(start.saturating_sub(radius));
    let ctx_end = input.ceil_char_boundary((end + radius).min(input.len()));
    input[ctx_start..ctx_end].replace('\n', "\\n")
}

/// Detail of a single finding for debug-level logging.
#[derive(Debug)]
pub struct FindingDetail {
    pub kind: &'static str,
    pub matched: String,
    pub context: String,
    pub byte_offset: usize,
}

/// A structured report of what the safety pipeline did to a piece of content.
#[derive(Debug, Default)]
pub struct SanitizationReport {
    pub original_len: usize,
    pub final_len: usize,
    pub truncated: bool,
    pub null_bytes_removed: usize,
    pub injections_found: Vec<String>,
    pub secrets_redacted: usize,
    pub policy_violations: Vec<String>,
    pub blocked: bool,
    pub details: Vec<FindingDetail>,
    /// Low-confidence heuristic matches (e.g. long alphanumeric runs that look like base64
    /// but decode to benign content). These are logged at DEBUG, not INFO, and do not
    /// contribute to `had_findings()`.
    pub heuristic_detections: Vec<String>,
}

impl SanitizationReport {
    pub fn had_findings(&self) -> bool {
        self.truncated
            || self.null_bytes_removed > 0
            || !self.injections_found.is_empty()
            || self.secrets_redacted > 0
            || !self.policy_violations.is_empty()
            || self.blocked
    }

    pub fn log(&self, server: &str, tool: &str) {
        if !self.had_findings() {
            return;
        }
        tracing::warn!(
            server,
            tool,
            original_len = self.original_len,
            final_len = self.final_len,
            truncated = self.truncated,
            null_bytes = self.null_bytes_removed,
            injections = self.injections_found.len(),
            secrets = self.secrets_redacted,
            violations = self.policy_violations.len(),
            blocked = self.blocked,
            "Safety pipeline findings"
        );
        for inj in &self.injections_found {
            tracing::info!(server, tool, "Injection pattern detected: {inj}");
        }
        for v in &self.policy_violations {
            tracing::info!(server, tool, "Policy violation: {v}");
        }
        for detail in &self.details {
            tracing::debug!(
                server,
                tool,
                kind = detail.kind,
                byte_offset = detail.byte_offset,
                matched = detail.matched,
                "Safety finding: ...{}...",
                detail.context
            );
        }
        for h in &self.heuristic_detections {
            tracing::debug!(server, tool, "Heuristic (not redacted): {h}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_has_no_findings() {
        let report = SanitizationReport::default();
        assert!(!report.had_findings());
    }

    #[test]
    fn report_with_injection_has_findings() {
        let report = SanitizationReport { injections_found: vec!["test".into()], ..Default::default() };
        assert!(report.had_findings());
    }
}
