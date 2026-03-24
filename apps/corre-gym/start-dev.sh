#!/usr/bin/env bash
#
# Start the corre-gym voice pipeline: whisper (STT) + piper (TTS) containers,
# then run the corre-gym bot locally via cargo.
#
# Usage:
#   ./start.sh                  # default config
#   ./start.sh -c /path/to/corre.toml
#
# The script overrides stt_url/tts_url to point at localhost (the containers
# expose ports to the host). On exit, it stops both containers.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

WHISPER_PORT=5005
PIPER_PORT=5000
WHISPER_CONTAINER=corre-whisper-dev
PIPER_CONTAINER=corre-piper-dev

# ── Helpers ──────────────────────────────────────────────────────────────────

cleanup() {
    echo ""
    echo "Stopping containers..."
    docker stop "$WHISPER_CONTAINER" "$PIPER_CONTAINER" 2>/dev/null || true
    echo "Done."
}
trap cleanup EXIT

wait_healthy() {
    local name="$1" url="$2" timeout="${3:-30}"
    echo -n "  Waiting for $name"
    for i in $(seq 1 "$timeout"); do
        if curl -sf "$url" >/dev/null 2>&1; then
            echo " ready (${i}s)"
            return 0
        fi
        echo -n "."
        sleep 1
    done
    echo " FAILED after ${timeout}s"
    echo "  Logs:"
    docker logs --tail 20 "$name" 2>&1 | sed 's/^/    /'
    return 1
}

# ── Build containers if needed ───────────────────────────────────────────────

echo "Building voice containers..."
(cd "$REPO_ROOT" && docker compose --profile voice build whisper piper)

# ── Start containers ─────────────────────────────────────────────────────────

# Stop any previous instances
docker stop "$WHISPER_CONTAINER" "$PIPER_CONTAINER" 2>/dev/null || true
docker rm "$WHISPER_CONTAINER" "$PIPER_CONTAINER" 2>/dev/null || true

echo ""
echo "Starting whisper on localhost:${WHISPER_PORT}..."
docker run --rm -d \
    --name "$WHISPER_CONTAINER" \
    -p "${WHISPER_PORT}:${WHISPER_PORT}" \
    corre-whisper

echo "Starting piper on localhost:${PIPER_PORT}..."
docker run --rm -d \
    --name "$PIPER_CONTAINER" \
    -p "${PIPER_PORT}:${PIPER_PORT}" \
    corre-piper

# ── Wait for healthy ─────────────────────────────────────────────────────────

echo ""
wait_healthy "$WHISPER_CONTAINER" "http://localhost:${WHISPER_PORT}/health" 30
wait_healthy "$PIPER_CONTAINER" "http://localhost:${PIPER_PORT}/health" 15

# ── Run corre-gym ────────────────────────────────────────────────────────────

echo ""
echo "Starting corre-gym..."
echo "  STT: http://localhost:${WHISPER_PORT}"
echo "  TTS: http://localhost:${PIPER_PORT}"
echo ""

# Override voice URLs to point at localhost (not Docker network hostnames)
export CORRE_GYM_STT_URL="http://localhost:${WHISPER_PORT}"
export CORRE_GYM_TTS_URL="http://localhost:${PIPER_PORT}"

cargo run -p corre-gym --release -- "$@"
