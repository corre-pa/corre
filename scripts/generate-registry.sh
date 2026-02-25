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
REGISTRY_DIR="${REPO_DIR}/mcp-registry"
REGISTERED_DIR="${REGISTRY_DIR}/registered"
SITE_DIR="${REGISTRY_DIR}/site"
OUTPUT="${SITE_DIR}/registry.json"
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
for f in "${REGISTERED_DIR}"/*.json; do
    [[ -f "$f" ]] || continue
    method="$(jq -r '.install.method' "$f")"
    if [[ "${method}" == "binary" ]]; then
        bin="$(jq -r '.install.binary_name' "$f")"
        BINARIES+=("${bin}")
        MCP_PACKAGES+=" --package ${bin}"
    fi
done

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
mkdir -p "${SITE_DIR}/bin"

# ── Build binaries per target ────────────────────────────────────────────────

# Associative array: CHECKSUMS["binary|platform_key"] = hex
declare -A CHECKSUMS
BUILT_KEYS=()

if [[ ${#BINARIES[@]} -gt 0 ]]; then
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
            for bin in "${BINARIES[@]}"; do
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
                cargo build --release ${MCP_PACKAGES} --target "${TRIPLE}"
            else
                CARGO_TARGET_DIR="${CROSS_TARGET_DIR}" cross build --release ${MCP_PACKAGES} --target "${TRIPLE}"
            fi
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
        echo "ERROR: no targets were built (use --no-build only after building at least once)" >&2
        exit 1
    fi
else
    info "No binary MCP servers to build — skipping compilation"
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

# ── Assemble registry from individual server definitions ─────────────────────

info "Assembling registry from ${REGISTERED_DIR}/*.json ..."

# Collect server entries, applying build-time substitutions:
#   ${VERSION}           -> crate version from Cargo.toml
#   "${SHA256:<binary>}" -> per-platform checksum object from the build step
SERVERS="[]"
for server_file in "${REGISTERED_DIR}"/*.json; do
    [[ -f "${server_file}" ]] || continue
    id="$(jq -r '.id' "${server_file}")"

    # Start with the raw JSON from the file
    server_json="$(cat "${server_file}")"

    # Substitute ${VERSION} with the crate version
    server_json="$(echo "${server_json}" | sed "s/\\\${VERSION}/${VERSION}/g")"

    # Substitute "${SHA256:<binary>}" with actual checksum objects
    if echo "${server_json}" | grep -q '"\${SHA256:' ; then
        bin_name="$(echo "${server_json}" | grep -oP '"\$\{SHA256:\K[^}]+' | head -1)"
        if [[ -n "${bin_name}" ]]; then
            sha_obj="$(sha256_json "${bin_name}")"
            # Replace the placeholder string with the JSON object (remove surrounding quotes)
            server_json="$(echo "${server_json}" | sed "s|\"\\\${SHA256:${bin_name}}\"|${sha_obj}|g")"
        fi
    fi

    # Validate the result and append
    if ! echo "${server_json}" | jq . > /dev/null 2>&1; then
        echo "ERROR: invalid JSON after substitution in ${server_file}" >&2
        exit 1
    fi

    SERVERS="$(echo "${SERVERS}" | jq --argjson entry "${server_json}" '. + [$entry]')"
    ok "Added ${id}"
done

if [[ "$(echo "${SERVERS}" | jq 'length')" -eq 0 ]]; then
    echo "ERROR: no server definitions found in ${REGISTERED_DIR}/" >&2
    exit 1
fi

# ── Generate JSON ────────────────────────────────────────────────────────────

info "Generating ${OUTPUT}..."

jq -n \
    --arg updated_at "${TODAY}" \
    --argjson servers "${SERVERS}" \
    '{ version: 1, updated_at: $updated_at, servers: $servers }' \
    > "${OUTPUT}"

ok "Registry written to ${OUTPUT}"

# Also copy to the legacy location for non-Docker use
cp "${OUTPUT}" "${REGISTRY_DIR}/mcp-registry.json"
ok "Copied to ${REGISTRY_DIR}/mcp-registry.json"

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}Registry summary:${NC}"
jq -r '.servers[] | "  \(.id) (v\(.version)) — \(.description)"' "${OUTPUT}"
echo ""
echo -e "${BOLD}Staged files:${NC}"
ls -lh "${SITE_DIR}/bin/" 2>/dev/null || echo "  (no binaries staged)"
