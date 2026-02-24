# Integration Smoke Runbook

Use this runbook to verify newly wired runtime surfaces and non-stub endpoints on a local server.

## Prerequisites

- Server running locally (default: `http://127.0.0.1:8787`)
- `jq` installed
- If API key is enabled, set `API_KEY` and keep the `AUTH` variable below

```bash
BASE_URL="${BASE_URL:-http://127.0.0.1:8787}"
AUTH=()
if [ -n "${API_KEY:-}" ]; then AUTH=(-H "Authorization: Bearer ${API_KEY}"); fi
```

## 1) Runtime Surfaces

```bash
curl -s "${AUTH[@]}" "$BASE_URL/api/runtime/surfaces" | jq
curl -s "${AUTH[@]}" "$BASE_URL/api/runtime/discovery" | jq
curl -s "${AUTH[@]}" "$BASE_URL/api/runtime/devices" | jq
curl -s "${AUTH[@]}" "$BASE_URL/api/runtime/mcp" | jq
```

Pass criteria:

- All requests return HTTP 200
- `runtime/surfaces` includes `discovery`, `devices`, and `mcp` objects

## 2) Discovery Write Path

```bash
curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/runtime/discovery" \
  -H "Content-Type: application/json" \
  -d '{"agent_id":"smoke-remote-1","name":"Smoke Remote","url":"http://smoke-remote.local:9000","capabilities":["search","tools"]}' | jq

curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/runtime/discovery/smoke-remote-1/verify" | jq
curl -s "${AUTH[@]}" "$BASE_URL/api/runtime/discovery" | jq
```

Pass criteria:

- Register + verify return `ok: true`
- Agent appears as `verified: true` in listing

## 3) Device Pairing Path

```bash
curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/runtime/devices/pair" \
  -H "Content-Type: application/json" \
  -d '{"device_id":"smoke-peer-1","public_key_hex":"04abcdef","device_name":"Smoke Peer"}' | jq

curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/runtime/devices/smoke-peer-1/verify" | jq
curl -s "${AUTH[@]}" "$BASE_URL/api/runtime/devices" | jq
curl -s "${AUTH[@]}" -X DELETE "$BASE_URL/api/runtime/devices/smoke-peer-1" | jq
```

Pass criteria:

- Pair + verify + unpair all return `ok: true`

## 4) MCP Client Runtime Path

```bash
curl -s "${AUTH[@]}" "$BASE_URL/api/runtime/mcp" | jq
curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/runtime/mcp/clients/missing/discover" | jq
```

Pass criteria:

- Runtime listing returns 200 with `connections` array
- Discovering unknown client returns HTTP 404 with JSON error

## 5) Cron Normalization Path

```bash
JOB_ID=$(curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/cron/jobs" \
  -H "Content-Type: application/json" \
  -d '{"name":"smoke-interval","agent_id":"smoke","schedule_kind":"interval","schedule_expr":"5m","payload_json":"{\"action\":\"metric_snapshot\"}"}' | jq -r '.job_id')

curl -s "${AUTH[@]}" "$BASE_URL/api/cron/jobs/$JOB_ID" | jq
curl -s "${AUTH[@]}" -X DELETE "$BASE_URL/api/cron/jobs/$JOB_ID" | jq
```

Pass criteria:

- Created job exists
- `schedule_kind` is normalized to `every`

## 6) Analyze Endpoints (No Stub Contract)

```bash
SID=$(curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/sessions" -H "Content-Type: application/json" -d '{"agent_id":"smoke-analysis"}' | jq -r '.session_id')
curl -s "${AUTH[@]}" -X POST "$BASE_URL/api/sessions/$SID/messages" -H "Content-Type: application/json" -d '{"role":"user","content":"summarize test"}' | jq

TURN_ID=$(curl -s "${AUTH[@]}" "$BASE_URL/api/sessions/$SID/turns" | jq -r '.turns[0].id')
curl -i -s "${AUTH[@]}" -X POST "$BASE_URL/api/turns/$TURN_ID/analyze" | sed -n '1,20p'
curl -i -s "${AUTH[@]}" -X POST "$BASE_URL/api/sessions/$SID/analyze" | sed -n '1,20p'
curl -i -s "${AUTH[@]}" -X POST "$BASE_URL/api/recommendations/generate" | sed -n '1,20p'
```

Pass criteria:

- Endpoints no longer return `"status":"stub"`
- Acceptable results:
  - `200` with `"status":"complete"` and analysis payload
  - `502`/`503` when provider is unavailable (still non-stub concrete execution path)
