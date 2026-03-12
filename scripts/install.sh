#!/usr/bin/env bash
set -euo pipefail

VERSION="${IRONCLAD_VERSION:-latest}"
INSTALL_DIR="${IRONCLAD_INSTALL_DIR:-$HOME/.ironclad/bin}"
REPO="robot-accomplice/ironclad"

AUTO_YES="${IRONCLAD_YES:-0}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

info()  { echo -e "${CYAN}[info]${RESET}  $*"; }
ok()    { echo -e "${GREEN}[ok]${RESET}    $*"; }
warn()  { echo -e "${YELLOW}[warn]${RESET}  $*"; }
fail()  { echo -e "${RED}[error]${RESET} $*"; exit 1; }

confirm_or_exit() {
    local prompt="$1"
    if [ "$AUTO_YES" = "1" ]; then
        return 0
    fi
    printf "  %s [Y/n] " "$prompt"
    read -r answer </dev/tty
    case "${answer:-Y}" in
        [Yy]|[Yy][Ee][Ss]) return 0 ;;
        *) echo "Cancelled."; exit 0 ;;
    esac
}

detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        *)       fail "Unsupported OS: $(uname -s). Use Linux or macOS." ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)   echo "x86_64" ;;
        aarch64|arm64)  echo "aarch64" ;;
        *)              fail "Unsupported architecture: $(uname -m)" ;;
    esac
}

get_latest_version() {
    if command -v curl &>/dev/null; then
        curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": "v\?\([^"]*\)".*/\1/'
    elif command -v wget &>/dev/null; then
        wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": "v\?\([^"]*\)".*/\1/'
    else
        fail "curl or wget is required"
    fi
}

download() {
    local url="$1" dest="$2"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$dest"
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$dest"
    fi
}

verify_sha256() {
    local file="$1" expected="$2"
    local actual
    if command -v sha256sum &>/dev/null; then
        actual="$(sha256sum "$file" | awk '{print $1}')"
    elif command -v shasum &>/dev/null; then
        actual="$(shasum -a 256 "$file" | awk '{print $1}')"
    else
        warn "No SHA256 tool found (sha256sum or shasum), skipping verification"
        return 0
    fi
    [ "$actual" = "$expected" ]
}

main() {
    echo ""
    echo -e "${BOLD}Ironclad Installer${RESET}"
    echo ""

    # ── Liability Waiver ──────────────────────────────────────────
    echo -e "  ${BOLD}IMPORTANT — PLEASE READ${RESET}"
    echo ""
    echo "  Ironclad is an autonomous AI agent that can execute actions,"
    echo "  interact with external services, and manage digital assets"
    echo "  including cryptocurrency wallets and on-chain transactions."
    echo ""
    echo "  THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND."
    echo -e "  The developers and contributors bear ${BOLD}no responsibility${RESET} for:"
    echo ""
    echo "    - Actions taken by the agent, whether intended or unintended"
    echo "    - Loss of funds, income, cryptocurrency, or other digital assets"
    echo "    - Security vulnerabilities, compromises, or unauthorized access"
    echo "    - Damages arising from the agent's use, misuse, or malfunction"
    echo "    - Any financial, legal, or operational consequences whatsoever"
    echo ""
    echo "  By proceeding, you acknowledge that you use Ironclad entirely"
    echo "  at your own risk and accept full responsibility for its operation."
    echo ""
    confirm_or_exit "I understand and accept these terms"

    local os arch
    os="$(detect_os)"
    arch="$(detect_arch)"
    info "Detected: ${os}/${arch}"

    if [ "$VERSION" = "latest" ]; then
        info "Fetching latest release..."
        VERSION="$(get_latest_version)"
        if [ -z "$VERSION" ]; then
            fail "Could not determine latest version. Set IRONCLAD_VERSION manually."
        fi
    fi
    info "Version: ${VERSION}"

    local artifact="ironclad-${VERSION}-${arch}-${os}"
    local url="https://github.com/${REPO}/releases/download/v${VERSION}/${artifact}.tar.gz"
    local tmpdir
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    info "Downloading ${url}..."
    download "$url" "${tmpdir}/ironclad.tar.gz" || fail "Download failed. Check the version and try again."

    local sums_url="https://github.com/${REPO}/releases/download/v${VERSION}/SHA256SUMS.txt"
    info "Verifying SHA256 checksum..."
    if download "$sums_url" "${tmpdir}/SHA256SUMS.txt" 2>/dev/null; then
        local expected_sum
        expected_sum="$(grep "${artifact}.tar.gz" "${tmpdir}/SHA256SUMS.txt" | awk '{print $1}')"
        if [ -n "$expected_sum" ]; then
            if verify_sha256 "${tmpdir}/ironclad.tar.gz" "$expected_sum"; then
                ok "Checksum verified"
            else
                fail "SHA256 checksum mismatch! Archive may be corrupted or tampered with."
            fi
        else
            warn "No checksum entry for ${artifact}.tar.gz, skipping verification"
        fi
    else
        warn "SHA256SUMS.txt not available, skipping verification"
    fi

    info "Extracting..."
    tar -xzf "${tmpdir}/ironclad.tar.gz" -C "${tmpdir}" || fail "Extraction failed"

    mkdir -p "$INSTALL_DIR"
    local binary
    binary="$(find "$tmpdir" -name ironclad -type f -perm +111 2>/dev/null || find "$tmpdir" -name ironclad -type f 2>/dev/null | head -1)"
    if [ -z "$binary" ]; then
        binary="${tmpdir}/ironclad"
    fi

    if [ ! -f "$binary" ]; then
        fail "Binary not found in archive"
    fi

    cp "$binary" "${INSTALL_DIR}/ironclad"
    chmod +x "${INSTALL_DIR}/ironclad"
    ok "Installed to ${INSTALL_DIR}/ironclad"

    if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
        warn "Add ${INSTALL_DIR} to your PATH:"
        echo ""
        echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        warn "Add the above line to your shell profile (~/.bashrc, ~/.zshrc, etc.)"
    fi

    echo ""
    ok "Ironclad v${VERSION} installed successfully!"
    echo ""
    info "Next steps:"
    echo "    ironclad init        # Initialize a workspace"
    echo "    ironclad mechanic    # Run health checks"
    echo "    ironclad serve       # Start the server"
    echo ""
}

main "$@"
