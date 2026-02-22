#!/usr/bin/env bash
set -euo pipefail

VERSION="${IRONCLAD_VERSION:-latest}"
INSTALL_DIR="${IRONCLAD_INSTALL_DIR:-$HOME/.ironclad/bin}"
REPO="robot-accomplice/ironclad"

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

main() {
    echo ""
    echo -e "${BOLD}Ironclad Installer${RESET}"
    echo ""

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
