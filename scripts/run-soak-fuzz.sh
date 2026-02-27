#!/usr/bin/env bash
set -euo pipefail

SOAK_ROUNDS="${SOAK_ROUNDS:-5}"
FUZZ_SECONDS="${FUZZ_SECONDS:-45}"

echo "Soak/fuzz run starting (rounds=${SOAK_ROUNDS}, fuzz_seconds=${FUZZ_SECONDS})"

echo "1) deterministic soak loops"
for i in $(seq 1 "$SOAK_ROUNDS"); do
  echo "  - soak round ${i}/${SOAK_ROUNDS}"
  cargo test -p ironclad-tests router_falls_through_multiple_blocked_candidates --locked
  cargo test -p ironclad-tests scoped_sessions_remain_isolated_between_peer_and_group --locked
done

echo "2) bounded fuzz targets (ironclad-agent)"
if command -v cargo-fuzz >/dev/null 2>&1; then
  if ! rustup toolchain list | grep -q '^nightly'; then
    echo "nightly Rust toolchain is required for cargo-fuzz sanitizers"
    echo "Install with: rustup toolchain install nightly"
    exit 1
  fi
  (
    cd crates/ironclad-agent/fuzz
    cargo +nightly fuzz run fuzz_check_injection -- -max_total_time="${FUZZ_SECONDS}"
    cargo +nightly fuzz run fuzz_scan_output -- -max_total_time="${FUZZ_SECONDS}"
  )
else
  echo "cargo-fuzz not installed; refusing to skip in life-or-death gate"
  echo "Install with: cargo install cargo-fuzz"
  exit 1
fi

echo "Soak/fuzz battery PASSED"
