# Integration Test Matrix

This matrix enumerates known end-to-end integration paths in Ironclad and marks where behavior is fully implemented versus partial/stubbed.

Regression governance companion docs:
- `docs/testing/regression-matrix.md`
- `docs/testing/regression-policy.md`
- Release/CI battery commands:
  - `just test-regression` (focused deterministic subset)
  - `just test-v080-go-live` (full v0.8.0 zero-regression gate)

Status key:
- `READY` — path is wired and testable end-to-end.
- `PARTIAL` — path exists but has meaningful gaps or reduced behavior.
- `STUB` — endpoint/path intentionally returns placeholder behavior.

## A) Core Runtime Entry Paths

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| A1 | Web chat request/response | `POST /api/agent/message` | Validate -> scope session -> retrieve context -> route model -> infer -> persist -> return JSON | READY | Turn-level persistence still depends on specific route flow; verify tool-call linkage per turn |
| A2 | Streaming chat (SSE) | `POST /api/agent/message/stream` | Same core pipeline as A1 plus chunked SSE + final persisted assistant message | READY | Requires SSE client handling; verify parity with non-stream cost/metrics |
| A3 | WebSocket dashboard events | `GET /ws` | Subscribe to EventBus, receive status/stream events | READY | With API key enabled, client auth/token handling can cause connection failures |
| A4 | Health + logs | `GET /api/health`, `GET /api/logs` | Return runtime health and structured logs | READY | None significant |

## B) Channel Ingress/Egress Dataflows

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| B1 | Telegram webhook ingress | `POST /api/webhooks/telegram` | Parse inbound -> addressability/injection -> scope session -> infer -> send reply | READY | Verify chat/group metadata mapping in mixed chat types |
| B2 | Telegram poll ingress | Poll loop bootstrap path | Receive update -> same processing as B1 | READY | Poll mode disabled when webhook mode active |
| B3 | WhatsApp webhook ingress | `GET/POST /api/webhooks/whatsapp` | Verify + parse inbound -> infer -> send reply | READY | Ensure signature verification configured in production |
| B4 | Discord outbound messaging | Channel send via router | Outbound send/reply/typing works | READY | Inbound gateway loop not fully wired as default runtime path |
| B5 | Signal outbound messaging | Channel send via router | Outbound send/typing path works | READY | Continuous inbound listener wiring is partial |
| B6 | Channel retry queue | Background drain loop + channel router | Failed sends are retried/backed off | PARTIAL | Queue is in-memory; restart drops pending retries |

## C) Session / Scope / Lifecycle

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| C1 | Session auto-create with scope | Web + channel message paths | Use `agent/peer/group` scope, enforce one active scoped session | READY | Cross-channel canonical identity linking beyond scope key remains limited |
| C2 | Session backfill + uniqueness migration | `012_session_scope_backfill_unique.sql` | Null scope rows backfilled, duplicate active rows archived, unique index enforced | READY | Existing dirty datasets may still require operator review before migration |
| C3 | TTL expiration | Heartbeat `SessionGovernor` | Expire stale sessions via configured TTL | READY | Verify uptime/heartbeat health in long-running environments |
| C4 | Scheduled session rotation | `session.reset_schedule` + heartbeat | Rotate agent-scope sessions on schedule boundary | PARTIAL | Current behavior checks hourly boundary; does not fully parse arbitrary cron expression |
| C5 | Compaction-on-expire draft | Governor compaction path | Draft summary added before status transition | PARTIAL | Uses prompt draft text, not full LLM summarization pass |

## D) LLM Routing / Capacity / Breakers

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| D1 | Complexity routing | Router selection in message paths | Select model based on complexity + local/cloud characteristics | READY | Validate behavior under mixed provider costs/tiers |
| D2 | Capacity-aware routing | `CapacityTracker` + router scoring | Record usage, compute headroom, bias selection away from saturation | READY | Sustained-pressure thresholds are heuristic and should be tuned with traffic |
| D3 | Preemptive breaker pressure | Capacity -> breaker pressure signal | Mark providers soft `half_open` under sustained load | READY | Operational semantics should be monitored to avoid oscillation |
| D4 | Breaker status/reset ops | `GET /api/breaker/status`, `POST /api/breaker/reset/{provider}` | Observe and reset breaker state | READY | Requires auth in secured mode |
| D5 | Capacity telemetry API/UI | `GET /api/stats/capacity` + dashboard metrics | Display headroom/utilization by provider | READY | Ensure dashboard refresh cadence matches operator expectations |

## E) Memory / Retrieval / Context

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| E1 | Retrieval-augmented context assembly | Message inference path + retriever | Memory retrieval contributes to prompt context budget | READY | Quality depends on embedding provider availability |
| E2 | Post-turn memory ingestion | Background ingestion after responses | Store episodic/semantic/procedural/working memory and embeddings | READY | Background failures can degrade recall silently unless monitored |
| E3 | Context explorer data APIs | `/api/sessions/{id}/turns`, `/api/turns/{id}/context`, etc. | Visualize per-turn context and tool info | READY | Analyze endpoints are still stubbed (see G1/G2) |

## F) Tools / Browser / Plugins

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| F1 | Tool policy + approval gate | ReAct tool execution path | Policy check -> approval if gated -> execute tool -> audit log | READY | Approval UX latency depends on dashboard WS health |
| F2 | Browser admin API | `/api/browser/*` | Start/stop/status/action for browser automation | READY | Not equivalent to “browser as autonomous LLM tool” in all flows |
| F3 | Plugin discovery/execute | `/api/plugins`, `/api/plugins/{name}/execute/{tool}` | Discover scripts, enable/disable, execute plugin tools | READY | Script plugin behavior depends on external script reliability |

## G) Explicit Stub/Placeholder Paths

| ID | Path | Entrypoint(s) | Current Behavior | Status |
|---|---|---|---|---|
| G1 | Turn deep analysis | `POST /api/turns/{id}/analyze` | Returns stub payload | STUB |
| G2 | Session deep analysis | `POST /api/sessions/{id}/analyze` | Returns stub payload | STUB |
| G3 | Recommendations generation | `POST /api/recommendations/generate` | Returns stub payload | STUB |
| G4 | Slash retry command | `/retry` bot command path | Placeholder response | STUB |

## H) Scheduler / Cron / Background Operations

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| H1 | Cron CRUD | `/api/cron/jobs*` | Create/update/delete/list jobs persisted in DB | READY | Validate all schedule kinds used by UI |
| H2 | Cron execution loop | `run_cron_worker` | Detect due jobs, lease, execute payload action, record run | PARTIAL | Executor supports limited action set (`log`/`noop`) |
| H3 | Schedule kind compatibility | UI schedule + worker kinds | Jobs created by UI should execute | PARTIAL | Potential mismatch between `interval` and worker-recognized kinds |
| H4 | Heartbeat task logging | `run_heartbeat` | Task outcomes recorded, metrics snapshots produced | PARTIAL | Some task handlers are success/no-op style placeholders |
| H5 | Cache persistence daemon | Bootstrap cache flush task | Flush in-memory cache to SQLite and evict expired entries | READY | None significant |

## I) Wallet / Treasury / Yield / Payments

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| I1 | Wallet read endpoints | `/api/wallet/balance`, `/api/wallet/address` | Surface balances/address/network | READY | Chain/RPC configuration dependent |
| I2 | Treasury policy checks in runtime | Internal policy usage | Enforce spend/risk limits in real execution paths | PARTIAL | Some policy logic is library-implemented but not fully enforced in all runtime paths |
| I3 | Yield engine lifecycle | Yield engine module + heartbeat mention | Deposit/withdraw/harvest style operations | PARTIAL | Runtime orchestration is limited; much behavior tested at unit level |
| I4 | x402 payment flow | x402 module | Full inbound/outbound payment protocol behavior | PARTIAL | Core module exists; broad server integration path remains limited |

## J) Discovery / A2A / MCP / Device

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| J1 | A2A hello handshake | `POST /api/a2a/hello` | Handshake and response path | READY | Does not imply full encrypted session/message exchange coverage |
| J2 | Discovery protocol runtime | Discovery module | DNS/mDNS discovery + runtime registry usage | PARTIAL | Module-level support exists; full runtime network path is incomplete |
| J3 | Device identity/pairing runtime | Device module | Pair/verify/sync flows in production runtime | PARTIAL | Core state machine exists; full operator/runtime workflows incomplete |
| J4 | MCP client/server integration | MCP modules/config | Discover and execute MCP tools/resources | PARTIAL | Scaffolding exists; full remote integration wiring is incomplete |

## K) Dashboard Surface Paths (Operator UX)

| ID | Integration Path | Entrypoint(s) | Expected End-to-End | Status | Known Risks / Gaps |
|---|---|---|---|---|---|
| K1 | Sessions + chat UI | Dashboard SPA + sessions APIs | List/open/send/chat stream with role labels and context actions | READY | Requires WS/SSE stability for best UX |
| K2 | Safe markdown rendering | Session + context renderers | Render markdown with sanitized links and no unsafe HTML/script execution | READY | Keep payload fuzz tests in regression suite |
| K3 | Context Explorer | Turns/context/tools/tips APIs | Turn timeline + context details + tips | READY | Analyze actions are stub-backed today |
| K4 | Metrics/capacity panel | Stats APIs + dashboard metrics page | Cost/tokens/capacity visibility | READY | Capacity values depend on traffic volume |

## Recommended P0 Integration Tests (Run Every RC)

1. A1 + A2 parity: same prompt through non-stream and stream; verify persistence and metrics parity.
2. B1/B3 channel ingress: webhook message -> assistant reply -> session scope correctness.
3. C1/C2 migration + scope integrity: run migration on representative pre-0.6.0 DB snapshot.
4. C3 TTL + C4 rotation: heartbeat tick simulation with configured `session` settings.
5. D2/D3 capacity stress: force provider saturation and confirm routing shift + breaker pressure signal.
6. E1/E2 retrieval and post-turn ingestion: verify memory entries and embeddings materialize.
7. F1 policy/approval loop: gated tool request blocks until approval and resumes correctly.
8. H2/H3 cron execution: verify UI-created schedule kinds execute as expected.
9. K2 markdown safety fuzz: malicious markdown payload corpus in session/context surfaces.
10. G-path sanity: verify stub endpoints are explicitly labeled and not silently treated as complete.
11. UAT CLI/Web smoke: run `bash scripts/run-uat-stack.sh` and ensure operator-critical commands and dashboard APIs are healthy.
12. Soak/fuzz stability: run `SOAK_ROUNDS=4 FUZZ_SECONDS=45 just test-soak-fuzz` with no flakes or crashes.
