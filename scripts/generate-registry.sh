#!/usr/bin/env bash
set -euo pipefail

# ── Constants ────────────────────────────────────────────────────────────────

# ── Parse arguments ──────────────────────────────────────────────────────────

NO_BUILD=false
for arg in "$@"; do
    case "${arg}" in
        --no-build) NO_BUILD=true ;;
        -h|--help)
            echo "Usage: $(basename "$0") [--no-build]"
            echo ""
            echo "  --no-build  Skip compilation; use existing binaries from the target directory"
            exit 0
            ;;
        *)
            echo "Unknown option: ${arg}" >&2
            exit 1
            ;;
    esac
done

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REGISTRY_DIR="${REPO_DIR}/corre-registry"
MCP_REGISTERED_DIR="${REGISTRY_DIR}/registered"
CAP_REGISTERED_DIR="${REGISTRY_DIR}/apps/registered"
SITE_DIR="${REGISTRY_DIR}/site"
OUTPUT="${SITE_DIR}/mcp/registry.json"
VERSION="$(grep -m1 '^version' "${REPO_DIR}/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
TODAY="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
CROSS_TARGET_DIR="${REPO_DIR}/target-cross"

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

# ── Prerequisites ────────────────────────────────────────────────────────────

if ! command -v jq &>/dev/null; then
    echo "ERROR: jq is required" >&2
    exit 1
fi

# ── Discover binary MCP servers from registered definitions ──────────────────

# Servers whose install.method is "binary" need to be cross-compiled.
# install.binary_name is both the cargo package name and the staged artifact.
BINARIES=()
MCP_PACKAGES=""
for f in "${MCP_REGISTERED_DIR}"/*.json; do
    [[ -f "$f" ]] || continue
    method="$(jq -r '.install.method' "$f")"
    if [[ "${method}" == "binary" ]]; then
        bin="$(jq -r '.install.binary_name' "$f")"
        BINARIES+=("${bin}")
        MCP_PACKAGES+=" --package ${bin}"
    fi
done

# ── Discover binary apps from registered definitions ─────────────────────

CAP_BINARIES=()
CAP_PACKAGES=""
if [[ -d "${CAP_REGISTERED_DIR}" ]]; then
    for f in "${CAP_REGISTERED_DIR}"/*.json; do
        [[ -f "$f" ]] || continue
        method="$(jq -r '.install.method' "$f")"
        if [[ "${method}" == "binary" ]]; then
            bin="$(jq -r '.install.binary_name' "$f")"
            CAP_BINARIES+=("${bin}")
            CAP_PACKAGES+=" --package ${bin}"
        fi
    done
fi

# Merge all binary names for the build step
ALL_BINARIES=("${BINARIES[@]}" "${CAP_BINARIES[@]}")
ALL_PACKAGES="${MCP_PACKAGES}${CAP_PACKAGES}"

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

# ── Helpers ──────────────────────────────────────────────────────────────────

sha256_hex() {
    if command -v sha256sum &>/dev/null; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

# ── Prepare staging directory ────────────────────────────────────────────────

rm -rf "${SITE_DIR}"
mkdir -p "${SITE_DIR}/mcp/bin"
mkdir -p "${SITE_DIR}/apps/bin"

# ── Build binaries per target ────────────────────────────────────────────────

# Associative array: CHECKSUMS["binary|platform_key"] = hex
declare -A CHECKSUMS
BUILT_KEYS=()

if [[ ${#ALL_BINARIES[@]} -gt 0 ]]; then
    for target_entry in "${TARGETS[@]}"; do
        IFS='|' read -r TRIPLE PLATFORM_KEY BUILD_TOOL OS_REQUIRED <<< "${target_entry}"

        if [[ "${BUILD_TOOL}" == "cross" ]]; then
            TARGET_DIR="${CROSS_TARGET_DIR}"
        else
            TARGET_DIR="${REPO_DIR}/target"
        fi

        if [[ "${NO_BUILD}" == true ]]; then
            # In --no-build mode, check whether binaries exist for this target
            found_all=true
            for bin in "${ALL_BINARIES[@]}"; do
                if [[ "${TRIPLE}" == *"-windows-"* ]]; then
                    binary_path="${TARGET_DIR}/${TRIPLE}/release/${bin}.exe"
                else
                    binary_path="${TARGET_DIR}/${TRIPLE}/release/${bin}"
                fi
                if [[ ! -f "${binary_path}" ]]; then
                    found_all=false
                    break
                fi
            done
            if [[ "${found_all}" == false ]]; then
                warn "Skipping ${PLATFORM_KEY} (${TRIPLE}) — binary not found"
                continue
            fi
            info "Found existing binaries for ${TRIPLE}"
        else
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
                cargo build --release ${ALL_PACKAGES} --target "${TRIPLE}"
            else
                CARGO_TARGET_DIR="${CROSS_TARGET_DIR}" cross build --release ${ALL_PACKAGES} --target "${TRIPLE}"
            fi
        fi

        for bin in "${ALL_BINARIES[@]}"; do
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
        done

        # Stage MCP binaries into site/mcp/bin/
        for bin in "${BINARIES[@]}"; do
            staged_name="${bin}-${PLATFORM_KEY}"
            if [[ "${TRIPLE}" == *"-windows-"* ]]; then
                cp "${TARGET_DIR}/${TRIPLE}/release/${bin}.exe" "${SITE_DIR}/mcp/bin/${staged_name}"
            else
                cp "${TARGET_DIR}/${TRIPLE}/release/${bin}" "${SITE_DIR}/mcp/bin/${staged_name}"
            fi
            ok "Staged MCP binary ${staged_name}"
        done

        # Stage app binaries into site/apps/bin/
        for bin in "${CAP_BINARIES[@]}"; do
            staged_name="${bin}-${PLATFORM_KEY}"
            if [[ "${TRIPLE}" == *"-windows-"* ]]; then
                cp "${TARGET_DIR}/${TRIPLE}/release/${bin}.exe" "${SITE_DIR}/apps/bin/${staged_name}"
            else
                cp "${TARGET_DIR}/${TRIPLE}/release/${bin}" "${SITE_DIR}/apps/bin/${staged_name}"
            fi
            ok "Staged app binary ${staged_name}"
        done

        BUILT_KEYS+=("${PLATFORM_KEY}")
    done

    if [[ ${#BUILT_KEYS[@]} -eq 0 ]]; then
        echo "ERROR: no targets were built (use --no-build only after building at least once)" >&2
        exit 1
    fi
else
    info "No binary servers or apps to build — skipping compilation"
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

# ── Helper: substitute version and sha256 placeholders ──────────────────────

substitute_entry() {
    local entry_json="$1"

    # Substitute ${VERSION} with the crate version
    entry_json="$(echo "${entry_json}" | sed "s/\\\${VERSION}/${VERSION}/g")"

    # Substitute "${SHA256:<binary>}" with actual checksum objects
    if echo "${entry_json}" | grep -q '"\${SHA256:' ; then
        bin_name="$(echo "${entry_json}" | grep -oP '"\$\{SHA256:\K[^}]+' | head -1)"
        if [[ -n "${bin_name}" ]]; then
            sha_obj="$(sha256_json "${bin_name}")"
            entry_json="$(echo "${entry_json}" | sed "s|\"\\\${SHA256:${bin_name}}\"|${sha_obj}|g")"
        fi
    fi

    # Validate the result
    if ! echo "${entry_json}" | jq . > /dev/null 2>&1; then
        return 1
    fi

    echo "${entry_json}"
}

# ── Assemble MCP server entries ──────────────────────────────────────────────

info "Assembling MCP servers from ${MCP_REGISTERED_DIR}/*.json ..."

SERVERS="[]"
for server_file in "${MCP_REGISTERED_DIR}"/*.json; do
    [[ -f "${server_file}" ]] || continue
    id="$(jq -r '.id' "${server_file}")"

    server_json="$(substitute_entry "$(cat "${server_file}")")" || {
        echo "ERROR: invalid JSON after substitution in ${server_file}" >&2
        exit 1
    }

    SERVERS="$(echo "${SERVERS}" | jq --argjson entry "${server_json}" '. + [$entry]')"
    ok "Added MCP server: ${id}"
done

if [[ "$(echo "${SERVERS}" | jq 'length')" -eq 0 ]]; then
    echo "ERROR: no server definitions found in ${MCP_REGISTERED_DIR}/" >&2
    exit 1
fi

# ── Assemble app entries ─────────────────────────────────────────────────────

APPS="[]"
if [[ -d "${CAP_REGISTERED_DIR}" ]]; then
    info "Assembling apps from ${CAP_REGISTERED_DIR}/*.json ..."

    for cap_file in "${CAP_REGISTERED_DIR}"/*.json; do
        [[ -f "${cap_file}" ]] || continue
        id="$(jq -r '.id' "${cap_file}")"

        cap_json="$(substitute_entry "$(cat "${cap_file}")")" || {
            echo "ERROR: invalid JSON after substitution in ${cap_file}" >&2
            exit 1
        }

        APPS="$(echo "${APPS}" | jq --argjson entry "${cap_json}" '. + [$entry]')"
        ok "Added app: ${id}"
    done
fi

# ── Determine registry version ──────────────────────────────────────────────

if [[ "$(echo "${APPS}" | jq 'length')" -gt 0 ]]; then
    REGISTRY_VERSION=2
else
    REGISTRY_VERSION=1
fi

# ── Generate JSON ────────────────────────────────────────────────────────────

info "Generating ${OUTPUT}..."

jq -n \
    --argjson version "${REGISTRY_VERSION}" \
    --arg updated_at "${TODAY}" \
    --argjson servers "${SERVERS}" \
    --argjson apps "${APPS}" \
    '{ version: $version, updated_at: $updated_at, servers: $servers, apps: $apps }' \
    > "${OUTPUT}"

ok "Registry written to ${OUTPUT}"

# Also copy to the legacy location for non-Docker use
cp "${OUTPUT}" "${REGISTRY_DIR}/mcp-registry.json"
ok "Copied to ${REGISTRY_DIR}/mcp-registry.json"

# ── Build docker image ────────────────────────────────────────────────────────────

info "Building Docker image for registry site..."
docker build -t corre-registry "${REGISTRY_DIR}"

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}Registry summary:${NC}"
echo -e "  Version: ${REGISTRY_VERSION}"
jq -r '.servers[] | "  [mcp] \(.id) (v\(.version)) — \(.description)"' "${OUTPUT}"
jq -r '.apps[] | "  [app] \(.id) (v\(.version)) — \(.description)"' "${OUTPUT}" 2>/dev/null || true
echo ""
echo -e "${BOLD}Staged files:${NC}"
echo "  MCP binaries:"
ls -lh "${SITE_DIR}/mcp/bin/" 2>/dev/null || echo "    (none)"
echo "  App binaries:"
ls -lh "${SITE_DIR}/apps/bin/" 2>/dev/null || echo "    (none)"
