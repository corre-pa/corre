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
