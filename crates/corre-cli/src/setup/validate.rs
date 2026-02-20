/// Validates that input is non-empty after trimming.
pub fn non_empty(input: &str) -> Result<(), String> {
    if input.trim().is_empty() { Err("Value cannot be empty".into()) } else { Ok(()) }
}

/// Validates a port number string.
pub fn port_number(input: &str) -> Result<(), String> {
    match input.trim().parse::<u16>() {
        Ok(p) if p > 0 => Ok(()),
        _ => Err("Must be a valid port number (1-65535)".into()),
    }
}

/// Validates an hour (0-23).
pub fn hour(input: &str) -> Result<(), String> {
    match input.trim().parse::<u8>() {
        Ok(h) if h < 24 => Ok(()),
        _ => Err("Must be an hour between 0 and 23".into()),
    }
}

/// Validates a URL-like string (basic check).
pub fn url_like(input: &str) -> Result<(), String> {
    let trimmed = input.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(())
    } else {
        Err("Must start with http:// or https://".into())
    }
}
