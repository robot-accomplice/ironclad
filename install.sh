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
# Native Windows (PowerShell):
#   irm https://roboticus.ai/install.ps1 | iex
#
# Environment variables:
#   IRONCLAD_VERSION  Specific version to install (default: latest)
#   IRONCLAD_NO_INIT  Skip "ironclad init" after install (set to 1)
#   IRONCLAD_YES      Skip all confirmation prompts (set to 1)
#   IRONCLAD_INSTALL_LOCKED  Use cargo --locked (default: 1, set to 0 for non-locked)
#   CARGO_HOME        Custom cargo home (respected if set)

set -euo pipefail

CRATE="ironclad-server"
VERSION="${IRONCLAD_VERSION:-}"
MIN_RUST="1.85.0"
AUTO_YES="${IRONCLAD_YES:-0}"
INSTALL_LOCKED="${IRONCLAD_INSTALL_LOCKED:-1}"
APERTUS_8B_REPO="${APERTUS_8B_REPO:-swiss-ai/Apertus-8B-Instruct}"
APERTUS_70B_REPO="${APERTUS_70B_REPO:-swiss-ai/Apertus-70B-Instruct}"
APERTUS_8B_OLLAMA="${APERTUS_8B_OLLAMA:-apertus:8b-instruct}"
APERTUS_70B_OLLAMA="${APERTUS_70B_OLLAMA:-apertus:70b-instruct}"

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

verify_sha256() {
    local file="$1" expected="$2"
    local actual
    if command_exists sha256sum; then
        actual="$(sha256sum "$file" | awk '{print $1}')"
    elif command_exists shasum; then
        actual="$(shasum -a 256 "$file" | awk '{print $1}')"
    else
        return 1  # no tool available, skip verification
    fi
    [ "$actual" = "$expected" ]
}

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

detect_total_ram_gb() {
    case "$(uname -s)" in
        Darwin)
            local mem
            mem="$(sysctl -n hw.memsize 2>/dev/null || echo 0)"
            echo $((mem / 1024 / 1024 / 1024))
            ;;
        Linux)
            local kb
            kb="$(awk '/MemTotal/ {print $2}' /proc/meminfo 2>/dev/null || echo 0)"
            echo $((kb / 1024 / 1024))
            ;;
        *)
            echo 0
            ;;
    esac
}

detect_gpu_vram_gb() {
    if command_exists nvidia-smi; then
        local mb
        mb="$(nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits 2>/dev/null | head -n1 || echo 0)"
        echo $((mb / 1024))
        return
    fi
    echo 0
}

host_ready() {
    case "$1" in
        sglang) command_exists sglang ;;
        vllm) command_exists vllm ;;
        docker-model-runner) command_exists docker ;;
        ollama) command_exists ollama ;;
        *) return 1 ;;
    esac
}

host_has_models() {
    case "$1" in
        ollama)
            if ! command_exists ollama; then
                return 1
            fi
            local lines
            lines="$(ollama list 2>/dev/null | wc -l | tr -d ' ')"
            [ "${lines:-0}" -gt 1 ]
            ;;
        *)
            return 1
            ;;
    esac
}

has_hf_model_cache() {
    local cache_root="${HF_HOME:-$HOME/.cache/huggingface}/hub"
    [ -d "$cache_root" ] || return 1
    compgen -G "$cache_root/models--*" >/dev/null 2>&1
}

has_existing_local_model_stack() {
    for h in sglang vllm docker-model-runner ollama; do
        if host_ready "$h"; then
            return 0
        fi
    done

    if host_has_models "ollama"; then
        return 0
    fi

    if has_hf_model_cache; then
        return 0
    fi

    return 1
}

attempt_host_install() {
    local host="$1"
    case "$host" in
        sglang)
            local pybin=""
            if command_exists python3; then pybin="python3"; elif command_exists python; then pybin="python"; fi
            if [ -z "$pybin" ]; then
                warn "Python not found. Cannot auto-install SGLang."
                return 1
            fi
            "$pybin" -m pip install --user "sglang[all]"
            ;;
        vllm)
            local pybin=""
            if command_exists python3; then pybin="python3"; elif command_exists python; then pybin="python"; fi
            if [ -z "$pybin" ]; then
                warn "Python not found. Cannot auto-install vLLM."
                return 1
            fi
            "$pybin" -m pip install --user vllm
            ;;
        docker-model-runner)
            warn "Automatic Docker installation is not supported by this installer."
            info "Install Docker Desktop/Engine first, then re-run setup."
            return 1
            ;;
        ollama)
            warn "Automatic Ollama installation is not supported by this installer."
            info "Install Ollama from https://ollama.ai and re-run setup."
            return 1
            ;;
        *)
            return 1
            ;;
    esac
}

install_apertus_for_host() {
    local host="$1"
    local variant="$2"
    local repo=""
    local ollama_model=""
    if [ "$variant" = "70b" ]; then
        repo="$APERTUS_70B_REPO"
        ollama_model="$APERTUS_70B_OLLAMA"
    else
        repo="$APERTUS_8B_REPO"
        ollama_model="$APERTUS_8B_OLLAMA"
    fi

    case "$host" in
        ollama)
            if ! command_exists ollama; then
                warn "Ollama is not installed; cannot pull Apertus."
                return 1
            fi
            ollama pull "$ollama_model"
            ;;
        sglang|vllm|docker-model-runner)
            local pybin=""
            if command_exists python3; then pybin="python3"; elif command_exists python; then pybin="python"; fi
            if [ -z "$pybin" ]; then
                warn "Python not found. Cannot pre-download Apertus weights."
                return 1
            fi
            "$pybin" - <<PY
from huggingface_hub import snapshot_download
snapshot_download(repo_id="${repo}")
print("Downloaded ${repo}")
PY
            ;;
        *)
            return 1
            ;;
    esac
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
LOCKED_DISPLAY="enabled"
if [ "$INSTALL_LOCKED" = "0" ]; then
    LOCKED_DISPLAY="disabled"
fi

step "Installation plan"
info "This installer will:"
info ""
info "  1. Check prerequisites (C compiler)"
info "  2. Install or update the Rust toolchain (>= $MIN_RUST)"
info "  3. Download precompiled binary from GitHub Releases (SHA256-verified)"
info "     Falls back to cargo install if binary unavailable for this platform"
info ""
info "Version:          $(bold "$VERSION_DISPLAY")"
info "Cargo --locked:   $(bold "$LOCKED_DISPLAY")"
info "Binary location:  $(bold "$CARGO_BIN/ironclad")"

# ── Liability Waiver ────────────────────────────────────────────────────────
printf "\n"
step "Disclaimer"
info "$(bold "IMPORTANT — PLEASE READ")"
info ""
info "  Ironclad is an autonomous AI agent that can execute actions,"
info "  interact with external services, and manage digital assets"
info "  including cryptocurrency wallets and on-chain transactions."
info ""
info "  THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND."
info "  The developers and contributors bear $(bold "no responsibility") for:"
info ""
info "    • Actions taken by the agent, whether intended or unintended"
info "    • Loss of funds, income, cryptocurrency, or other digital assets"
info "    • Security vulnerabilities, compromises, or unauthorized access"
info "    • Damages arising from the agent's use, misuse, or malfunction"
info "    • Any financial, legal, or operational consequences whatsoever"
info ""
info "  By proceeding, you acknowledge that you use Ironclad entirely"
info "  at your own risk and accept full responsibility for its operation."
info ""

abort_if_declined "I understand and accept these terms"

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

# ── Install ─────────────────────────────────────────────────────────────────

BINARY_INSTALLED=0
GITHUB_REPO="robot-accomplice/ironclad"

# Map platform to release artifact naming
map_platform() {
    local os arch
    case "$OS" in
        Darwin) os="macos" ;;
        Linux)  os="linux" ;;
        *)      return 1 ;;
    esac
    case "$ARCH" in
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *)              return 1 ;;
    esac
    echo "${arch}-${os}"
}

try_binary_download() {
    local platform version archive_url sums_url tmpdir archive expected_sum

    platform="$(map_platform)" || return 1

    # Resolve version if not set
    if [ -z "$VERSION" ]; then
        info "Resolving latest release version..."
        version="$(curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" 2>/dev/null \
            | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": "v\?\([^"]*\)".*/\1/')" || return 1
        if [ -z "$version" ]; then
            return 1
        fi
    else
        version="$VERSION"
    fi

    local artifact="ironclad-${version}-${platform}"
    archive_url="https://github.com/${GITHUB_REPO}/releases/download/v${version}/${artifact}.tar.gz"
    sums_url="https://github.com/${GITHUB_REPO}/releases/download/v${version}/SHA256SUMS.txt"

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' RETURN

    # Download SHA256SUMS
    info "Fetching checksums..."
    if ! curl -fsSL "$sums_url" -o "${tmpdir}/SHA256SUMS.txt" 2>/dev/null; then
        info "No SHA256SUMS.txt found for v${version}"
        return 1
    fi

    # Extract expected checksum for our artifact
    expected_sum="$(grep "${artifact}.tar.gz" "${tmpdir}/SHA256SUMS.txt" | awk '{print $1}')"
    if [ -z "$expected_sum" ]; then
        info "No checksum entry for ${artifact}.tar.gz"
        return 1
    fi

    # Download archive
    info "Downloading precompiled binary (v${version}, ${platform})..."
    if ! curl -fsSL "$archive_url" -o "${tmpdir}/${artifact}.tar.gz" 2>/dev/null; then
        info "Download failed for ${artifact}.tar.gz"
        return 1
    fi

    # Verify SHA256
    info "Verifying SHA256 checksum..."
    if ! verify_sha256 "${tmpdir}/${artifact}.tar.gz" "$expected_sum"; then
        warn "SHA256 checksum mismatch! Archive may be corrupted or tampered with."
        return 1
    fi
    info "Checksum verified ✓"

    # Extract and install
    tar -xzf "${tmpdir}/${artifact}.tar.gz" -C "${tmpdir}" || return 1

    local binary
    binary="$(find "$tmpdir" -name ironclad -type f 2>/dev/null | head -1)"
    if [ -z "$binary" ] || [ ! -f "$binary" ]; then
        info "Binary not found in archive"
        return 1
    fi

    mkdir -p "$CARGO_BIN"
    cp "$binary" "${CARGO_BIN}/ironclad"
    chmod +x "${CARGO_BIN}/ironclad"

    # Update VERSION so downstream steps know what was installed
    VERSION="$version"
    return 0
}

step "Installing Ironclad"

abort_if_declined "Install Ironclad now?"

# Attempt 1: Precompiled binary download with SHA256 verification
info "Attempting precompiled binary download..."
if try_binary_download; then
    BINARY_INSTALLED=1
    info "Precompiled binary installed ✓"
else
    # Attempt 2: Fall back to cargo install from crates.io
    warn "Binary download unavailable for this platform, falling back to source build"
    step "Building from source (crates.io)"

    if [ -n "$VERSION" ]; then
        INSTALL_CMD="cargo install $CRATE --version $VERSION"
    else
        INSTALL_CMD="cargo install $CRATE"
    fi
    if [ "$INSTALL_LOCKED" != "0" ]; then
        INSTALL_CMD="$INSTALL_CMD --locked"
    fi

    info "This will run:"
    info "  $(bold "$INSTALL_CMD")"
    info ""
    info "Cargo will download, compile, and install the Ironclad binary."
    info "This typically takes 2-5 minutes on the first install."

    $INSTALL_CMD 2>&1 | while IFS= read -r line; do
        case "$line" in
            *Compiling*) printf "\r    Compiling: %-40s" "$(echo "$line" | awk '{print $2}')" ;;
            *Finished*)  printf "\r    %-60s\n" "$line" ;;
            *Installing*) info "$line" ;;
            *warning*) ;;
        esac
    done
fi

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

# ── Optional Apertus bootstrap ───────────────────────────────────────────────

step "Optional local Apertus model setup"
info "Ironclad supports local hosts for Apertus: SGLang (recommended), vLLM,"
info "Docker Model Runner, and Ollama."
if has_existing_local_model_stack; then
    info "Detected an existing local model framework or model cache."
    info "Skipping SGLang + Apertus onboarding recommendation."
    info "Use 'ironclad setup' if you want to switch to Apertus manually."
elif confirm "Install a local Apertus model now?" "y"; then
    RAM_GB="$(detect_total_ram_gb)"
    VRAM_GB="$(detect_gpu_vram_gb)"
    info "Detected RAM: ${RAM_GB} GB"
    if [ "$VRAM_GB" -gt 0 ]; then
        info "Detected GPU VRAM: ${VRAM_GB} GB"
    else
        info "GPU VRAM not detected (CPU-only assumptions will be used)"
    fi

    CAN_USE_8B=0
    CAN_USE_70B=0
    if [ "$RAM_GB" -ge 16 ] || [ "$VRAM_GB" -ge 8 ]; then
        CAN_USE_8B=1
    fi
    if [ "$RAM_GB" -ge 64 ] || [ "$VRAM_GB" -ge 40 ]; then
        CAN_USE_70B=1
    fi

    if [ "$CAN_USE_8B" -ne 1 ] && [ "$CAN_USE_70B" -ne 1 ]; then
        warn "System resources appear below recommended minimum for Apertus local runtime."
        info "Skipping model download. You can configure a lighter model in ironclad setup."
    else
        HOSTS=()
        for h in sglang vllm docker-model-runner ollama; do
            if host_ready "$h"; then
                HOSTS+=("$h")
            fi
        done

        if [ "${#HOSTS[@]}" -eq 0 ]; then
            warn "No supported local host detected."
            if confirm "Install/configure a host now? (SGLang recommended)" "y"; then
                if confirm "Use SGLang (recommended)?" "y"; then
                    attempt_host_install "sglang" || true
                    host_ready "sglang" && HOSTS+=("sglang")
                elif confirm "Use vLLM?" "y"; then
                    attempt_host_install "vllm" || true
                    host_ready "vllm" && HOSTS+=("vllm")
                elif confirm "Use Docker Model Runner?" "y"; then
                    attempt_host_install "docker-model-runner" || true
                    host_ready "docker-model-runner" && HOSTS+=("docker-model-runner")
                elif confirm "Use Ollama?" "y"; then
                    attempt_host_install "ollama" || true
                    host_ready "ollama" && HOSTS+=("ollama")
                fi
            fi
        fi

        if [ "${#HOSTS[@]}" -gt 0 ]; then
            SELECTED_HOST="${HOSTS[0]}"
            info "Recommended host: ${SELECTED_HOST}"
            if [ "${#HOSTS[@]}" -gt 1 ]; then
                if ! confirm "Use recommended host (${SELECTED_HOST})?" "y"; then
                    info "Available hosts: ${HOSTS[*]}"
                    printf "    Enter host name to use: "
                    read -r entered_host </dev/tty
                    for h in "${HOSTS[@]}"; do
                        if [ "$h" = "$entered_host" ]; then
                            SELECTED_HOST="$h"
                            break
                        fi
                    done
                fi
            fi

            SELECTED_MODEL_VARIANT="8b"
            if [ "$CAN_USE_70B" -eq 1 ] && ! confirm "Use Apertus 8B Instruct (recommended default)?" "y"; then
                SELECTED_MODEL_VARIANT="70b"
            fi

            info "Downloading Apertus (${SELECTED_MODEL_VARIANT}) for host ${SELECTED_HOST}..."
            if install_apertus_for_host "$SELECTED_HOST" "$SELECTED_MODEL_VARIANT"; then
                info "Apertus download complete ✓"
            else
                warn "Apertus download failed; continuing installation."
                info "You can retry later with ironclad setup."
            fi
        else
            warn "No local host available after bootstrap attempt; skipping Apertus download."
        fi
    fi
fi

# ── Done ─────────────────────────────────────────────────────────────────────

cat <<DONE

  $(green "✔") $(bold "Ironclad installed successfully!")

  Get started:
    $(bold "ironclad setup")       Interactive configuration wizard
    $(bold "ironclad serve")       Start the agent runtime
    $(bold "ironclad mechanic")    Run diagnostics and self-repair
    $(bold "ironclad --help")      Show all commands

DONE
