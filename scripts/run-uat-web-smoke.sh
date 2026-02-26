#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:18789}"
API_KEY="${API_KEY:-}"

if command -v ghola >/dev/null 2>&1; then
  HTTP_BIN="ghola"
else
  HTTP_BIN="curl"
fi

echo "Web UAT target: ${BASE_URL} (http client: ${HTTP_BIN})"

fetch_to_file() {
  local url="$1"
  local out="$2"
  if [[ "$HTTP_BIN" == "ghola" ]]; then
    if [[ -n "$API_KEY" ]]; then
      ghola -H "Authorization: Bearer ${API_KEY}" -o "$out" "$url" >/dev/null 2>&1 || true
    else
      ghola -o "$out" "$url" >/dev/null 2>&1 || true
    fi
    if [[ ! -s "$out" ]] || rg -q "Ghola Snoop Mode|Snoop End" "$out"; then
      if [[ -n "$API_KEY" ]]; then
        curl -fsS -H "Authorization: Bearer ${API_KEY}" "$url" -o "$out"
      else
        curl -fsS "$url" -o "$out"
      fi
    fi
  else
    if [[ -n "$API_KEY" ]]; then
      curl -fsS -H "Authorization: Bearer ${API_KEY}" "$url" -o "$out"
    else
      curl -fsS "$url" -o "$out"
    fi
  fi
}

echo "1) dashboard shell renders"
fetch_to_file "${BASE_URL}/" /tmp/ironclad-uat-web-dashboard.html
rg -q "Ironclad|dashboard|Context|Sessions" /tmp/ironclad-uat-web-dashboard.html

echo "2) health endpoint"
fetch_to_file "${BASE_URL}/api/health" /tmp/ironclad-uat-web-health.json
jq -e '.status == "ok"' /tmp/ironclad-uat-web-health.json >/dev/null

echo "3) core dashboard APIs"
fetch_to_file "${BASE_URL}/api/agent/status" /tmp/ironclad-uat-web-agent-status.json
fetch_to_file "${BASE_URL}/api/config/status" /tmp/ironclad-uat-web-config-status.json
fetch_to_file "${BASE_URL}/api/config/capabilities" /tmp/ironclad-uat-web-config-capabilities.json
fetch_to_file "${BASE_URL}/api/subagents" /tmp/ironclad-uat-web-subagents.json

jq -e '.state == "running" or .diagnostics' /tmp/ironclad-uat-web-agent-status.json >/dev/null
jq -e '.status' /tmp/ironclad-uat-web-config-status.json >/dev/null
jq -e '.mutable_sections or .notes' /tmp/ironclad-uat-web-config-capabilities.json >/dev/null
jq -e '.agents and (.count >= 0)' /tmp/ironclad-uat-web-subagents.json >/dev/null

echo "Web UAT smoke PASSED"
