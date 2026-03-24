#!/usr/bin/env bash
#
# Start the corre-gym voice pipeline: whisper (STT) + piper (TTS) containers,
# then run the corre-gym bot locally via cargo.
#
# Usage:
#   ./start.sh                        # uses default config path
#   ./start.sh -c /path/to/corre.toml
#
# Ensure your corre.toml [gym.voice] section points at localhost:
#   stt_url = "http://localhost:5005"
#   tts_url = "http://localhost:5000"
#
# On exit (Ctrl-C), both containers are stopped automatically.
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

echo "Building voice containers (if needed)..."
(cd "$REPO_ROOT" && docker compose --profile voice build whisper piper)

# ── Start containers ─────────────────────────────────────────────────────────

# Stop any previous dev instances
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
echo "Voice pipeline ready:"
echo "  STT: http://localhost:${WHISPER_PORT}"
echo "  TTS: http://localhost:${PIPER_PORT}"
echo ""

exec cargo run -p corre-gym --release -- "$@"
