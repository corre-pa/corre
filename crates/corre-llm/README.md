# corre-llm

The LLM provider layer for Corre. Contains the single concrete implementation of the
`LlmProvider` trait -- `OpenAiCompatProvider` -- which sends chat completion requests to any
OpenAI-compatible HTTP API.

## Role in the Corre project

The `LlmProvider` trait is defined in `corre-core` so that capabilities and the safety middleware
can depend on the abstraction without pulling in HTTP code. `corre-llm` provides the one
concrete implementation used in production.

## Supported providers

Any service that speaks the OpenAI `/chat/completions` wire format works out of the box:
Venice.ai, OpenAI, Ollama, LM Studio, and any other OpenAI-compatible API.

## Key types

### `OpenAiCompatProvider`

Constructed from a parsed config via `from_config`. Resolves `config.api_key` at runtime --
if it matches `${VAR}` the env var is read, otherwise the literal value is used.

### Wire-format types (`types.rs`)

Crate-private structs (`ApiRequest`, `ApiResponse`, `ApiChoice`, `Usage`) that map directly to
the OpenAI JSON request and response bodies.

## Configuration

```toml
[llm]
provider    = "openai-compatible"
base_url    = "https://api.venice.ai/api/v1"
model       = "zai-org-glm-4.7-flash"
api_key     = "${VENICE_API_KEY}"
temperature = 0.3
```

Per-capability overrides are supported via `[capabilities.llm]` in `corre.toml`.
