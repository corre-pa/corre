#!/usr/bin/env bash
set -euo pipefail

# ── Constants ────────────────────────────────────────────────────────────────

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REGISTRY_DIR="${REPO_DIR}/mcp-registry"
SITE_DIR="${REGISTRY_DIR}/site"
OUTPUT="${SITE_DIR}/registry.json"
VERSION="$(grep -m1 '^version' "${REPO_DIR}/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
TODAY="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
CROSS_TARGET_DIR="${REPO_DIR}/target-cross"

# Binary download URLs are relative paths; the installer resolves them
# against the user's configured mcp_repository base URL at install time.

BINARIES=(mcp-smtp mcp-telegram)
MCP_PACKAGES="--package mcp-smtp --package mcp-telegram"

# Target definitions: triple|platform_key|build_tool|os_required
# platform_key matches the keys the installer uses ("{platform}-{arch}")
TARGETS=(
    "x86_64-unknown-linux-gnu|linux-x86_64|cross|"
    "aarch64-unknown-linux-gnu|linux-aarch64|cross|"
    "armv7-unknown-linux-gnueabihf|linux-armv7|cross|"
    "x86_64-pc-windows-gnu|windows-x86_64|cross|"
    "aarch64-apple-darwin|darwin-aarch64|cargo|Darwin"
    "x86_64-apple-darwin|darwin-x86_64|cargo|Darwin"
)

# ── Color helpers ────────────────────────────────────────────────────────────

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info() { echo -e "${BLUE}[INFO]${NC}  $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC}  $*"; }
ok()   { echo -e "${GREEN}[OK]${NC}    $*"; }

host_os() { uname -s; }

sha256_hex() {
    if command -v sha256sum &>/dev/null; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

# ── Prepare staging directory ────────────────────────────────────────────────

rm -rf "${SITE_DIR}"
mkdir -p "${SITE_DIR}/bin"

# ── Build binaries per target ────────────────────────────────────────────────

# Associative array: CHECKSUMS["binary|platform_key"] = hex
declare -A CHECKSUMS
BUILT_KEYS=()

for target_entry in "${TARGETS[@]}"; do
    IFS='|' read -r TRIPLE PLATFORM_KEY BUILD_TOOL OS_REQUIRED <<< "${target_entry}"

    if [[ -n "${OS_REQUIRED}" && "$(host_os)" != "${OS_REQUIRED}" ]]; then
        warn "Skipping ${PLATFORM_KEY} (${TRIPLE}) — requires ${OS_REQUIRED}"
        continue
    fi

    info "Building for ${TRIPLE} (${BUILD_TOOL})..."

    if [[ "${BUILD_TOOL}" == "cargo" ]]; then
        if ! rustup target list --installed | grep -q "^${TRIPLE}$"; then
            info "Installing rustup target ${TRIPLE}..."
            rustup target add "${TRIPLE}"
        fi
        cargo build --release ${MCP_PACKAGES} --target "${TRIPLE}"
        TARGET_DIR="${REPO_DIR}/target"
    else
        CARGO_TARGET_DIR="${CROSS_TARGET_DIR}" cross build --release ${MCP_PACKAGES} --target "${TRIPLE}"
        TARGET_DIR="${CROSS_TARGET_DIR}"
    fi

    for bin in "${BINARIES[@]}"; do
        # Windows binaries have .exe extension
        if [[ "${TRIPLE}" == *"-windows-"* ]]; then
            binary_path="${TARGET_DIR}/${TRIPLE}/release/${bin}.exe"
        else
            binary_path="${TARGET_DIR}/${TRIPLE}/release/${bin}"
        fi
        if [[ ! -f "${binary_path}" ]]; then
            echo "ERROR: binary not found: ${binary_path}" >&2
            exit 1
        fi
        checksum="$(sha256_hex "${binary_path}")"
        CHECKSUMS["${bin}|${PLATFORM_KEY}"]="${checksum}"
        ok "${bin} ${PLATFORM_KEY}: ${checksum}"

        # Stage binary into site/bin/
        staged_name="${bin}-${PLATFORM_KEY}"
        cp "${binary_path}" "${SITE_DIR}/bin/${staged_name}"
        ok "Staged ${staged_name}"
    done

    BUILT_KEYS+=("${PLATFORM_KEY}")
done

if [[ ${#BUILT_KEYS[@]} -eq 0 ]]; then
    echo "ERROR: no targets were built" >&2
    exit 1
fi

# ── Helper: emit sha256 JSON object for a binary ────────────────────────────

sha256_json() {
    local bin="$1"
    local first=true
    echo -n "{"
    for key in "${BUILT_KEYS[@]}"; do
        local checksum="${CHECKSUMS["${bin}|${key}"]}"
        if [[ "${first}" == true ]]; then
            first=false
        else
            echo -n ", "
        fi
        echo -n "\"${key}\": \"${checksum}\""
    done
    echo -n "}"
}

# ── Generate JSON ────────────────────────────────────────────────────────────

info "Generating ${OUTPUT}..."

cat > "${OUTPUT}" <<MANIFEST
{
  "version": 1,
  "updated_at": "${TODAY}",
  "servers": [
    {
      "id": "brave-search",
      "name": "Brave Search",
      "description": "Web search via the Brave Search API",
      "version": "1.1.2",
      "install": {
        "method": "npx",
        "package": "@brave/brave-search-mcp-server",
        "command": "npx",
        "args": ["-y", "@brave/brave-search-mcp-server"]
      },
      "config": [
        {
          "name": "BRAVE_API_KEY",
          "description": "Brave Search API key (https://brave.com/search/api/)",
          "required": true
        }
      ],
      "tags": ["search", "web"],
      "verified": true
    },
    {
      "id": "kagi-search",
      "name": "Kagi Search",
      "description": "Search and summarization via the Kagi API",
      "version": "0.1.3",
      "install": {
        "method": "pip",
        "package": "kagimcp",
        "command": "uvx",
        "args": ["kagimcp"]
      },
      "config": [
        {
          "name": "KAGI_API_KEY",
          "description": "Kagi API key (https://www.kagi.com/account/api)",
          "required": true
        },
        {
          "name": "KAGI_SUMMARIZER_ENGINE",
          "description": "Choose which Kagi summarization engine to use: 'cecil' or 'gemini-1.5-pro'. Defaults to 'cecil' if not set.",
          "required": false
        }
      ],
      "tags": ["search", "web"],
      "verified": false
    },
    {
      "id": "smtp",
      "name": "SMTP Email",
      "description": "Send and draft emails via SMTP",
      "version": "${VERSION}",
      "install": {
        "method": "binary",
        "download_url_template": "/mcp/bin/mcp-smtp-{platform}-{arch}",
        "binary_name": "mcp-smtp",
        "sha256": $(sha256_json mcp-smtp),
        "command": "mcp-smtp",
        "args": []
      },
      "config": [
        {
          "name": "SMTP_HOST",
          "description": "SMTP server hostname",
          "required": true
        },
        {
          "name": "SMTP_PORT",
          "description": "SMTP server port (default: 587)",
          "required": false
        },
        {
          "name": "SMTP_USER",
          "description": "SMTP authentication username",
          "required": true
        },
        {
          "name": "SMTP_PASSWORD",
          "description": "SMTP authentication password",
          "required": true
        },
        {
          "name": "SMTP_FROM",
          "description": "Sender email address",
          "required": true
        }
      ],
      "tags": ["email", "messaging"],
      "verified": true
    },
    {
      "id": "telegram",
      "name": "Telegram",
      "description": "Send and draft messages via Telegram Bot API",
      "version": "${VERSION}",
      "install": {
        "method": "binary",
        "download_url_template": "/mcp/bin/mcp-telegram-{platform}-{arch}",
        "binary_name": "mcp-telegram",
        "sha256": $(sha256_json mcp-telegram),
        "command": "mcp-telegram",
        "args": []
      },
      "config": [
        {
          "name": "TELEGRAM_BOT_TOKEN",
          "description": "Telegram Bot API token from @BotFather",
          "required": true
        }
      ],
      "tags": ["messaging", "telegram"],
      "verified": true
    }
  ]
}
MANIFEST

ok "Registry written to ${OUTPUT}"

# Also copy to the legacy location for non-Docker use
cp "${OUTPUT}" "${REGISTRY_DIR}/mcp-registry.json"
ok "Copied to ${REGISTRY_DIR}/mcp-registry.json"

# ── Validate JSON ────────────────────────────────────────────────────────────

if command -v jq &>/dev/null; then
    jq . "${OUTPUT}" > /dev/null
    ok "JSON is valid"
    echo ""
    echo -e "${BOLD}Registry summary:${NC}"
    jq -r '.servers[] | "  \(.id) (v\(.version)) — \(.description)"' "${OUTPUT}"
    echo ""
    echo -e "${BOLD}Staged files:${NC}"
    ls -lh "${SITE_DIR}/bin/" 2>/dev/null || echo "  (no binaries staged)"
else
    info "jq not found, skipping validation"
fi
