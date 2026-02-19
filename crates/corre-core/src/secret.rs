use std::collections::HashMap;

/// Resolves environment variable references in a config env map.
///
/// Every value is treated as an environment variable name and looked up from the
/// host environment. This matches the same pattern used for `api_key_env` in the
/// LLM config -- you never put actual secrets in `corre.toml`, only the names of
/// the env vars that hold them.
///
/// ```toml
/// [mcp.servers.brave-search]
/// env = { BRAVE_API_KEY = "BRAVE_API_KEY" }
///        ^^^^^^^^^^^^^^^   ^^^^^^^^^^^^^^^
///        passed to child   looked up from host env
/// ```
pub fn resolve_env_vars(env_map: &HashMap<String, String>) -> HashMap<String, String> {
    env_map
        .iter()
        .map(|(key, var_name)| {
            let resolved = std::env::var(var_name).unwrap_or_default();
            (key.clone(), resolved)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_env_var_by_name() {
        unsafe { std::env::set_var("TEST_CORRE_KEY", "secret123") };
        let mut env = HashMap::new();
        env.insert("API_KEY".into(), "TEST_CORRE_KEY".into());

        let resolved = resolve_env_vars(&env);
        assert_eq!(resolved["API_KEY"], "secret123");
        unsafe { std::env::remove_var("TEST_CORRE_KEY") };
    }

    #[test]
    fn missing_env_var_resolves_to_empty() {
        let mut env = HashMap::new();
        env.insert("MISSING".into(), "DEFINITELY_NOT_SET_XYZ".into());

        let resolved = resolve_env_vars(&env);
        assert_eq!(resolved["MISSING"], "");
    }
}
