//! Environment variable interpolation for config files.
//!
//! `interpolate_env_vars` replaces `${VAR_NAME}` tokens with values from the host environment.
//! Called by `CorreConfig::load` so that secrets never need to be written to disk.

use regex::Regex;
use std::sync::LazyLock;

static ENV_VAR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\\?\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap());

/// Interpolate `${VAR_NAME}` references in a string with values from the host
/// environment. Missing variables resolve to the empty string. A leading
/// backslash (`\${VAR}`) produces the literal `${VAR}` (escape removed).
pub fn interpolate_env_vars(input: &str) -> String {
    ENV_VAR_RE
        .replace_all(input, |caps: &regex::Captures| {
            let full = caps.get(0).unwrap().as_str();
            if full.starts_with('\\') {
                // Escaped: produce literal ${NAME}
                full[1..].to_string()
            } else {
                let name = &caps[1];
                std::env::var(name).unwrap_or_default()
            }
        })
        .into_owned()
}

/// Resolve a config value holding a secret that may contain an env-var reference.
///
/// If `value` matches the pattern `${VAR_NAME}`, the env var is looked up and
/// its value returned. Otherwise the string is returned as-is, allowing literal
/// API keys or other values. Thin wrapper over [`crate::config::resolve_env_ref`];
/// use this name for genuine secrets (API keys, tokens) and `resolve_env_ref` for
/// non-secret config such as endpoint URLs.
pub fn resolve_value(value: &str) -> anyhow::Result<String> {
    crate::config::resolve_env_ref(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_replaces_env_vars() {
        unsafe { std::env::set_var("TEST_INTERP_A", "hello") };
        let result = interpolate_env_vars("value = \"${TEST_INTERP_A}\"");
        assert_eq!(result, "value = \"hello\"");
        unsafe { std::env::remove_var("TEST_INTERP_A") };
    }

    #[test]
    fn interpolate_escaped_dollar_produces_literal() {
        let result = interpolate_env_vars(r"value = \${NOT_REPLACED}");
        assert_eq!(result, "value = ${NOT_REPLACED}");
    }

    #[test]
    fn resolve_value_env_ref() {
        unsafe { std::env::set_var("TEST_RESOLVE_A", "secret123") };
        assert_eq!(resolve_value("${TEST_RESOLVE_A}").unwrap(), "secret123");
        unsafe { std::env::remove_var("TEST_RESOLVE_A") };
    }

    #[test]
    fn resolve_value_literal() {
        assert_eq!(resolve_value("sk-literal-key").unwrap(), "sk-literal-key");
    }

    #[test]
    fn resolve_value_missing_env() {
        assert!(resolve_value("${DEFINITELY_NOT_SET_XYZ_999}").is_err());
    }

    #[test]
    fn interpolate_missing_var_becomes_empty() {
        let result = interpolate_env_vars("key = \"${DEFINITELY_NOT_SET_XYZ_123}\"");
        assert_eq!(result, "key = \"\"");
    }
}
