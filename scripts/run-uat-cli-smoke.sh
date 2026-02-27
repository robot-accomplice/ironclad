#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:18789}"
API_KEY="${API_KEY:-}"
CLI_BIN="${CLI_BIN:-cargo run --bin ironclad --}"

echo "CLI UAT target: ${BASE_URL}"

run_cli() {
  local cmd=("$@")
  if [[ -n "$API_KEY" ]]; then
    IRONCLAD_API_KEY="$API_KEY" $CLI_BIN --url "$BASE_URL" "${cmd[@]}"
  else
    $CLI_BIN --url "$BASE_URL" "${cmd[@]}"
  fi
}

echo "1) status"
run_cli status >/tmp/ironclad-uat-cli-status.txt 2>&1
grep -qiE "status|online|uptime|running" /tmp/ironclad-uat-cli-status.txt

echo "2) sessions list"
run_cli sessions list >/tmp/ironclad-uat-cli-sessions.txt 2>&1
test -s /tmp/ironclad-uat-cli-sessions.txt

echo "3) config show + cache metrics"
run_cli config show >/tmp/ironclad-uat-cli-config-show.txt 2>&1
run_cli metrics cache >/tmp/ironclad-uat-cli-cache-metrics.txt 2>&1
grep -qiE "agent|server|models|config" /tmp/ironclad-uat-cli-config-show.txt
grep -qiE "cache|hit|miss" /tmp/ironclad-uat-cli-cache-metrics.txt

echo "4) subagent list"
run_cli agents list >/tmp/ironclad-uat-cli-subagents.txt 2>&1
test -s /tmp/ironclad-uat-cli-subagents.txt

echo "CLI UAT smoke PASSED"
