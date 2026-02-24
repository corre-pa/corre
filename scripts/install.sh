#!/usr/bin/env bash
# Corre installer — run with: curl -fsSL <url>/install.sh | sh
set -euo pipefail

CORRE_DIR="${CORRE_DIR:-$HOME/.local/share/corre}"
COMPOSE_URL="${COMPOSE_URL:-}"

# Resolve the directory containing the default config files shipped alongside
# this script.  Works whether the script is invoked directly or via a symlink.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULTS_DIR="${DEFAULTS_DIR:-${SCRIPT_DIR}/install/defaults}"

# ── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'

info()  { printf "${CYAN}▸${NC} %s\n" "$1"; }
ok()    { printf "${GREEN}✓${NC} %s\n" "$1"; }
warn()  { printf "${YELLOW}!${NC} %s\n" "$1"; }
fail()  { printf "${RED}✗${NC} %s\n" "$1" >&2; exit 1; }

# ── Prerequisites ────────────────────────────────────────────────────────────
check_prereqs() {
    info "Checking prerequisites..."

    command -v docker >/dev/null 2>&1 || fail "Docker is not installed. See https://docs.docker.com/get-docker/"
    ok "Docker found"

    if docker compose version >/dev/null 2>&1; then
        ok "Docker Compose v2 found"
    else
        fail "Docker Compose v2 not found. Update Docker or install the compose plugin."
    fi

    command -v curl >/dev/null 2>&1 || fail "curl is required"
    ok "curl found"

    [ -d "${DEFAULTS_DIR}" ] || fail "Defaults directory not found at ${DEFAULTS_DIR}"
}

# ── Directory structure ──────────────────────────────────────────────────────
create_dirs() {
    info "Creating data directory at ${CORRE_DIR}"
    mkdir -p "${CORRE_DIR}/config/mcp"
    mkdir -p "${CORRE_DIR}/editions"
    mkdir -p "${CORRE_DIR}/plugins"
    ok "Directory structure created"
}

# ── API keys ─────────────────────────────────────────────────────────────────
prompt_api_keys() {
    info "Configuring API keys"
    local env_file="${CORRE_DIR}/.env"
    local changed=false

    # Create the file if it doesn't exist
    if [ ! -f "${env_file}" ]; then
        touch "${env_file}"
        chmod 600 "${env_file}"
    fi

    # Check each required key; prompt only for missing ones
    if ! grep -q '^VENICE_API_KEY=' "${env_file}" 2>/dev/null; then
        printf "LLM API key (Venice.ai, OpenAI, or compatible): "
        read -r llm_key
        case "${llm_key}" in
            VENICE-INFERENCE-KEY*) ;; # Venice.ai
            sk-proj-*|sk-*)           ;; # OpenAI
            sk-ant-*)                 ;; # Anthropic
            AIza*)                    ;; # Google Gemini
            gsk_*)                    ;; # Groq
            *) warn "Key prefix not recognised. Known prefixes: VENICE-INFERENCE-KEY (Venice), sk-proj- (OpenAI), sk-ant- (Anthropic), AIza (Gemini), gsk_ (Groq)." ;;
        esac
        echo "VENICE_API_KEY=${llm_key}" >> "${env_file}"
        changed=true
    else
        ok "VENICE_API_KEY already set in ${env_file}"
    fi

    if ! grep -q '^BRAVE_API_KEY=' "${env_file}" 2>/dev/null; then
        printf "Brave Search API key (for daily brief): "
        read -r brave_key
        case "${brave_key}" in
            BSA*) ;;
            *) warn "Brave keys typically start with 'BSA'. Double-check this is correct." ;;
        esac
        echo "BRAVE_API_KEY=${brave_key}" >> "${env_file}"
        changed=true
    else
        ok "BRAVE_API_KEY already set in ${env_file}"
    fi

    chmod 600 "${env_file}"
    if [ "${changed}" = true ]; then
        ok "API keys written to ${env_file} (mode 600)"
    else
        ok "All required API keys already present"
    fi
}

# ── Default config ───────────────────────────────────────────────────────────
write_default_config() {
    local config_file="${CORRE_DIR}/corre.toml"
    if [ -f "${config_file}" ]; then
        warn "Config already exists at ${config_file}, skipping"
        return
    fi

    info "Writing default config"

    printf "LLM base URL [https://api.venice.ai/api/v1]: "
    read -r base_url
    base_url="${base_url:-https://api.venice.ai/api/v1}"

    printf "LLM model [zai-org-glm-4.7-flash]: "
    read -r model
    model="${model:-zai-org-glm-4.7-flash}"

    # Copy template and substitute placeholders
    sed -e "s|__BASE_URL__|${base_url}|g" \
        -e "s|__MODEL__|${model}|g" \
        "${DEFAULTS_DIR}/corre.toml" > "${config_file}"

    ok "Config written to ${config_file}"
}

# ── Default topics ───────────────────────────────────────────────────────────
write_default_topics() {
    local topics_file="${CORRE_DIR}/config/topics.yml"
    if [ -f "${topics_file}" ]; then
        warn "Topics file already exists, skipping"
        return
    fi

    cp "${DEFAULTS_DIR}/topics.yml" "${topics_file}"
    ok "Default topics written to ${topics_file}"
}

# ── Docker Compose file ─────────────────────────────────────────────────────
write_compose_file() {
    local compose_file="${CORRE_DIR}/docker-compose.yml"

    if [ -n "${COMPOSE_URL}" ]; then
        info "Downloading docker-compose.yml"
        curl -fsSL "${COMPOSE_URL}" -o "${compose_file}"
    else
        info "Writing docker-compose.yml"
        cp "${DEFAULTS_DIR}/docker-compose.yml" "${compose_file}"
    fi

    ok "docker-compose.yml written"
}

# ── Optional Tailscale ───────────────────────────────────────────────────────
prompt_tailscale() {
    printf "\nEnable Tailscale for remote access? [y/N]: "
    read -r ts_enable
    if [ "${ts_enable}" != "y" ] && [ "${ts_enable}" != "Y" ]; then
        return
    fi

    printf "Do you already have a Tailscale auth key? [y/N]: "
    read -r has_key
    if [ "${has_key}" != "y" ] && [ "${has_key}" != "Y" ]; then
        echo ""
        info "How to get a Tailscale auth key:"
        echo ""
        echo "  1. Sign up or log in at https://login.tailscale.com"
        echo "     (or your Headscale instance's admin panel)"
        echo ""
        echo "  2. Skip the walkthrough by clicking I'm already familiar with Tailscale"
        echo ""
        echo "  3. Go to Settings > Keys > Generate auth key"
        echo "     https://login.tailscale.com/admin/settings/keys"
        echo ""
        echo "  4. Enable these options when generating the key:"
        echo "     - Reusable    — so the container can reconnect after restarts"
        echo "     - Ephemeral   — so the node is auto-removed if the container stops"
        echo "     - Pre-approved — so the node joins without manual approval"
        echo "       (only visible if device approval is enabled for your tailnet;"
        echo "        if you don't see it, all devices are auto-approved already)"
        echo ""
        echo "  5. If you use ACLs, make sure the tagged node (or your user) is"
        echo "     allowed to accept connections on ports 5510 and 5500. Example ACL:"
        echo ""
        echo "       { \"action\": \"accept\","
        echo "         \"src\": [\"autogroup:member\"],"
        echo "         \"dst\": [\"tag:corre:5510\", \"tag:corre:5500\"] }"
        echo ""
        echo "  6. Copy the generated key (starts with 'tskey-auth-')."
        echo ""
        printf "Press Enter when you have your auth key ready..."
        read -r _
    fi

    printf "Tailscale auth key: "
    read -r ts_key
    case "${ts_key}" in
        tskey-auth-*) ;;
        *) fail "Auth key should start with 'tskey-auth-'. Got: ${ts_key:0:20}..." ;;
    esac
    {
        echo "TAILSCALE_ENABLED=true"
        echo "TAILSCALE_AUTHKEY=${ts_key}"
    } >> "${CORRE_DIR}/.env"

    printf "Headscale login server URL (leave empty for official Tailscale): "
    read -r ts_server
    if [ -n "${ts_server}" ]; then
        echo "TAILSCALE_LOGIN_SERVER=${ts_server}" >> "${CORRE_DIR}/.env"
    fi

    ok "Tailscale configured"
}

# ── Launch ───────────────────────────────────────────────────────────────────
launch() {
    info "Starting Corre..."
    cd "${CORRE_DIR}"
    docker compose up -d

    info "Waiting for services to become healthy..."
    local attempts=0
    while [ $attempts -lt 30 ]; do
        if docker compose ps --format json 2>/dev/null | grep -q '"Health":"healthy"'; then
            break
        fi
        sleep 2
        attempts=$((attempts + 1))
    done

    echo ""
    ok "Corre is running!"
    echo ""
    info "Newspaper:  http://localhost:5510"
    info "Dashboard:  http://localhost:5500"
    echo ""
    info "Logs:       cd ${CORRE_DIR} && docker compose logs -f"
    info "Stop:       cd ${CORRE_DIR} && docker compose down"
}

# ── Main ─────────────────────────────────────────────────────────────────────
main() {
    echo ""
    printf "${CYAN}╔══════════════════════════════════╗${NC}\n"
    printf "${CYAN}║     Corre — AI Task Scheduler    ║${NC}\n"
    printf "${CYAN}╚══════════════════════════════════╝${NC}\n"
    echo ""

    check_prereqs
    create_dirs
    prompt_api_keys
    write_default_config
    write_default_topics
    write_compose_file
    prompt_tailscale
    launch
}

main
