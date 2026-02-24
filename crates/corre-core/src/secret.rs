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
    fn interpolate_missing_var_becomes_empty() {
        let result = interpolate_env_vars("key = \"${DEFINITELY_NOT_SET_XYZ_123}\"");
        assert_eq!(result, "key = \"\"");
    }
}
