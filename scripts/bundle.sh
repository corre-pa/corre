#!/usr/bin/env bash
set -euo pipefail

# ── Constants ────────────────────────────────────────────────────────────────

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${REPO_DIR}/dist"
BUNDLE_DIR="${DIST_DIR}/bundles"
BINARY_NAME="corre"
VERSION="$(grep -m1 '^version' "${REPO_DIR}/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"

# label|exe_suffix|os_family|config_suffix
PLATFORMS=(
    "linux-amd64||unix|linux"
    "linux-arm64||unix|linux"
    "linux-armv7||unix|linux"
    "windows-amd64|.exe|windows|windows"
    "macos-arm64||unix|macos"
    "macos-amd64||unix|macos"
)

# Files to include in every bundle (relative to REPO_DIR)
STATIC_FILES=(
    README.md
    config/topics.md
    templates/newspaper.html
    templates/settings.html
    templates/topics.html
    static/style.css
    static/settings.css
    static/settings.js
)

# ── Color helpers ────────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${BLUE}[INFO]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }

# ── Error trap ───────────────────────────────────────────────────────────────

trap_err() {
    error "Script failed at line $1. Check output above for details."
    exit 1
}
trap 'trap_err ${LINENO}' ERR

# ── Helpers ──────────────────────────────────────────────────────────────────

parse_platform() {
    local entry="$1"
    IFS='|' read -r LABEL EXE_SUFFIX OS_FAMILY CONFIG_SUFFIX <<< "${entry}"
}

# ── Build ────────────────────────────────────────────────────────────────────

run_build() {
    info "Running build-all.sh..."
    bash "${REPO_DIR}/scripts/build-all.sh" "$@"
    echo ""
}

# ── Bundle ───────────────────────────────────────────────────────────────────

bundle_platform() {
    local label="$1"
    local exe_suffix="$2"
    local os_family="$3"
    local config_suffix="$4"

    local binary_src="${DIST_DIR}/${BINARY_NAME}-${VERSION}-${label}${exe_suffix}"
    if [[ ! -f "${binary_src}" ]]; then
        warn "Binary not found for ${label}, skipping: ${binary_src}"
        return
    fi

    local bundle_name="${BINARY_NAME}-${VERSION}-${label}"
    local staging="${BUNDLE_DIR}/${bundle_name}"

    # Create directory structure
    rm -rf "${staging}"
    mkdir -p "${staging}/config" "${staging}/templates" "${staging}/static"

    # Copy binary
    cp "${binary_src}" "${staging}/${BINARY_NAME}${exe_suffix}"

    # Copy OS-specific config as corre.toml
    local config_src="${REPO_DIR}/corre.toml.${config_suffix}"
    if [[ -f "${config_src}" ]]; then
        cp "${config_src}" "${staging}/corre.toml"
    else
        warn "No default config found: ${config_src}"
        cp "${REPO_DIR}/corre.toml" "${staging}/corre.toml"
    fi

    # Copy static files preserving directory structure
    for file in "${STATIC_FILES[@]}"; do
        local dest="${staging}/${file}"
        mkdir -p "$(dirname "${dest}")"
        cp "${REPO_DIR}/${file}" "${dest}"
    done

    # Create archive
    local archive
    if [[ "${os_family}" == "windows" ]]; then
        archive="${BUNDLE_DIR}/${bundle_name}.zip"
        (cd "${BUNDLE_DIR}" && zip -rq "${bundle_name}.zip" "${bundle_name}")
    else
        archive="${BUNDLE_DIR}/${bundle_name}.tar.gz"
        tar -czf "${archive}" -C "${BUNDLE_DIR}" "${bundle_name}"
    fi

    # Clean up staging directory
    rm -rf "${staging}"

    ok "Bundled ${archive}"
}

# ── Checksums ────────────────────────────────────────────────────────────────

generate_checksums() {
    local checksum_file="${BUNDLE_DIR}/checksums-sha256.txt"
    info "Generating bundle checksums..."

    local checksum_cmd
    if [[ "$(uname -s)" == "Darwin" ]]; then
        checksum_cmd="shasum -a 256"
    else
        checksum_cmd="sha256sum"
    fi

    (cd "${BUNDLE_DIR}" && ${checksum_cmd} *.tar.gz *.zip 2>/dev/null > checksums-sha256.txt)
    ok "Checksums written to ${checksum_file}"
}

# ── Summary ──────────────────────────────────────────────────────────────────

print_summary() {
    echo ""
    echo -e "${BOLD}════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}  Bundle Summary (v${VERSION})${NC}"
    echo -e "${BOLD}════════════════════════════════════════════════════════${NC}"
    echo ""

    printf "  ${BOLD}%-44s %10s${NC}\n" "Archive" "Size"
    printf "  %-44s %10s\n" "--------------------------------------------" "----------"

    for f in "${BUNDLE_DIR}"/*.tar.gz "${BUNDLE_DIR}"/*.zip; do
        [[ -f "${f}" ]] || continue
        local name size
        name="$(basename "${f}")"
        if [[ "$(uname -s)" == "Darwin" ]]; then
            size="$(stat -f '%z' "${f}" | awk '{ printf "%.1f MB", $1/1048576 }')"
        else
            size="$(stat --printf='%s' "${f}" | awk '{ printf "%.1f MB", $1/1048576 }')"
        fi
        printf "  %-44s %10s\n" "${name}" "${size}"
    done

    echo ""
    echo -e "  Output directory: ${BUNDLE_DIR}"
    echo ""
    echo -e "  ${BOLD}Usage:${NC}"
    echo -e "    tar xzf ${BINARY_NAME}-${VERSION}-linux-amd64.tar.gz"
    echo -e "    cd ${BINARY_NAME}-${VERSION}-linux-amd64"
    echo -e "    ./corre setup"
    echo ""
    echo -e "${BOLD}════════════════════════════════════════════════════════${NC}"
}

# ── Main ─────────────────────────────────────────────────────────────────────

main() {
    local skip_build=false
    local build_args=()

    for arg in "$@"; do
        case "${arg}" in
            --skip-build)
                skip_build=true
                ;;
            *)
                build_args+=("${arg}")
                ;;
        esac
    done

    echo ""
    echo -e "${BOLD}Corre Bundle${NC} (v${VERSION})"
    echo -e "Repository: ${REPO_DIR}"
    echo ""

    # Step 1: Build binaries (unless skipped)
    if [[ "${skip_build}" == false ]]; then
        run_build "${build_args[@]+"${build_args[@]}"}"
    else
        info "Skipping build (--skip-build)"
    fi

    # Step 2: Verify static files exist
    for file in "${STATIC_FILES[@]}"; do
        if [[ ! -f "${REPO_DIR}/${file}" ]]; then
            error "Required file missing: ${file}"
            exit 1
        fi
    done
    ok "All static files present"

    # Step 3: Create bundles
    rm -rf "${BUNDLE_DIR}"
    mkdir -p "${BUNDLE_DIR}"

    local bundled=0
    for entry in "${PLATFORMS[@]}"; do
        parse_platform "${entry}"
        bundle_platform "${LABEL}" "${EXE_SUFFIX}" "${OS_FAMILY}" "${CONFIG_SUFFIX}"
        bundled=$((bundled + 1))
    done

    if [[ "${bundled}" -eq 0 ]]; then
        error "No bundles were created."
        exit 1
    fi

    # Step 4: Checksums and summary
    generate_checksums
    print_summary
}

main "$@"
