#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# run-expanded-fuzz.sh
#
# Runs all fuzz targets across the six expanded crates plus the original
# ironclad-agent targets.  Each target runs for FUZZ_SECONDS (default 60).
#
# Prerequisites:
#   - cargo-fuzz:  cargo install cargo-fuzz
#   - nightly Rust: rustup toolchain install nightly
#
# Usage:
#   ./scripts/run-expanded-fuzz.sh              # 60 s per target
#   FUZZ_SECONDS=120 ./scripts/run-expanded-fuzz.sh
# ---------------------------------------------------------------------------

FUZZ_SECONDS="${FUZZ_SECONDS:-60}"

echo "=== Expanded fuzz run (${FUZZ_SECONDS}s per target) ==="
echo ""

if ! command -v cargo-fuzz >/dev/null 2>&1; then
    echo "ERROR: cargo-fuzz not installed"
    echo "Install with: cargo install cargo-fuzz"
    exit 1
fi

if ! rustup toolchain list | grep -q '^nightly'; then
    echo "ERROR: nightly Rust toolchain required for cargo-fuzz sanitizers"
    echo "Install with: rustup toolchain install nightly"
    exit 1
fi

PASSED=0
FAILED=0
FAILED_TARGETS=()

run_fuzz_dir() {
    local crate_dir="$1"
    shift
    local targets=("$@")

    local crate_name
    crate_name="$(basename "$crate_dir")"

    for target in "${targets[@]}"; do
        echo "--- ${crate_name} / ${target} (${FUZZ_SECONDS}s) ---"
        if (cd "${crate_dir}/fuzz" && cargo +nightly fuzz run "$target" -- -max_total_time="${FUZZ_SECONDS}"); then
            PASSED=$((PASSED + 1))
        else
            echo "FAIL: ${crate_name}/${target}"
            FAILED=$((FAILED + 1))
            FAILED_TARGETS+=("${crate_name}/${target}")
        fi
        echo ""
    done
}

# --- ironclad-agent (existing) ---
run_fuzz_dir crates/ironclad-agent \
    fuzz_check_injection \
    fuzz_scan_output

# --- ironclad-core ---
run_fuzz_dir crates/ironclad-core \
    fuzz_config_parse \
    fuzz_config_validate

# --- ironclad-db ---
run_fuzz_dir crates/ironclad-db \
    fuzz_derive_nickname \
    fuzz_session_status_parse \
    fuzz_message_role_parse

# --- ironclad-llm ---
run_fuzz_dir crates/ironclad-llm \
    fuzz_parse_sse_chunk

# --- ironclad-channels ---
run_fuzz_dir crates/ironclad-channels \
    fuzz_sanitize_platform \
    fuzz_parse_ws_message \
    fuzz_telegram_parse_inbound \
    fuzz_whatsapp_parse_inbound \
    fuzz_signal_parse_inbound

# --- ironclad-plugin-sdk ---
run_fuzz_dir crates/ironclad-plugin-sdk \
    fuzz_manifest_parse \
    fuzz_manifest_validate

# --- ironclad-schedule ---
run_fuzz_dir crates/ironclad-schedule \
    fuzz_evaluate_cron \
    fuzz_evaluate_at \
    fuzz_calculate_next_run

# --- Summary ---
TOTAL=$((PASSED + FAILED))
echo "==========================================="
echo "  Fuzz summary: ${PASSED}/${TOTAL} passed"
if [ "$FAILED" -gt 0 ]; then
    echo "  FAILED targets:"
    for t in "${FAILED_TARGETS[@]}"; do
        echo "    - $t"
    done
    echo "==========================================="
    exit 1
fi
echo "==========================================="
echo "All ${TOTAL} fuzz targets PASSED"
