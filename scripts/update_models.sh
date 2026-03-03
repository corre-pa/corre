OUTPUT_FILE="venice-models.jsonl"
API_URL="${VENICE_MODELS_ENDPOINT:-https://api.venice.ai/api/v1/models}"

TMP_FILE="$(mktemp)"
trap 'rm -f "$TMP_FILE"' EXIT
curl -fsSL "$API_URL" \
 | jq '.data |= sort_by(.model_spec.pricing.input.usd + 0) | .data[] | {
 id: .id,
 description: .model_spec.description,
 price: .model_spec.pricing.output.usd,
 context: .model_spec.availableContextTokens,
 max_output: .model_spec.maxCompletionTokens
 }' > "$TMP_FILE"

mv "$TMP_FILE" "$OUTPUT_FILE"
