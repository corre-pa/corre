# Path Traversal (Uncontrolled data in path expression)

Never join user-supplied data onto a filesystem path without validation — a payload like `../../etc/passwd` escapes the intended directory
entirely.

## Writes (path may not exist yet) — validate components

```rust
fn safe_join(base: &Path, untrusted: &str) -> anyhow::Result<PathBuf> {
    let rel = Path::new(untrusted);
    anyhow::ensure!(
        rel.components().all(|c| matches!(c, Component::Normal(_))),
        "path traversal detected: {untrusted}"
    );
    Ok(base.join(rel))
}
```

Reject `..`, absolute roots, and device paths. Use in CCPP `output/write` handlers and anywhere a caller supplies a
relative output path.

## Reads (path must already exist) — canonicalize and check prefix

```rust
fn safe_read_path(base: &Path, untrusted: &str) -> anyhow::Result<PathBuf> {
    let canonical = base.join(untrusted).canonicalize()?;
    anyhow::ensure!(canonical.starts_with(base.canonicalize()?), "path traversal detected");
    Ok(canonical)
}
```

Also catches symlinks pointing outside the base. Use in HTTP handlers serving plugin static files.

### Plain identifiers (server names, app slugs, config keys) — allowlist characters

```rust
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

anyhow::ensure!(is_safe_name(name), "invalid name: {name}");
let path = mcp_dir.join(format!("{name}.toml"));
```

### Never do this

- **`path.contains("..")`** — bypassed by symlinks and URL encoding.
- **`format!("{name}.toml")` with unvalidated input** — the format itself is the vulnerability.
- **`canonicalize()` without `starts_with`** — resolves the path but never confirms it stayed in bounds.

Validate at the system boundary (HTTP handler, RPC dispatcher, config loader). Don't push validation responsibility into helpers that assume
their caller already checked.

# Cleartext Transmission of Sensitive Information

Never send secrets (API keys, tokens, passwords) over unencrypted channels or in forms that leak into logs or proxies.

## Pass secrets in headers, not URLs

Query parameters and URL path segments appear in access logs, browser history, and proxy records. Always use the
`Authorization: Bearer` header (via `.bearer_auth()`) — never embed a key in a query param or path segment.

## Never disable TLS certificate validation

```rust
// BANNED
reqwest::ClientBuilder::new().danger_accept_invalid_certs(true)
```

reqwest validates certificates against the system trust store by default — don't override it. If a self-signed cert is
genuinely needed in development, add it to the trust store; don't disable validation globally.

## Redact secrets before logging

Call `redact_secrets()` (`corre-host/src/subprocess.rs`) on any JSON value that may contain config or request params
before passing it to `tracing::debug!` or similar. Never log raw config structs that include `api_key` fields.

## Store secrets as `${VAR}` references, never as literals

Config files (`.toml`) must use `${ENV_VAR_NAME}` references resolved at runtime via `resolve_value()`. Hardcoded keys in
config files get committed to version control and appear in plaintext on disk.

## Enforce HTTPS for external endpoints

Validate that any user-supplied `base_url` uses `https://` before making requests. `http://` is only acceptable for
explicitly local services (loopback addresses only). See `corre-cli/src/setup/validate.rs` for the existing pattern.
