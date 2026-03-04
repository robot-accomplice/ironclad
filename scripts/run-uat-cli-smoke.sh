#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:18789}"
API_KEY="${API_KEY:-}"
CLI_BIN="${CLI_BIN:-cargo run --bin ironclad --}"
OUT_DIR="${OUT_DIR:-$(mktemp -d)}"

cleanup() {
  rm -rf "$OUT_DIR"
}
trap cleanup EXIT

echo "CLI UAT target: ${BASE_URL}"

run_cli() {
  local cmd=("$@")
  if [[ -n "$API_KEY" ]]; then
    IRONCLAD_API_KEY="$API_KEY" $CLI_BIN --url "$BASE_URL" "${cmd[@]}"
  else
    $CLI_BIN --url "$BASE_URL" "${cmd[@]}"
  fi
}

run_and_capture() {
  local outfile="$1"
  shift
  if ! run_cli "$@" >"$outfile" 2>&1; then
    echo "CLI smoke step failed: $*"
    echo "----- begin captured output ($outfile) -----"
    cat "$outfile" || true
    echo "----- end captured output ($outfile) -----"
    return 1
  fi
}

run_with_retry() {
  local outfile="$1"
  shift
  local attempts=8
  local sleep_s=2
  local i

  for i in $(seq 1 "$attempts"); do
    if run_cli "$@" >"$outfile" 2>&1; then
      return 0
    fi
    if [[ "$i" -lt "$attempts" ]]; then
      echo "$* failed on attempt ${i}/${attempts}; retrying..."
      sleep "$sleep_s"
    fi
  done

  echo "CLI smoke step failed after retries: $*"
  echo "----- begin captured output ($outfile) -----"
  cat "$outfile" || true
  echo "----- end captured output ($outfile) -----"
  return 1
}

echo "1) sessions list"
SESSIONS_OUT="${OUT_DIR}/sessions.txt"
run_with_retry "$SESSIONS_OUT" sessions list
test -s "$SESSIONS_OUT"

echo "2) skills list"
SKILLS_OUT="${OUT_DIR}/skills.txt"
run_with_retry "$SKILLS_OUT" skills list
test -s "$SKILLS_OUT"

echo "3) subagent list"
SUBAGENTS_OUT="${OUT_DIR}/subagents.txt"
run_with_retry "$SUBAGENTS_OUT" agents list
test -s "$SUBAGENTS_OUT"

echo "CLI UAT smoke PASSED"
