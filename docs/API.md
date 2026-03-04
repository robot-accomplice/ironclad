# API Reference

Ironclad exposes a REST API on the configured `[server]` bind address and port (default: `http://127.0.0.1:18789`).

## Authentication

When `[server] api_key` is set, all API requests (except `GET /api/health`) must include an `x-api-key` header:

```
x-api-key: your-secret-key
```

Requests without a valid key return `401 Unauthorized`.

## Request/Response Format

- Request bodies: `application/json`
- Response bodies: `application/json`
- Maximum request body size: 1 MB

---

## Health

### `GET /api/health`

Returns server health status.

**Response:**

```json
{
  "status": "ok",
  "version": "0.6.0",
  "agent": "Roboticus",
  "uptime_seconds": 3600,
  "models": {
    "primary": "ollama/qwen3:8b",
    "current": "ollama/qwen3:8b",
    "fallbacks": []
  }
}
```

### `GET /api/logs`

Retrieve structured log entries.

**Query Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `lines` | `usize` | `100` | Number of log lines to return (max 10,000) |
| `level` | `String` | — | Filter by level: `trace`, `debug`, `info`, `warn`, `error` |

**Response:**

```json
{
  "entries": [
    {
      "timestamp": "2026-02-23T00:00:00Z",
      "level": "info",
      "message": "Server started",
      "target": "ironclad"
    }
  ]
}
```

---

## Configuration

### `GET /api/config`

Returns the running configuration (secrets redacted).

**Response:** The full `IroncladConfig` serialized as JSON, with `api_key` fields omitted.

### `PUT /api/config`

Update runtime configuration. Some sections are immutable at runtime.

**Request Body:** Partial config JSON with sections to update.

```json
{
  "agent": { "name": "NewName" },
  "models": { "primary": "anthropic/claude-opus-4" }
}
```

**Immutable sections** (return `403 Forbidden`): `server`, `wallet`, `treasury`, `a2a`.

**Response:**

```json
{ "updated": true }
```

**Error Codes:**

| Status | Condition |
|--------|-----------|
| `403` | Attempt to modify immutable section |
| `400` | Invalid configuration values |

### `PUT /api/providers/{name}/key`

Set or update the API key for a provider.

**Request Body:**

```json
{ "api_key": "sk-..." }
```

### `DELETE /api/providers/{name}/key`

Remove the API key for a provider.

---

## Agent

### `GET /api/agent/status`

Returns the agent's current operational state.

**Response:**

```json
{
  "state": "running",
  "name": "Roboticus",
  "id": "roboticus",
  "model": "ollama/qwen3:8b",
  "primary_provider_state": "closed",
  "uptime_seconds": 3600
}
```

The `primary_provider_state` reflects the circuit breaker state: `closed` (healthy), `open` (tripped), or `half_open` (testing recovery).

Diagnostics include `taskable_subagents_*` counts. These metrics explicitly exclude `model-proxy` records, which are routing proxies and not independently taskable subagents.

### `POST /api/agent/message`

Send a message to the agent and receive a response.

**Request Body:**

```json
{
  "content": "What is the weather today?",
  "session_id": "optional-session-id",
  "channel": "web",
  "sender_id": "user-123",
  "peer_id": "optional-peer-id",
  "group_id": "optional-group-id",
  "is_group": false
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | `String` | Yes | Message text |
| `session_id` | `String` | No | Existing session ID (auto-created if omitted) |
| `channel` | `String` | No | Source channel identifier |
| `sender_id` | `String` | No | Sender identifier |
| `peer_id` | `String` | No | Explicit peer scope key for session auto-create |
| `group_id` | `String` | No | Explicit group scope key for session auto-create |
| `is_group` | `bool` | No | Hint for scope resolver when `scope_mode` is group-aware |

**Response:**

```json
{
  "content": "I don't have access to real-time weather data...",
  "session_id": "abc-123",
  "user_message_id": "msg-001",
  "assistant_message_id": "msg-002",
  "selected_model": "moonshot/kimi-k2-turbo-preview",
  "model": "ollama/qwen3:8b",
  "model_shift_from": "moonshot/kimi-k2-turbo-preview",
  "cached": false,
  "tokens_in": 150,
  "tokens_out": 85,
  "cost": 0.0,
  "tools_used": []
}
```

`selected_model` is the router's chosen model before execution. `model` is the model that actually produced the response. `model_shift_from` is `null` when no shift happened.

When a cached response is returned, `cached: true` and `tokens_saved` is included.

**Error Codes:**

| Status | Condition |
|--------|-----------|
| `403` | Message blocked by injection detection (returns `error: "message_blocked"` and `threat_score`) |

### `POST /api/agent/message/stream`

Send a message and receive a streamed SSE response. Same request body as `POST /api/agent/message`.

**Response:** Server-Sent Events stream with `data:` frames containing JSON chunks.

---

## Sessions

### `GET /api/sessions`

List all sessions.

**Response:**

```json
{
  "sessions": [
    {
      "id": "abc-123",
      "agent_id": "roboticus",
      "scope_key": "agent",
      "status": "active",
      "model": "ollama/qwen3:8b",
      "nickname": "Weather Chat",
      "created_at": "2026-02-23T10:00:00Z",
      "updated_at": "2026-02-23T11:30:00Z",
      "metadata": null
    }
  ]
}
```

### `POST /api/sessions`

Create a new session.

**Request Body:**

```json
{ "agent_id": "roboticus" }
```

**Response:**

```json
{ "session_id": "abc-123" }
```

### `GET /api/sessions/{id}`

Get session details.

**Response:**

```json
{
  "id": "abc-123",
  "agent_id": "roboticus",
  "scope_key": null,
  "status": "active",
  "model": "ollama/qwen3:8b",
  "nickname": "Weather Chat",
  "created_at": "2026-02-23T10:00:00Z",
  "updated_at": "2026-02-23T11:30:00Z",
  "metadata": null
}
```

**Error Codes:** `404` if session not found.

### `GET /api/sessions/{id}/messages`

List messages in a session.

**Response:**

```json
{
  "messages": [
    {
      "id": "msg-001",
      "role": "user",
      "content": "Hello",
      "created_at": "2026-02-23T10:00:00Z"
    }
  ]
}
```

### `POST /api/sessions/{id}/messages`

Post a raw message to a session (does not trigger agent inference).

**Request Body:**

```json
{
  "role": "user",
  "content": "Hello"
}
```

### `GET /api/sessions/{id}/turns`

List conversation turns (paired user + assistant messages with metadata).

### `GET /api/sessions/{id}/insights`

Get session-level insights and analytics.

### `POST /api/sessions/{id}/analyze`

Trigger deep analysis of a session.

### `GET /api/sessions/{id}/feedback`

Get feedback entries for a session.

### `POST /api/sessions/backfill-nicknames`

Generate nicknames for all sessions missing one.

---

## Turns

### `GET /api/turns/{id}`

Get a single conversation turn.

### `GET /api/turns/{id}/context`

Get the context window state at the time of a turn.

### `GET /api/turns/{id}/model-selection`

Get per-task model-selection forensics for a turn.

**Response includes:**
- selected model
- strategy used
- candidate-by-candidate usability checks (provider availability, breaker state, usability reason)
- complexity label and task excerpt

### `GET /api/turns/{id}/tools`

Get tool calls executed during a turn.

### `GET /api/turns/{id}/tips`

Get improvement tips for a turn.

### `POST /api/turns/{id}/analyze`

Trigger analysis of a specific turn.

### `GET /api/turns/{id}/feedback`

Get feedback for a turn.

### `POST /api/turns/{id}/feedback`

Submit feedback for a turn.

**Request Body:**

```json
{
  "grade": 5,
  "comment": "Great response"
}
```

### `PUT /api/turns/{id}/feedback`

Update existing feedback for a turn.

---

## Memory

### `GET /api/memory/working`

List all working memory entries across sessions.

### `GET /api/memory/working/{session_id}`

List working memory entries for a specific session.

**Response:**

```json
{
  "entries": [
    {
      "id": 1,
      "classification": "fact",
      "content": "user prefers dark mode",
      "salience": 5,
      "created_at": "2026-02-23T10:00:00Z"
    }
  ]
}
```

### `GET /api/memory/episodic`

List episodic memory entries.

**Query Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `limit` | `i64` | Maximum entries to return |

**Response:**

```json
{
  "entries": [
    {
      "id": 1,
      "classification": "tool_use",
      "content": "ran a shell command",
      "salience": 5,
      "created_at": "2026-02-23T10:00:00Z"
    }
  ]
}
```

### `GET /api/memory/semantic`

List all semantic memory entries.

### `GET /api/memory/semantic/categories`

List semantic memory categories.

### `GET /api/memory/semantic/{category}`

List semantic memory entries in a category.

**Response:**

```json
{
  "entries": [
    {
      "id": 1,
      "key": "color",
      "value": "blue",
      "confidence": 0.9,
      "created_at": "2026-02-23T10:00:00Z"
    }
  ]
}
```

### `GET /api/memory/search`

Search across all memory tiers.

**Query Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `q` | `String` | Yes | Search query |
| `limit` | `i64` | No | Maximum results |

**Response:**

```json
{
  "results": [
    {
      "tier": "semantic",
      "content": "user prefers dark mode",
      "score": 0.85
    }
  ]
}
```

**Error Codes:** `400` if `q` parameter is missing.

---

## Cron Jobs

### `GET /api/cron/jobs`

List all scheduled jobs.

**Response:**

```json
{
  "jobs": [
    {
      "id": "job-123",
      "name": "heartbeat",
      "agent_id": "roboticus",
      "schedule_kind": "interval",
      "schedule_expr": "1h",
      "enabled": true
    }
  ]
}
```

### `POST /api/cron/jobs`

Create a new scheduled job.

**Request Body:**

```json
{
  "name": "daily-digest",
  "agent_id": "roboticus",
  "schedule_kind": "cron",
  "schedule_expr": "0 9 * * *"
}
```

**Response:**

```json
{ "job_id": "job-456" }
```

### `GET /api/cron/jobs/{id}`

Get job details.

**Error Codes:** `404` if not found.

### `PUT /api/cron/jobs/{id}`

Update a scheduled job.

### `DELETE /api/cron/jobs/{id}`

Delete a scheduled job.

**Response:**

```json
{ "deleted": true, "id": "job-456" }
```

**Error Codes:** `404` if not found.

---

## Statistics

### `GET /api/stats/costs`

Get inference cost breakdown by model and provider.

**Response:**

```json
{
  "costs": [
    {
      "model": "ollama/qwen3:8b",
      "provider": "ollama",
      "tokens_in": 1500,
      "tokens_out": 800,
      "cost": 0.0,
      "session": "default"
    }
  ]
}
```

### `GET /api/stats/efficiency`

Get model efficiency metrics.

### `GET /api/stats/transactions`

Get transaction history.

**Query Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `hours` | `i64` | Time window in hours |

**Response:**

```json
{
  "transactions": []
}
```

### `GET /api/stats/cache`

Get semantic cache statistics.

**Response:**

```json
{
  "hits": 42,
  "misses": 158,
  "entries": 200,
  "hit_rate": 0.21
}
```

### `GET /api/stats/capacity`

Get per-provider capacity/headroom telemetry used by routing decisions.

**Response:**

```json
{
  "providers": {
    "ollama": {
      "headroom": 0.92,
      "near_capacity": false,
      "sustained_hot": false,
      "tokens_used": 14321,
      "requests_used": 21,
      "tpm_limit": 200000,
      "rpm_limit": 120,
      "token_utilization": 0.07,
      "request_utilization": 0.18
    }
  }
}
```

### `GET /api/models/selections`

List recent model-selection forensic events (newest first).

**Query Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `limit` | `usize` | `100` | Maximum events to return (1-500) |

### `GET /api/models/routing-diagnostics`

Returns routing configuration and model-profile diagnostics used for operator analysis.

### `GET /api/models/routing-dataset`

Returns joined routing + cost rows.

`user_excerpt` is redacted by default in JSON responses. Use `include_user_excerpt=true` to opt in.

### `POST /api/models/routing-eval`

Runs offline replay against routing dataset rows.

Validation:
- `cost_weight` must be in `[0.0, 1.0]`
- `accuracy_floor` must be in `[0.0, 1.0]`
- `accuracy_min_obs` must be `>= 1`

### WebSocket Routing Events

- `model_selection`: emitted when routing selects a model candidate set for a turn.
- `model_shift`: emitted when execution model differs from selected model (fallback/cache continuity events).

---

## Circuit Breaker

### `GET /api/breaker/status`

Get circuit breaker state for all providers.

**Response:**

```json
{
  "providers": {
    "ollama": { "state": "closed", "failures": 0 },
    "openai": { "state": "open", "failures": 3 }
  },
  "config": {
    "threshold": 3,
    "window_seconds": 60
  }
}
```

### `POST /api/breaker/reset/{provider}`

Reset a tripped circuit breaker for a specific provider.

**Response:**

```json
{
  "provider": "openai",
  "state": "closed",
  "reset": true
}
```

---

## Wallet

### `GET /api/wallet/balance`

Get wallet balance and treasury policy.

**Response:**

```json
{
  "balance": "150.00",
  "currency": "USDC",
  "address": "0x...",
  "chain_id": 8453,
  "treasury": {
    "per_payment_cap": 100.0,
    "hourly_transfer_limit": 500.0,
    "daily_transfer_limit": 2000.0,
    "minimum_reserve": 5.0
  }
}
```

### `GET /api/wallet/address`

Get the wallet's on-chain address.

**Response:**

```json
{
  "address": "0x1234...abcd",
  "chain_id": 8453
}
```

---

## Skills

### `GET /api/skills`

List all registered skills.

**Response:**

```json
{
  "skills": [
    {
      "id": "skill-123",
      "name": "web-search",
      "kind": "tool",
      "description": "Search the web",
      "enabled": true
    }
  ]
}
```

### `GET /api/skills/{id}`

Get skill details.

**Response:**

```json
{
  "id": "skill-123",
  "name": "web-search",
  "kind": "tool",
  "description": "Search the web",
  "enabled": true,
  "path": "/home/user/.ironclad/skills/web-search.toml"
}
```

**Error Codes:** `404` if not found.

### `POST /api/skills/reload`

Reload skills from disk.

**Response:**

```json
{ "reloaded": true }
```

### `PUT /api/skills/{id}/toggle`

Toggle a skill's enabled state.

**Response:**

```json
{
  "id": "skill-123",
  "enabled": false
}
```

**Error Codes:** `404` if not found.

---

## Plugins

### `GET /api/plugins`

List installed plugins.

**Response:**

```json
{
  "plugins": [
    {
      "name": "my-plugin",
      "version": "1.0.0",
      "enabled": true,
      "tools": ["greet", "search"]
    }
  ]
}
```

### `PUT /api/plugins/{name}/toggle`

Toggle a plugin's enabled state.

**Error Codes:** `404` if not found.

### `POST /api/plugins/{name}/execute/{tool}`

Execute a plugin tool. Subject to policy engine evaluation.

**Request Body:** Tool-specific JSON parameters.

**Response:**

```json
{
  "result": {
    "success": true,
    "output": "Hello, World!",
    "metadata": null
  }
}
```

**Error Codes:**

| Status | Condition |
|--------|-----------|
| `404` | Plugin or tool not found |
| `403` | Denied by policy engine |

---

## Browser

### `GET /api/browser/status`

Get headless browser status.

### `POST /api/browser/start`

Start the headless browser.

### `POST /api/browser/stop`

Stop the headless browser.

### `POST /api/browser/action`

Execute a browser action (navigate, click, type, screenshot, etc.).

---

## Agents

### `GET /api/agents`

List all agents in a multi-agent setup.

**Response:**

```json
{
  "agents": [
    { "id": "roboticus", "name": "Roboticus", "status": "running" }
  ]
}
```

### `POST /api/agents/{id}/start`

Start an agent.

**Error Codes:** `404` if agent not found.

### `POST /api/agents/{id}/stop`

Stop an agent.

**Error Codes:** `404` if agent not found.

---

## Subagents

Ubiquitous language:
- A **subagent** is independently taskable, has a fixed skill set, is personality-free, and is orchestrated by the primary agent.
- A **model-proxy** is not a subagent. It represents model-routing indirection and is excluded from taskable subagent counts.

### `GET /api/subagents`

List all subagents.

**Response:**

```json
{
  "agents": [],
  "count": 0
}
```

### `POST /api/subagents`

Create a new subagent.

**Request Body:**

```json
{
  "name": "research-subagent",
  "model": "openai/gpt-4o",
  "role": "subagent",
  "skills": ["research", "summarization"]
}
```

`role` must be either `subagent` or `model-proxy` (legacy `specialist` is normalized to `subagent`).

`model` supports:
- a concrete provider/model string (fixed),
- `auto` (Ironclad chooses per assignment),
- `orchestrator` (primary agent chooses per assignment).

Validation rules:
- `personality` payloads are rejected (`400`) for all subagent records.
- `model-proxy` records cannot own skills.
- `model-proxy` records cannot use `auto` or `orchestrator`; they require a concrete provider/model.
- `subagent` records store a fixed skills list (`skills`) on the subagent record.

**Response:**

```json
{
  "created": true,
  "name": "research-subagent"
}
```

### `PUT /api/subagents/{name}`

Update a subagent's configuration.

Supports updating `display_name`, `model`, `role`, `description`, `skills`, and `enabled`, with the same validation rules as create.

### `DELETE /api/subagents/{name}`

Delete a subagent.

**Error Codes:** `404` if not found.

### `PUT /api/subagents/{name}/toggle`

Toggle a subagent's enabled state.

**Error Codes:** `404` if not found.

---

## Roster

### `GET /api/roster`

Get the orchestrated roster (orchestrator + taskable subagents) with fixed per-subagent skills and model assignments.

**Response:**

```json
{
  "roster": [
    {
      "name": "Roboticus",
      "role": "orchestrator",
      "model": "ollama/qwen3:8b",
      "skills": [],
      "capabilities": ["orchestrate-subagents", "assign-tasks", "select-subagent-model"]
    },
    {
      "name": "research-subagent",
      "role": "subagent",
      "model": "openai/gpt-4o-mini",
      "skills": ["research", "summarization"]
    }
  ],
  "taskable_subagent_count": 1,
  "model_proxy_count": 2,
  "model_proxies": [
    { "name": "cloud-fallback", "role": "model-proxy", "model": "openrouter/openai/gpt-4o-mini" }
  ]
}
```

### `PUT /api/roster/{name}/model`

Change the model for an agent in the roster.

**Request Body:**

```json
{ "model": "anthropic/claude-opus-4" }
```

For subagents, `model` may also be `auto` or `orchestrator`.

**Response:**

```json
{
  "updated": true,
  "old_model": "ollama/qwen3:8b",
  "new_model": "anthropic/claude-opus-4"
}
```

**Error Codes:**

| Status | Condition |
|--------|-----------|
| `400` | Empty model string |
| `404` | Agent not found in roster |

---

## Workspace

### `GET /api/workspace/state`

Get the workspace state graph, agent activity, and a top-level workspace file snapshot.

**Response:**

```json
{
  "agents": [],
  "systems": [],
  "files": {
    "workspace_root": "/path/to/workspace",
    "top_level_entries": [
      { "name": "docs", "kind": "dir" },
      { "name": "README.md", "kind": "file" }
    ],
    "entry_count": 2
  },
  "interactions": []
}
```

---

## Recommendations

### `GET /api/recommendations`

Get optimization recommendations.

### `POST /api/recommendations/generate`

Generate deep analysis recommendations.

---

## A2A Protocol

### `POST /api/a2a/hello`

Agent-to-Agent handshake endpoint.

**Request Body:**

```json
{
  "type": "a2a_hello",
  "did": "did:ironclad:peer-123",
  "nonce": "deadbeef01020304",
  "timestamp": 1708732800
}
```

**Response:**

```json
{
  "protocol": "a2a",
  "version": "0.1",
  "status": "ok",
  "peer_did": "did:ironclad:peer-123",
  "hello": {
    "did": "did:ironclad:self-456",
    "nonce": "..."
  }
}
```

**Error Codes:** `400` if payload is invalid (wrong type, missing fields).

### `GET /.well-known/agent.json`

Returns the agent's A2A discovery card per the Google A2A spec.

**Response:**

```json
{
  "name": "Roboticus",
  "version": "0.4.3",
  "capabilities": ["chat", "tools"]
}
```

---

## Channels

### `GET /api/channels/status`

Get status for all channel adapters.

**Response:** Array of channel status objects.

---

## Webhooks

### `POST /api/webhooks/telegram`

Telegram bot webhook endpoint. Requires `X-Telegram-Bot-Api-Secret-Token` header when `webhook_secret` is configured.

**Error Codes:**

| Status | Condition |
|--------|-----------|
| `401` | Missing or invalid webhook secret |
| `503` | Telegram adapter not configured |

### `GET /api/webhooks/whatsapp`

WhatsApp webhook verification endpoint (for Meta's challenge flow).

**Query Parameters:** `hub.mode`, `hub.verify_token`, `hub.challenge`.

**Error Codes:**

| Status | Condition |
|--------|-----------|
| `403` | Invalid verify token |
| `503` | WhatsApp adapter not configured |

### `POST /api/webhooks/whatsapp`

WhatsApp message webhook. Validates `X-Hub-Signature-256` when `app_secret` is configured.

**Error Codes:**

| Status | Condition |
|--------|-----------|
| `401` | Invalid HMAC signature |
| `503` | WhatsApp adapter not configured |

---

## Approvals

### `GET /api/approvals`

List pending approval requests.

### `POST /api/approvals/{id}/approve`

Approve a pending tool execution request.

### `POST /api/approvals/{id}/deny`

Deny a pending tool execution request.

---

## Interview

### `POST /api/interview/start`

Start a personality interview session.

### `POST /api/interview/turn`

Submit a turn in the interview conversation.

### `POST /api/interview/finish`

Finish the interview and generate personality files.

---

## Audit

### `GET /api/audit/policy/{turn_id}`

Get the policy audit trail for a specific turn.

### `GET /api/audit/tools/{turn_id}`

Get the tool execution audit trail for a specific turn.

---

## Dashboard

### `GET /`

Serves the single-page web dashboard application.
