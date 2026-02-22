#!/usr/bin/env bash
#
# Ironclad Installer
#
# Interactive usage (recommended):
#   bash <(curl -fsSL https://roboticus.ai/install.sh)
#
# Non-interactive (skip all prompts):
#   curl -fsSL ... | IRONCLAD_YES=1 bash
#
# Environment variables:
#   IRONCLAD_VERSION  Specific version to install (default: latest)
#   IRONCLAD_NO_INIT  Skip "ironclad init" after install (set to 1)
#   IRONCLAD_YES      Skip all confirmation prompts (set to 1)
#   CARGO_HOME        Custom cargo home (respected if set)

set -euo pipefail

CRATE="ironclad-server"
VERSION="${IRONCLAD_VERSION:-}"
MIN_RUST="1.85.0"
AUTO_YES="${IRONCLAD_YES:-0}"

# ── Helpers ──────────────────────────────────────────────────────────────────

bold()  { printf "\033[1m%s\033[0m"  "$*"; }
green() { printf "\033[1;32m%s\033[0m" "$*"; }
yellow(){ printf "\033[1;33m%s\033[0m" "$*"; }
red()   { printf "\033[1;31m%s\033[0m" "$*"; }
dim()   { printf "\033[2m%s\033[0m"  "$*"; }

step()  { printf "\n  $(green "▸") %s\n" "$*"; }
info()  { printf "    %s\n" "$*"; }
warn()  { printf "  $(yellow "⚠") %s\n" "$*"; }
fail()  { printf "  $(red "✖") %s\n" "$*"; exit 1; }

command_exists() { command -v "$1" >/dev/null 2>&1; }

version_ge() {
    printf '%s\n%s' "$1" "$2" | sort -V | head -n1 | grep -qx "$2"
}

confirm() {
    local prompt="$1"
    local default="${2:-y}"

    if [ "$AUTO_YES" = "1" ]; then
        return 0
    fi

    if [ ! -t 0 ]; then
        warn "Running non-interactively without IRONCLAD_YES=1"
        warn "Cannot prompt for confirmation — aborting."
        info "Re-run with: IRONCLAD_YES=1 curl ... | bash"
        info "         or: bash <(curl ...) for interactive mode"
        exit 1
    fi

    local hint
    if [ "$default" = "y" ]; then
        hint="[Y/n]"
    else
        hint="[y/N]"
    fi

    printf "\n    %s %s " "$prompt" "$(dim "$hint")"
    read -r answer </dev/tty

    case "${answer:-$default}" in
        [Yy]|[Yy][Ee][Ss]) return 0 ;;
        *) printf "    %s\n" "Skipped."; return 1 ;;
    esac
}

abort_if_declined() {
    local prompt="$1"
    if ! confirm "$prompt" "y"; then
        printf "\n  $(red "✖") Installation cancelled by user.\n\n"
        exit 0
    fi
}

# ── Banner ───────────────────────────────────────────────────────────────────

# Source of truth: banner.txt in the project root
cat <<'BANNER'

        ╔═══╗
        ║◉ ◉║
        ║ ▬ ║
        ╚═╤═╝
      ╔═══╪═══╗       I R O N C L A D
  ╔═══╣ ▓▓║▓▓ ╠═══╗   Autonomous Agent Runtime
  █   ║ ▓▓║▓▓ ║   █
      ╚══╤═╤══╝
         ║ ║
        ═╩═╩═

BANNER

if [ -n "$VERSION" ]; then
    printf "  Installer • version: $(bold "$VERSION")\n"
else
    printf "  Installer • version: $(bold "latest")\n"
fi

# ── Plan ─────────────────────────────────────────────────────────────────────

CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
VERSION_DISPLAY="${VERSION:-latest}"

step "Installation plan"
info "This installer will:"
info ""
info "  1. Check prerequisites (C compiler)"
info "  2. Install or update the Rust toolchain (>= $MIN_RUST)"
info "  3. Install Ironclad from crates.io (cargo install $CRATE)"
info ""
info "Version:          $(bold "$VERSION_DISPLAY")"
info "Binary location:  $(bold "$CARGO_BIN/ironclad")"

abort_if_declined "Proceed with installation?"

# ── Preflight ────────────────────────────────────────────────────────────────

step "Checking prerequisites"

OS="$(uname -s)"
ARCH="$(uname -m)"
info "Platform: $OS $ARCH"

case "$OS" in
    Linux|Darwin) ;;
    MINGW*|MSYS*|CYGWIN*)
        warn "Windows detected — using MSYS/MinGW environment"
        ;;
    *)
        fail "Unsupported OS: $OS"
        ;;
esac

if command_exists cc; then
    info "C compiler: cc ✓"
elif command_exists gcc; then
    info "C compiler: gcc ✓"
elif command_exists clang; then
    info "C compiler: clang ✓"
else
    warn "No C compiler found. Rust's bundled SQLite needs a C toolchain."
    case "$OS" in
        Linux)
            info "Try: sudo apt install build-essential   (Debian/Ubuntu)"
            info "  or: sudo dnf groupinstall 'Development Tools'   (Fedora)"
            ;;
        Darwin)
            info "Try: xcode-select --install"
            ;;
    esac
    fail "Install a C compiler and re-run this script."
fi

# ── Rust ─────────────────────────────────────────────────────────────────────

NEED_RUST_INSTALL=0
NEED_RUST_UPDATE=0

if command_exists rustc && command_exists cargo; then
    RUST_VER="$(rustc --version | awk '{print $2}')"
    info "Found Rust $RUST_VER"

    if version_ge "$RUST_VER" "$MIN_RUST"; then
        info "Version $RUST_VER meets minimum ($MIN_RUST) ✓"
    else
        NEED_RUST_UPDATE=1
    fi
else
    NEED_RUST_INSTALL=1
fi

if [ "$NEED_RUST_INSTALL" = "1" ]; then
    step "Rust toolchain not found"
    info "Ironclad requires Rust >= $MIN_RUST."
    info "This will install Rust via rustup (https://rustup.rs)."
    info "Rustup will be configured with the stable toolchain."

    abort_if_declined "Install Rust now?"

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck disable=SC1091
    source "${CARGO_HOME:-$HOME/.cargo}/env"
    RUST_VER="$(rustc --version | awk '{print $2}')"
    info "Installed Rust $RUST_VER ✓"

elif [ "$NEED_RUST_UPDATE" = "1" ]; then
    step "Rust update required"
    info "Installed version $RUST_VER is below the minimum ($MIN_RUST)."
    info "This will run: rustup update stable"

    abort_if_declined "Update Rust now?"

    rustup update stable
    RUST_VER="$(rustc --version | awk '{print $2}')"
    if ! version_ge "$RUST_VER" "$MIN_RUST"; then
        fail "Could not upgrade Rust to >= $MIN_RUST (got $RUST_VER)"
    fi
    info "Updated to Rust $RUST_VER ✓"
fi

# ── Install from crates.io ──────────────────────────────────────────────────

step "Install from crates.io"

if [ -n "$VERSION" ]; then
    INSTALL_CMD="cargo install $CRATE --version $VERSION --locked"
else
    INSTALL_CMD="cargo install $CRATE --locked"
fi

info "This will run:"
info "  $(bold "$INSTALL_CMD")"
info ""
info "Cargo will download, compile, and install the Ironclad binary."
info "This typically takes 2-5 minutes on the first install."

abort_if_declined "Install Ironclad now?"

$INSTALL_CMD 2>&1 | while IFS= read -r line; do
    case "$line" in
        *Compiling*) printf "\r    Compiling: %-40s" "$(echo "$line" | awk '{print $2}')" ;;
        *Finished*)  printf "\r    %-60s\n" "$line" ;;
        *Installing*) info "$line" ;;
        *warning*) ;;
    esac
done

IRONCLAD_BIN="$(command -v ironclad 2>/dev/null || echo "$CARGO_BIN/ironclad")"

if [ ! -x "$IRONCLAD_BIN" ]; then
    fail "Binary not found after install. Ensure $CARGO_BIN is in your PATH."
fi

INSTALLED_VER="$("$IRONCLAD_BIN" version 2>/dev/null | head -1 || echo "unknown")"

# ── Verify ───────────────────────────────────────────────────────────────────

step "Verifying installation"
info "Binary:  $IRONCLAD_BIN"
info "Version: $INSTALLED_VER"

case ":$PATH:" in
    *":$CARGO_BIN:"*) ;;
    *)
        warn "$CARGO_BIN is not in your PATH"
        info "Add this to your shell profile:"
        info "  export PATH=\"$CARGO_BIN:\$PATH\""
        ;;
esac

# ── Workspace Init ───────────────────────────────────────────────────────────

if [ "${IRONCLAD_NO_INIT:-0}" != "1" ]; then
    step "Initialize workspace"
    info "This creates the Ironclad data directory at $(bold "~/.ironclad/") with"
    info "default configuration, skill templates, and an empty database."

    if confirm "Initialize workspace now?" "y"; then
        "$IRONCLAD_BIN" init 2>/dev/null || true
        info "Workspace ready ✓"
    else
        info "You can initialize later with: $(bold "ironclad init")"
    fi
fi

# ── Done ─────────────────────────────────────────────────────────────────────

cat <<DONE

  $(green "✔") $(bold "Ironclad installed successfully!")

  Get started:
    $(bold "ironclad setup")       Interactive configuration wizard
    $(bold "ironclad serve")       Start the agent runtime
    $(bold "ironclad dashboard")   Open the web dashboard
    $(bold "ironclad --help")      Show all commands

DONE
