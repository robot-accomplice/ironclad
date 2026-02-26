#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:18789}"
AUTH_HEADER="${API_KEY:-}"

echo "Smoke target: ${BASE_URL}"

curl_json() {
  local method="$1"
  local url="$2"
  local body="${3:-}"
  if [[ -n "$AUTH_HEADER" && -n "$body" ]]; then
    curl -fsS -H "Authorization: Bearer ${AUTH_HEADER}" -X "$method" "$url" -H "Content-Type: application/json" -d "$body"
  elif [[ -n "$AUTH_HEADER" ]]; then
    curl -fsS -H "Authorization: Bearer ${AUTH_HEADER}" -X "$method" "$url"
  elif [[ -n "$body" ]]; then
    curl -fsS -X "$method" "$url" -H "Content-Type: application/json" -d "$body"
  else
    curl -fsS -X "$method" "$url"
  fi
}

echo "1) runtime surfaces"
surfaces="$(curl_json GET "$BASE_URL/api/runtime/surfaces")"
jq -e '.discovery and .devices and .mcp' >/dev/null <<<"$surfaces"

echo "2) discovery register + verify"
curl_json POST "$BASE_URL/api/runtime/discovery" \
  '{"agent_id":"smoke-remote-1","name":"Smoke Remote","url":"http://smoke-remote.local:9000","capabilities":["search","tools"]}' \
  | jq -e '.ok == true' >/dev/null
curl_json POST "$BASE_URL/api/runtime/discovery/smoke-remote-1/verify" \
  | jq -e '.ok == true' >/dev/null
curl_json GET "$BASE_URL/api/runtime/discovery" \
  | jq -e '.agents[] | select(.agent_id=="smoke-remote-1" and .verified==true)' >/dev/null

echo "3) devices pair + verify + unpair"
curl_json POST "$BASE_URL/api/runtime/devices/pair" \
  '{"device_id":"smoke-peer-1","public_key_hex":"04abcdef","device_name":"Smoke Peer"}' \
  | jq -e '.ok == true' >/dev/null
curl_json POST "$BASE_URL/api/runtime/devices/smoke-peer-1/verify" \
  | jq -e '.ok == true' >/dev/null
curl_json DELETE "$BASE_URL/api/runtime/devices/smoke-peer-1" \
  | jq -e '.ok == true' >/dev/null

echo "4) mcp runtime"
curl_json GET "$BASE_URL/api/runtime/mcp" | jq -e '.connections and .exposed_tools and .exposed_resources' >/dev/null

echo "5) cron interval normalization"
job_id="$(
  curl_json POST "$BASE_URL/api/cron/jobs" \
    '{"name":"smoke-interval","agent_id":"smoke","schedule_kind":"interval","schedule_expr":"5m","payload_json":"{\"action\":\"metric_snapshot\"}"}' \
    | jq -r '.job_id'
)"
[[ -n "$job_id" && "$job_id" != "null" ]]
curl_json GET "$BASE_URL/api/cron/jobs/$job_id" | jq -e '.schedule_kind=="every"' >/dev/null
curl_json DELETE "$BASE_URL/api/cron/jobs/$job_id" >/dev/null

echo "6) analyze/recommendation endpoint is not stub"
sid="$(curl_json POST "$BASE_URL/api/sessions" '{"agent_id":"smoke-analysis"}' | jq -r '.session_id')"
curl_json POST "$BASE_URL/api/sessions/$sid/messages" '{"role":"user","content":"summarize test"}' >/dev/null

if [[ -n "$AUTH_HEADER" ]]; then
  session_status="$(curl -s -H "Authorization: Bearer ${AUTH_HEADER}" -o /tmp/ironclad-session-analyze.json -w "%{http_code}" -X POST "$BASE_URL/api/sessions/$sid/analyze")"
  recs_status="$(curl -s -H "Authorization: Bearer ${AUTH_HEADER}" -o /tmp/ironclad-recs-analyze.json -w "%{http_code}" -X POST "$BASE_URL/api/recommendations/generate")"
else
  session_status="$(curl -s -o /tmp/ironclad-session-analyze.json -w "%{http_code}" -X POST "$BASE_URL/api/sessions/$sid/analyze")"
  recs_status="$(curl -s -o /tmp/ironclad-recs-analyze.json -w "%{http_code}" -X POST "$BASE_URL/api/recommendations/generate")"
fi

if [[ "$session_status" == "200" ]]; then
  jq -e '.status=="complete"' /tmp/ironclad-session-analyze.json >/dev/null
elif [[ "$session_status" != "502" && "$session_status" != "503" ]]; then
  echo "Unexpected status from /api/sessions/{id}/analyze: $session_status"
  cat /tmp/ironclad-session-analyze.json
  exit 1
fi

if [[ "$recs_status" == "200" ]]; then
  jq -e '.status=="complete"' /tmp/ironclad-recs-analyze.json >/dev/null
elif [[ "$recs_status" != "502" && "$recs_status" != "503" ]]; then
  echo "Unexpected status from /api/recommendations/generate: $recs_status"
  cat /tmp/ironclad-recs-analyze.json
  exit 1
fi

echo "Smoke run PASSED"
