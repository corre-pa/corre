# corre-safety

Prompt injection defense middleware for Corre. Sits between external content (MCP tool outputs)
and the LLM, neutralizing adversarial content before it can influence model behavior.

## Role in the Corre project

The safety layer wraps the `McpCaller` and `LlmProvider` traits transparently with two
decorator types -- `SafeMcpCaller` and `SafeLlmProvider` -- so no capability code requires
modification. Safety is enabled by default.

## Safety pipeline stages

Every MCP tool output passes through four sequential stages:

### Stage 1 -- Validation (`validator`)

Length truncation, null byte removal, whitespace obfuscation collapse, and token-stuffing
truncation.

### Stage 2 -- Injection sanitization (`sanitizer`)

~45 known injection phrases via Aho-Corasick, `eval()`/`exec()` detection, base64 payload
inspection, special LLM token escaping, and role marker prefixing.

### Stage 3 -- Leak detection (`leak_detector`)

Two-phase detection (Aho-Corasick prefix scan + regex confirmation) of API keys, cloud
credentials, PEM headers, tokens, and high-entropy hex strings. Also applied to LLM responses
via `SafeLlmProvider`.

### Stage 4 -- Policy evaluation (`policy`)

Rule-based engine for shell injection, SQL injection, path traversal, XSS, and encoded
exploits. User-supplied `custom_block_patterns` always escalate to `Block`.

## Configuration

```toml
[safety]
enabled = true                    # master switch
max_output_bytes = 100000         # truncation limit
sanitize_injections = true        # stages 1 and 2
detect_leaks = true               # stage 3
boundary_wrap = true              # wrap in <tool_output> XML
high_severity_action = "sanitize" # "warn" | "sanitize" | "block"
custom_block_patterns = []        # regex list; any match triggers block
```

## Modules

| Module | Purpose |
|--------|---------|
| `validator` | Stage 1: structural validation |
| `sanitizer` | Stage 2: injection detection and redaction |
| `leak_detector` | Stage 3: credential detection and redaction |
| `policy` | Stage 4: rule-based policy evaluation |
| `boundary` | XML boundary wrapping |
| `report` | `SanitizationReport` accumulator |
| `safe_mcp` | `SafeMcpCaller` decorator |
| `safe_llm` | `SafeLlmProvider` decorator |
| `config` | Re-exports `SafetyConfig` and `PolicyAction` from `corre-core` |
