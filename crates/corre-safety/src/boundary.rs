/// Wrap tool output in XML delimiters to establish clear boundaries for the LLM.
///
/// This prevents the LLM from confusing tool output with system instructions.
pub fn wrap_tool_output(server: &str, tool: &str, content: &str, sanitized: bool) -> String {
    let server_escaped = xml_escape_attr(server);
    let tool_escaped = xml_escape_attr(tool);
    format!("<tool_output server=\"{server_escaped}\" tool=\"{tool_escaped}\" sanitized=\"{sanitized}\">\n{content}\n</tool_output>")
}

fn xml_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_simple_output() {
        let result = wrap_tool_output("brave-search", "web_search", "some results", false);
        assert!(result.starts_with("<tool_output"));
        assert!(result.contains("server=\"brave-search\""));
        assert!(result.contains("tool=\"web_search\""));
        assert!(result.contains("sanitized=\"false\""));
        assert!(result.contains("some results"));
        assert!(result.ends_with("</tool_output>"));
    }

    #[test]
    fn escapes_special_chars_in_attrs() {
        let result = wrap_tool_output("server<>\"name", "tool&'test", "content", true);
        assert!(result.contains("server=\"server&lt;&gt;&quot;name\""));
        assert!(result.contains("tool=\"tool&amp;&apos;test\""));
    }

    #[test]
    fn sanitized_flag_true() {
        let result = wrap_tool_output("s", "t", "c", true);
        assert!(result.contains("sanitized=\"true\""));
    }
}
