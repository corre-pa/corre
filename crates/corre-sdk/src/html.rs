//! HTML sanitization helpers for app plugin output.
//!
//! [`sanitize_html`] allows only basic inline formatting tags and is used when rendering
//! article bodies in the newspaper template. [`sanitize_custom_html`] permits a wider set of
//! structural and form elements for plugins that use `content_type = "custom"`. Both strip
//! scripts, event handlers, and dangerous URL schemes via `ammonia`.

use ammonia::Builder;
use std::collections::HashSet;

/// Sanitize HTML content, allowing only safe formatting tags.
/// Strips scripts, event handlers, iframes, and other dangerous elements.
pub fn sanitize_html(input: &str) -> String {
    Builder::new()
        .tags(HashSet::from(["p", "br", "b", "strong", "i", "em", "a", "ul", "ol", "li", "blockquote"]))
        .link_rel(Some("noopener"))
        .url_schemes(HashSet::from(["http", "https"]))
        .clean(input)
        .to_string()
}

/// Sanitize custom plugin HTML with a wider allowlist (for `content_type = "custom"`).
/// Allows structural elements like div, form, input, table, img, and CSS classes.
pub fn sanitize_custom_html(input: &str) -> String {
    Builder::new()
        .tags(HashSet::from([
            "p",
            "br",
            "b",
            "strong",
            "i",
            "em",
            "a",
            "ul",
            "ol",
            "li",
            "blockquote",
            "div",
            "span",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "form",
            "input",
            "label",
            "button",
            "select",
            "option",
            "textarea",
            "table",
            "thead",
            "tbody",
            "tfoot",
            "tr",
            "th",
            "td",
            "img",
            "figure",
            "figcaption",
            "pre",
            "code",
            "details",
            "summary",
            "section",
            "article",
            "nav",
            "header",
            "footer",
            "hr",
        ]))
        .generic_attributes(HashSet::from(["class", "id", "style", "type", "name", "value", "placeholder", "for"]))
        .link_rel(Some("noopener"))
        .url_schemes(HashSet::from(["http", "https"]))
        .clean(input)
        .to_string()
}

/// Validate a URL, returning `"#"` for anything that isn't http or https.
pub fn sanitize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") { trimmed.to_string() } else { "#".to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_html_strips_scripts_preserves_formatting() {
        let dangerous = r#"<p>Hello</p><script>alert('xss')</script><b>bold</b>"#;
        let result = sanitize_html(dangerous);
        assert!(result.contains("<p>Hello</p>"));
        assert!(result.contains("<b>bold</b>"));
        assert!(!result.contains("<script>"));
    }

    #[test]
    fn sanitize_custom_html_allows_divs_and_forms() {
        let input = r#"<div class="quiz"><form><input type="text" name="answer"><button>Submit</button></form></div>"#;
        let result = sanitize_custom_html(input);
        assert!(result.contains("<div"));
        assert!(result.contains("<form>"));
        assert!(result.contains("<input"));
        assert!(result.contains("<button>"));
    }

    #[test]
    fn sanitize_custom_html_strips_scripts() {
        let input = r#"<div>safe</div><script>alert('xss')</script>"#;
        let result = sanitize_custom_html(input);
        assert!(result.contains("<div>safe</div>"));
        assert!(!result.contains("<script>"));
    }

    #[test]
    fn sanitize_url_allows_http_and_https() {
        assert_eq!(sanitize_url("https://example.com"), "https://example.com");
        assert_eq!(sanitize_url("http://example.com"), "http://example.com");
    }

    #[test]
    fn sanitize_url_rejects_javascript_protocol() {
        assert_eq!(sanitize_url("javascript:alert(1)"), "#");
        assert_eq!(sanitize_url("data:text/html,<script>"), "#");
        assert_eq!(sanitize_url(""), "#");
    }
}
