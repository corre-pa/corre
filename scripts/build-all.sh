#!/usr/bin/env bash
set -euo pipefail

# ── Constants ────────────────────────────────────────────────────────────────

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${REPO_DIR}/dist"
PACKAGE="corre-cli"
BINARY_NAME="corre"
VERSION="$(grep -m1 '^version' "${REPO_DIR}/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"

# Target definitions: triple|label|exe_suffix|build_tool|os_required
TARGETS=(
    "x86_64-unknown-linux-gnu|linux-amd64||cross|"
    "aarch64-unknown-linux-gnu|linux-arm64||cross|"
    "armv7-unknown-linux-gnueabihf|linux-armv7||cross|"
    "x86_64-pc-windows-gnu|windows-amd64|.exe|cross|"
    "aarch64-apple-darwin|macos-arm64||cargo|Darwin"
    "x86_64-apple-darwin|macos-amd64||cargo|Darwin"
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

command_exists() {
    command -v "$1" &>/dev/null
}

host_os() {
    uname -s
}

parse_target() {
    local entry="$1"
    IFS='|' read -r TRIPLE LABEL EXE_SUFFIX BUILD_TOOL OS_REQUIRED <<< "${entry}"
}

should_build() {
    local os_required="$1"
    [[ -z "${os_required}" ]] || [[ "$(host_os)" == "${os_required}" ]]
}

output_name() {
    echo "${BINARY_NAME}-${VERSION}-${LABEL}${EXE_SUFFIX}"
}

source_binary() {
    local triple="$1"
    local exe_suffix="$2"
    echo "${REPO_DIR}/target/${triple}/release/${BINARY_NAME}${exe_suffix}"
}

# ── Prerequisites ────────────────────────────────────────────────────────────

check_prerequisites() {
    local need_cross=false
    local need_cargo=false

    for entry in "${TARGETS[@]}"; do
        parse_target "${entry}"
        if ! should_build "${OS_REQUIRED}"; then
            continue
        fi
        if [[ -n "${SINGLE_TARGET:-}" && "${TRIPLE}" != "${SINGLE_TARGET}" ]]; then
            continue
        fi
        case "${BUILD_TOOL}" in
            cross) need_cross=true ;;
            cargo) need_cargo=true ;;
        esac
    done

    if [[ "${need_cross}" == true ]]; then
        if ! command_exists cross; then
            error "'cross' is not installed. Install with: cargo install cross --git https://github.com/cross-rs/cross"
            exit 1
        fi
        ok "cross found at $(command -v cross)"

        if ! docker info &>/dev/null 2>&1; then
            error "Docker is not running. cross requires Docker for cross-compilation."
            exit 1
        fi
        ok "Docker is running"
    fi

    if [[ "${need_cargo}" == true ]]; then
        if ! command_exists cargo; then
            error "'cargo' is not installed. Install via https://rustup.rs"
            exit 1
        fi
        ok "cargo found at $(command -v cargo)"
    fi
}

# ── Build ────────────────────────────────────────────────────────────────────

ensure_rustup_target() {
    local triple="$1"
    if ! rustup target list --installed | grep -q "^${triple}$"; then
        info "Installing rustup target ${triple}..."
        rustup target add "${triple}"
    fi
}

build_target() {
    local triple="$1"
    local build_tool="$2"

    info "Building for ${triple} (${build_tool})..."

    if [[ "${build_tool}" == "cargo" ]]; then
        ensure_rustup_target "${triple}"
        cargo build --release --package "${PACKAGE}" --target "${triple}"
    else
        cross build --release --package "${PACKAGE}" --target "${triple}"
    fi
}

# ── Strip ────────────────────────────────────────────────────────────────────

strip_binary() {
    local file="$1"
    local triple="$2"

    if [[ "${triple}" == *"windows"* ]]; then
        # Windows binaries — try llvm-strip or skip
        if command_exists llvm-strip; then
            llvm-strip "${file}" && ok "Stripped ${file} (llvm-strip)" && return
        fi
        warn "No strip tool available for ${triple}, skipping"
        return
    fi

    case "${triple}" in
        *apple*)
            # macOS targets — native strip works
            if command_exists strip; then
                strip "${file}" && ok "Stripped ${file}" && return
            fi
            ;;
        *)
            # Linux cross targets — prefer llvm-strip, fall back to strip
            if command_exists llvm-strip; then
                llvm-strip "${file}" && ok "Stripped ${file} (llvm-strip)" && return
            elif command_exists strip; then
                strip "${file}" && ok "Stripped ${file}" && return
            fi
            ;;
    esac

    warn "No strip tool available for ${triple}, skipping"
}

# ── Checksums ────────────────────────────────────────────────────────────────

generate_checksums() {
    local checksum_file="${DIST_DIR}/checksums-sha256.txt"

    info "Generating checksums..."

    if [[ "$(host_os)" == "Darwin" ]]; then
        (cd "${DIST_DIR}" && shasum -a 256 "${BINARY_NAME}"-* > checksums-sha256.txt)
    else
        (cd "${DIST_DIR}" && sha256sum "${BINARY_NAME}"-* > checksums-sha256.txt)
    fi

    ok "Checksums written to ${checksum_file}"
}

# ── Summary ──────────────────────────────────────────────────────────────────

print_summary() {
    echo ""
    echo -e "${BOLD}════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}  Build Summary (v${VERSION})${NC}"
    echo -e "${BOLD}════════════════════════════════════════════════════════${NC}"
    echo ""

    printf "  ${BOLD}%-38s %10s${NC}\n" "Binary" "Size"
    printf "  %-38s %10s\n" "--------------------------------------" "----------"

    for f in "${DIST_DIR}/${BINARY_NAME}"-*; do
        [[ -f "${f}" ]] || continue
        local name size
        name="$(basename "${f}")"
        # skip checksums file
        [[ "${name}" == "checksums-sha256.txt" ]] && continue
        if [[ "$(host_os)" == "Darwin" ]]; then
            size="$(stat -f '%z' "${f}" | awk '{ printf "%.1f MB", $1/1048576 }')"
        else
            size="$(stat --printf='%s' "${f}" | awk '{ printf "%.1f MB", $1/1048576 }')"
        fi
        printf "  %-38s %10s\n" "${name}" "${size}"
    done

    echo ""
    echo -e "  Output directory: ${DIST_DIR}"
    echo ""
    echo -e "${BOLD}════════════════════════════════════════════════════════${NC}"
}

# ── Main ─────────────────────────────────────────────────────────────────────

main() {
    local SINGLE_TARGET="${1:-}"

    echo ""
    echo -e "${BOLD}Corre Cross-Compile${NC} (v${VERSION})"
    echo -e "Repository: ${REPO_DIR}"
    echo ""

    if [[ -n "${SINGLE_TARGET}" ]]; then
        info "Single target mode: ${SINGLE_TARGET}"
        # Validate the target exists
        local found=false
        for entry in "${TARGETS[@]}"; do
            parse_target "${entry}"
            if [[ "${TRIPLE}" == "${SINGLE_TARGET}" ]]; then
                found=true
                break
            fi
        done
        if [[ "${found}" == false ]]; then
            error "Unknown target: ${SINGLE_TARGET}"
            echo ""
            info "Available targets:"
            for entry in "${TARGETS[@]}"; do
                parse_target "${entry}"
                echo "  ${TRIPLE}"
            done
            exit 1
        fi
    fi

    check_prerequisites

    # Clean and recreate dist/
    rm -rf "${DIST_DIR}"
    mkdir -p "${DIST_DIR}"
    ok "Clean dist/ directory created"

    # Build each target
    local built=0
    for entry in "${TARGETS[@]}"; do
        parse_target "${entry}"

        if [[ -n "${SINGLE_TARGET}" && "${TRIPLE}" != "${SINGLE_TARGET}" ]]; then
            continue
        fi

        if ! should_build "${OS_REQUIRED}"; then
            info "Skipping ${LABEL} (${TRIPLE}) — requires ${OS_REQUIRED} (macOS builds need Apple hardware)"
            continue
        fi

        build_target "${TRIPLE}" "${BUILD_TOOL}"

        local src dst
        src="$(source_binary "${TRIPLE}" "${EXE_SUFFIX}")"
        dst="${DIST_DIR}/$(output_name)"

        if [[ ! -f "${src}" ]]; then
            error "Expected binary not found: ${src}"
            exit 1
        fi

        cp "${src}" "${dst}"
        strip_binary "${dst}" "${TRIPLE}"
        ok "Built $(output_name)"
        built=$((built + 1))
    done

    if [[ "${built}" -eq 0 ]]; then
        error "No targets were built."
        exit 1
    fi

    generate_checksums
    print_summary
}

main "$@"
