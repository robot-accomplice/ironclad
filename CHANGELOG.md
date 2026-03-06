# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Behavior soak hardening**: `scripts/run-agent-behavior-soak.py` now includes regression checks for filesystem capability truthfulness, subagent capability response quality, and affirmative continuation quality, with rubric updates to score substantive outcomes over brittle phrase matching.
- **Roadmap/release traceability**: `docs/releases/v0.9.5.md` and `docs/ROADMAP.md` updated with current v0.9.5 prep status for speculative execution, browser runtime support, CLI skill roadmap slice, and behavior continuity validation.
- **Architecture documentation**: Added explicit v0.9.5-prep control/dataflow coverage for deterministic execution shortcuts and guarded response sanitization in `docs/architecture/ironclad-dataflow.md` and `docs/architecture/ironclad-sequences.md`.

### Fixed

- **Internal protocol fallback leakage**: response sanitization no longer surfaces protocol-placeholder fallback text; empty/degraded sanitized content now resolves through deterministic user-facing quality fallback.
- **Markdown count execution reliability**: execution shortcut path now handles recursive markdown-file count prompts deterministically, including strict numeric-only responses when requested (`count only` / `only the number` style prompts).
- **Delegation shortcut boundary**: markdown-count shortcut no longer hijacks explicitly delegated prompts, preserving delegation intent handling.

## [0.9.4] - 2026-03-05

### Added

- **Routing observability UX**: Metrics dashboard now includes an explorable model-decision graph and a routing-profile spider graph (correctness/cost/speed) with runtime apply support via safe config patching.
- **Model shift telemetry**: Non-streaming inference pipeline now emits websocket `model_shift` events when execution model differs from selected model (fallback or cache continuity path).
- **Routing profile roadmap spec**: Added `docs/roadmap/0.9.4/features/user-routing-profile-spider-graph.md` and linked roadmap entry.

### Changed

- **Agent message contract**: `/api/agent/message` responses now expose both routing-time and execution-time model fields (`selected_model`, `model`, `model_shift_from`) for continuity diagnostics.
- **Routing dataset privacy default**: `GET /api/models/routing-dataset` now redacts `user_excerpt` by default; explicit opt-in is required to include excerpts.
- **Routing eval validation**: `POST /api/models/routing-eval` now validates `cost_weight`, `accuracy_floor`, and `accuracy_min_obs` bounds.
- **Config defaults/tests**: routing defaults now use `metascore`; legacy `heuristic` input is accepted and normalized to `metascore` during validation.
- **Cache integrity mode for live agent path**: semantic near-match cache reuse is now disabled in the inference pipeline (`lookup_strict`: exact + tool-TTL only) to prevent instruction-mismatched cached responses.
- **Path normalization parity**: runtime `PUT /api/config` updates now apply the same tilde (`~`) path expansion as TOML load (`normalize_paths`), including multimodal, device, and knowledge source path fields.
- **Explicit config path behavior**: `resolve_config_path(Some(\"~/...\"))` now expands to the user home directory instead of preserving a literal `~`.

### Fixed

- **Live startup migration deadlock on legacy DBs**: database initialization/migration order no longer fails on `inference_costs.turn_id` index creation when the column is absent in legacy state.
- **Migration 13 idempotency**: routing v0.9.4 migration path now handles pre-existing `turn_id`/routing columns without `duplicate column` failures.

### Security

- **Strict deny-by-default channels**: adapters now reject traffic when allowlists are empty (`deny_on_empty=true`). Alpha update/mechanic flows are expected to repair channel allowlists during upgrade/install.

## [0.9.3] - 2026-03-03

### Fixed

- **Sub-agent fallback model persistence**: `upsert_sub_agent` now normalizes missing/empty `fallback_models_json` to `'[]'` so inserts/updates cannot violate the `sub_agents.fallback_models_json NOT NULL` constraint.
- **Agent loop detection semantics**: loop detection in `AgentLoop::transition` now evaluates against prior calls before recording the current call, matching `LOOP_DETECTION_WINDOW` intent.
- **Financial policy amount normalization**: `FinancialRule::extract_amount_cents` now interprets `amount` consistently as dollars (int/float), while cent-denominated keys remain explicit (`amount_cents`, `cents`, `value_cents`).

## [0.9.2] - 2026-03-02

### Added

- **Wiring Remediation (Phase 0)**: Comprehensive Tier 1–3 wiring audit remediation. 14 gates cleared — all functional wires verified against code. See `docs/audit/wiring-audit-v0.9.md` for the full re-audit.
- **Unified Request Pipeline**: API (`agent_message`) and channel (`process_channel_message`) paths now share `prepare_inference` + `execute_inference_pipeline` in `core.rs`, eliminating 6+ behavioral asymmetries between entry points.
- **Multi-Tool Parsing**: `parse_tool_calls` (plural) correctly parses multiple tool invocations from a single LLM response across all four provider formats.
- **OpenAI Responses + Google Tool Wiring**: Bidirectional tool support for OpenAI Responses API and Google Generative AI — tool definitions translated into requests, structured tool calls parsed from responses with `{"tool_call": ...}` shim.
- **Quality Warm Start**: `QualityTracker` is seeded from `inference_costs` on startup, eliminating cold-start assumptions for metascore routing.
- **Escalation Read Feedback**: `EscalationTracker` acceptance history now feeds routing weight adjustments via `escalation_bias`, closing the feedback loop.
- **Approval Resume**: Blocked tool calls are re-executed asynchronously after approval via `execute_tool_call_after_approval`.
- **Hippocampus (2.13)**: Self-describing schema map with auto-discovery of all system tables. Agent-created tables (`ag_<id>_*`) with access levels, row counts, and guardrails. Compact summary injected into system prompt (~200 tokens) for ambient storage awareness.
- **Agent Data Tools**: `CreateTable`, `AlterTable`, `DropTable` registered in ToolRegistry with hippocampus auto-registration, size limits, and reserved-name enforcement.
- **Document Ingestion Pipeline (3.5.5)**: `ironclad ingest <path>` CLI and `POST /api/knowledge/ingest` API. Supports `.md`, `.txt`, `.rs`, `.py`, `.js`, `.ts`, `.pdf` files. Parse → chunk (512 tokens, 64-token overlap) → embed → store in memory system.
- **IANA Timezone Support (1.18)**: Cron scheduler evaluates session reset schedules using IANA timezone identifiers. Conformance tests for DST transitions, sub-minute cron, timezone-prefixed expressions.
- **Inference Costs Extension**: `latency_ms` (INTEGER), `quality_score` (REAL), `escalation` (BOOLEAN) columns added to `inference_costs` table. All inference calls now record latency and escalation state.
- **MCP Server Gateway**: First plugin release. `IroncladMcpHandler` bridges rmcp's `ServerHandler` to the ToolRegistry. External MCP clients (Claude Desktop, Cursor, VS Code) connect via StreamableHTTP, discover tools through `tools/list`, invoke through `tools/call`. All MCP tool calls run with `InputAuthority::External`.
- **Golden Test Fixtures**: Deterministic golden files for delegation, delegation follow-up, echo follow-up, and echo tool-call pathways.
- **Tool-Call Shim Tests**: Harness integration tests verifying the full structured tool_call → parse → execute → observation → follow-up pipeline.

### Changed

- **`post_turn_ingest` Tool Results**: All call sites now pass actual tool call name + result from the ReAct loop instead of `&[]`. Episodic memory captures tool-use context, improving digest quality.
- **Gate System Note**: `build_gate_system_note` now wired in both API and channel paths (previously channel-only).
- **Shared Confidence Evaluator**: `infer_with_fallback` uses the shared `LlmService.confidence` instance instead of creating a local copy.
- **Context Pruning**: `needs_pruning()` → `soft_trim()` wired in `build_context` when assembled context exceeds the token budget.
- **Checkpoint Load**: `load_checkpoint` called during inference preparation for session resume (previously write-only).
- **Importance Decay**: `decay_importance` called from `SessionGovernor.tick()` after digest, preventing stale context accumulation.
- **CI Pipeline**: Parallelized per-crate test execution and harness quick-test stages for faster CI runtime.

### Removed

- **`SpawnManager`**: Dead module removed (`spawning.rs` deleted, zero references). Virtual delegation tool pattern replaced it.
- **Dead Routing Surfaces**: `uniroute.rs` (ModelVector, QueryRequirements, ModelVectorRegistry) deleted. Dead selector functions (`select_for_complexity`, `select_cheapest_qualified`, `select_for_quality_target`) removed. `ModelRouter` retained as active runtime override/fallback router.
- **`router_integration.rs`**: Dead test module removed (tested deleted routing code).
- **`skills-roadmap-2026.md`**: Superseded by `capabilities-roadmap-2026.md`.

## [0.9.1] - 2026-03-01

### Added

- **Model Metascore Routing (2.19 core)**: Unified per-model scoring replaces availability-first routing. `ModelProfile` combines static provider attributes (cost, tier, locality) with dynamic observations (quality, capacity headroom, circuit breaker health). `metascore()` produces a transparent 5-dimension breakdown (efficacy, cost, availability, locality, confidence) with configurable weights for cost-aware mode. `select_by_metascore()` is now the primary routing decision in `select_routed_model_with_audit()`.
- **Tiered Inference Pipeline (2.3)**: `ConfidenceEvaluator` scores local model responses using token probability, response length, and self-reported uncertainty signals. Responses below the confidence floor trigger automatic escalation to the next model in the fallback chain. `EscalationTracker` records escalation events for capacity/cost telemetry.
- **Throttle Event Observability (1.17)**: New `GET /api/stats/throttle` endpoint exposes live rate-limit counters including global/per-IP/per-actor request counts, throttle tallies, and top-10 offenders. `ThrottleSnapshot` struct provides admin visibility into abuse patterns.
- **Quality Tracking**: `QualityTracker` now records observations on every inference success with a heuristic quality signal (response structure, finish reason, latency). Exponential moving average feeds into metascore efficacy dimension.
- **Audit Trail Extensions**: `ModelSelectionAudit` now includes `metascore_breakdown` (full per-dimension scores) and `complexity_score` for routing decisions. `ModelCandidateAudit` includes per-candidate metascores.
- **Profile module** (`ironclad-llm::profile`): `ModelProfile`, `MetascoreBreakdown`, `build_model_profiles()`, `select_by_metascore()` — 9 unit tests covering local/cloud task routing, cold-start penalties, cost-aware selection, blocked model filtering, and deterministic tie-breaking.

### Changed

- **Routing hot path**: `select_routed_model_with_audit()` now extracts features from user content, classifies task complexity, builds model profiles, and selects via metascore — replacing the previous first-usable-model strategy.
- **Rate limiter architecture**: `GlobalRateLimitLayer` is now constructed once at startup and shared between the axum middleware stack and `AppState`, enabling admin observability of the same rate-limit counters the middleware uses.

## [0.9.0] - 2026-03-01

### Added

- **Durable Delivery Queue**: Channel messages now persist to SQLite before delivery. On startup, `DeliveryQueue::with_store(db)` recovers undelivered messages and retries them with exponential backoff, preventing message loss across restarts.
- **Episodic Digest**: `digest_on_close()` is now wired into `SessionGovernor`. When sessions expire or rotate, the governor summarizes conversation history via the LLM and stores it as episodic memory, improving long-term context quality and reducing stale-context dredging.
- **Prompt Compression**: A `PromptCompressor` gate in the context assembly pipeline compresses prompts when `config.cache.prompt_compression` is enabled. Reduces token usage on large context windows while preserving semantic fidelity.
- **Context Checkpoint**: New `[context.checkpoint]` config section with `enabled` and `every_n_turns` controls. Checkpoints save system-prompt hash, memory summary, active tasks, and conversation digest to `context_checkpoints` table, enabling fast context warm-up on session restore.
- **Introspection Skill**: Four new read-only tools (`get_runtime_context`, `get_memory_stats`, `get_channel_health`, `get_subagent_status`) give the agent self-awareness of its runtime state, memory tiers, channel connectivity, and subagent/task status.
- **ToolContext extensions**: `channel: Option<String>` and `db: Option<Database>` fields added to `ToolContext`, enabling tools to understand their invocation context and query runtime state directly.
- **Architecture diagrams**: Five new dataflow diagrams (§20–§24) and three new sequence diagrams (§14–§16) documenting checkpoint, delivery queue, digest, compression, and introspection subsystems.

### Changed

- **Agent module decomposition**: The monolithic `agent.rs` (5,832 lines) has been decomposed into 15 focused submodules under `agent/`: `mod.rs` (86 lines), `handlers.rs`, `streaming.rs`, `channel_message.rs`, `core.rs`, `decomposition.rs`, `routing.rs`, `tools.rs`, `guards.rs`, `delegation.rs`, `diagnostics.rs`, `orchestration.rs`, `bot_commands.rs`, `channel_helpers.rs`, `poll_loops.rs`. No file exceeds 500 lines.
- **Test decomposition**: The monolithic `tests.rs` (1,067 lines) split into 6 focused test modules under `agent/tests/`: `guard_tests`, `tool_tests`, `channel_tests`, `decomposition_tests`, `diagnostics_tests`, `routing_tests`.
- **Decomposition helper**: Extracted shared decomposition orchestration logic (previously duplicated between `agent_message` and `process_channel_message`) into `decomposition.rs::apply_decomposition_decision()`.
- **DigestConfig threading**: `SessionGovernor` now receives `DigestConfig` from the heartbeat scheduler, enabling configurable digest behavior without hardcoded defaults.

## [0.8.9] - 2026-03-01

### Security

- **HIGH: RwLock held across LLM call**: Config read-lock was held for the entire duration of streaming LLM calls, blocking all config writes. Now clones needed values and drops the lock before the network call.
- **HIGH: CSS selector injection**: Browser `click` and `type_text` actions now validate CSS selectors, rejecting inputs containing `{`/`}` (which can escape selector context into rule injection) and enforcing a 500-character length limit.
- **HIGH: Relaxed atomic ordering**: Cross-task flags and counters using `Ordering::Relaxed` upgraded to `Acquire`/`Release`/`AcqRel` to ensure correct visibility guarantees across async task boundaries.

### Fixed

- **HIGH: SSE streaming drops tool-use deltas**: OpenAI-format SSE chunks with `content: null` (common in function-call and tool-use deltas) were silently dropped. Now emits an empty-string delta, matching the Anthropic and Google format arms.
- **HIGH: Done event schema mismatch**: The SSE `stream_done` event used `"content"` key while all streaming chunks used `"delta"`, causing clients to miss the done signal. Now consistently uses `"delta"`.
- **HIGH: Dead-letter replay race**: Two locks acquired non-atomically during message replay could interleave with concurrent deliveries. Now holds both locks in a single scope.
- **HIGH: ReAct tool errors bypass scan_output**: Error messages from tool execution were returned directly to the model without content scanning. Now calls `scan_output()` on tool error strings.
- **HIGH: derive_nickname Unicode panic**: `&text[prefix.len()..]` applied a byte offset from a lowercased string to the original, panicking on multi-byte characters. Now uses `char_indices().nth()` for safe boundary detection.
- **MED: WebSocket idle timeout missing**: `handle_socket` had no timeout — idle clients held file descriptors and broadcast receivers indefinitely. Now sends ping every 30s with a 90s idle timeout.
- **MED: Web path bypasses decomposition gate**: `evaluate_decomposition_gate` was only called in `process_channel_message`, not in the web API's `agent_message`. Extracted into a shared helper called from both paths.
- **MED: Agent processing invisible in logs**: Neither `agent_message` nor `process_channel_message` logged entry spans. Added `info!` spans with session_id and channel at function entry.
- **MED: --json flag ignored**: The `--json` CLI flag was only threaded to `cmd_defrag`. Now threaded to `cmd_status` and other output-producing commands.
- **MED: Config capabilities empty**: `/api/config/capabilities` returned an empty `immutable_sections` list. Now populated with `["server", "treasury", "a2a", "wallet"]`.
- **MED: config get returns stale TOML**: `ironclad config get` read from the on-disk TOML even when the server was running with different runtime values. Now tries the live API first, falling back to TOML when offline.
- **MED: A2A missing from channel status**: `/api/channels/status` omitted the A2A channel. Now includes a hardcoded A2A entry reading enabled/listening state from server state.
- **MED: Dashboard scheduler hardcodes agent_id**: The scheduler panel used a hardcoded `agent_id: 'ironclad'` instead of the active agent. Now uses `App._activeAgentId`.
- **LOW: Missing #[must_use] annotations**: Added `#[must_use]` to 8 builder/constructor methods across `speculative.rs`, `actions.rs`, and `knowledge.rs` to prevent accidental discard of return values.

## [0.8.8] - 2026-03-01

### Security

- **HIGH: WebSocket API key leak**: Replaced `?token=` query-string authentication on WebSocket upgrade with a ticket-based flow, preventing API keys from appearing in server logs, proxy logs, and browser history.
- **HIGH: Prompt injection in tips**: `get_turn_tips` and `get_session_insights` now sanitize LLM-generated tips before rendering, preventing stored prompt injection via malicious session content.
- **HIGH: Provider error info leak**: `classify_provider_error` in `run_llm_analysis` now strips internal details from error responses before returning to callers.
- **MED: XSS in sanitize_html**: `sanitize_html` now escapes all 5 OWASP-recommended HTML entities (`& < > " '`), closing a reflected XSS vector.
- **MED: Input validation on identifiers**: `peer_id`, `group_id`, and `channel` fields now enforce length and character-set constraints, preventing injection of oversized or malformed identifiers.
- **MED: Webhook body size limit**: Public webhook router now applies `DefaultBodyLimit` to prevent memory exhaustion from oversized payloads.
- **MED: Analysis route DoS protection**: Analysis routes now apply `ConcurrencyLimitLayer(3)` to prevent resource exhaustion from concurrent expensive LLM calls.
- **MED: Config schema leak**: `update_config` error responses now return a generic message instead of leaking internal schema details.
- **MED: Feedback comment size limit**: `FeedbackRequest.comment` now enforces a 4096-character cap, preventing oversized payloads from reaching storage.
- **MED: Config allowlist tightening**: Removed `extra_headers` from the `get_config` response allowlist, preventing exposure of sensitive header values.
- **LOW: Unsafe UTF-8 decode**: Replaced `from_utf8_unchecked` with safe `from_utf8` to prevent undefined behavior on malformed input.
- **LOW: Embedding test env isolation**: Embedding test uses a unique env var name with a SAFETY comment to prevent cross-test interference.
- **LOW: Path traversal defense-in-depth**: `obsidian_read` now validates paths against directory traversal patterns as an additional defense layer.

### Fixed

- **HIGH: Float policy bypass**: Policy enforcement on `amount` fields now falls back to `as_f64()` conversion, closing a bypass where float amounts evaded integer-only checks.
- **HIGH: Tool call parsing failures**: `parse_tool_call` now uses `rfind` with a candidate loop, correctly parsing tool calls that contain the delimiter character in arguments.
- **HIGH: Unicode string metric**: `common_prefix_ratio` now operates on `chars()` instead of byte slices, producing correct ratios for multi-byte characters.
- **HIGH: Incorrect P50 latency**: `latency_p50` now computes the true median by averaging the two middle values for even-length arrays.
- **HIGH: Speculation cache collisions**: `SpeculationKey` now stores full parameter JSON instead of using `DefaultHasher`, which was not stable across processes and caused incorrect cache hits.
- **HIGH: WhatsApp adapter panic**: `WhatsAppAdapter::new` now returns `Result<Self>` instead of panicking on initialization failures.
- **HIGH: Export agents silent failure**: `export_agents` now matches on `Result` and propagates errors instead of silently dropping them.
- **HIGH: Inference cost logging**: `record_inference_cost` now uses `inspect_err` to log failures instead of silently discarding them with `.ok()`.
- **MED: Turn count inflation**: `turn_count` now only increments on `Think` state transitions, fixing 2-3x count inflation from duplicate counting.
- **MED: Archive truncation**: `compact_before_archive` now fetches all messages instead of being capped at 20, preventing data loss during session archival.
- **MED: URL decoder corruption**: `%XX` decoder now preserves characters on invalid hex sequences instead of silently dropping them.
- **MED: Task handoff stalls**: Handoff logic now skips `Failed` tasks to find the next `Pending` task, preventing the scheduler from stalling on failed work.
- **MED: Config write propagation**: `write_defaults` now propagates errors with `?` instead of silently discarding them with `.ok()`.
- **MED: Cron validation logging**: Invalid cron expressions now log a warning before returning `false`, replacing a silent rejection.
- **MED: Wallet passphrase fallthrough**: An incorrect `IRONCLAD_WALLET_PASSPHRASE` now produces a hard error instead of silently falling through to the default passphrase.
- **MED: Config/session export errors**: `to_string_pretty` failures in config/session export now return proper error responses instead of empty bodies.
- **MED: Corrupt skills warning**: Corrupt `skills_json` values now log a warning instead of being silently ignored.
- **MED: Translation request errors**: `translate_request` failures now return HTTP 500 with a proper error body instead of an empty response.
- **MED: Translation response errors**: `translate_response` failures now return HTTP 502 with a descriptive message instead of `"(no response)"`.
- **LOW: Loop detection consolidation**: Removed redundant `is_looping` pre-check, consolidating loop detection into a single code path.
- **LOW: Archive count accuracy**: `rotate_agent_scope_sessions` now returns the actual archived count instead of a potentially incorrect value.
- **LOW: Token parse overflow**: Token parsing now uses saturating `u32` casts, capping at `u32::MAX` instead of panicking on overflow.
- **LOW: Subtask dedup ordering**: `split_subtasks` now uses a `HashSet` for order-preserving deduplication instead of unstable dedup.
- **LOW: Session row corruption logging**: Corrupted session rows now log a warning instead of being silently dropped during iteration.
- **LOW: DB error logging for cost queries**: Database errors in turn-query average cost calculations are now logged instead of silently ignored.
- **LOW: Defrag read error handling**: File defragmentation now skips files on read error with a warning instead of substituting an empty string.

## [0.8.7] - 2026-02-28

### Fixed

- **CRIT: Cron jobs silently never firing**: `run_cron_worker` timestamp format lacked timezone suffix (`Z`), causing `evaluate_cron` RFC 3339 parse to always fail — all cron-scheduled jobs were silently skipped.
- **HIGH: Telegram chunk_message UTF-8 panic**: Byte-level string slicing in `chunk_message` panicked on multi-byte characters (emoji, CJK). Now uses `floor_char_boundary()` matching the Discord adapter.
- **HIGH: Keystore redact_key_name UTF-8 panic**: Byte-level `&key[..3]` slicing panicked on multi-byte key names. Now uses `key.chars().take(3)`.
- **HIGH: LLM forward_stream missing query: auth mode**: Streaming requests to providers using query-string authentication (e.g., Google Generative AI) failed because the `query:` prefix was not handled, sending it as a literal HTTP header instead.
- **HIGH: yield_engine U256-to-u64 panic**: `real_a_token_balance` panicked via `U256::to::<u64>()` if an aToken balance exceeded `u64::MAX`. Now uses safe `try_into::<u128>()`.
- **HIGH: yield_engine amount_to_raw saturation**: `amount_to_raw` silently saturated USDC amounts above ~$18.4B via unchecked `f64 -> u64` cast. Now explicitly clamps.
- **MED: Email adapter SMTP relay panic**: `EmailAdapter::new` panicked via `.expect()` on invalid SMTP hostname. Now returns `Result`.
- **MED: Email adapter mutex panics**: `push_message`/`recv` used `.expect("mutex poisoned")`. Now uses `.unwrap_or_else(|e| e.into_inner())` for poison recovery, matching other adapters.
- **MED: Discord GatewayConnection mutex panics**: All 4 accessor methods used `.expect("mutex poisoned")`. Now uses poison recovery matching the rest of the Discord adapter.
- **MED: CDP client initialization panic**: `CdpClient::new` panicked via `.expect()` on TLS cert issues. Now returns `Result`.
- **MED: Embedding URL double API key**: When both Google format and `query:` auth were active, the API key was appended twice. Made the two paths mutually exclusive.
- **MED: Embedding URL missing percent-encoding**: API keys were interpolated into URLs without encoding. Now uses `pct_encode_query_value`.
- **MED: Hippocampus Unicode/ASCII mismatch**: `create_agent_table` allowed Unicode alphanumeric characters but `drop_agent_table` required ASCII-only, creating undeletable tables. Both now require ASCII.
- **MED: Skills reload counters wrong on failure**: `added`/`updated` counters incremented even when DB operations failed. Now only increment on success.
- **MED: Skills rollback silent failures**: File rollback operations used `let _ =` silently. Now log errors at error level.
- **LOW: sanitize_platform mixed byte/char units**: Truncation used `.chars().take()` (char count) after a `.len()` (byte count) guard. Now truncates at byte boundary consistently.
- **LOW: mock_tx_hash f64 saturation**: Used `amount * 1e18` (overflows u64 above ~18.4). Changed to USDC scale (1e6).
- **LOW: Session model column never populated**: `update_model()` was not called after LLM routing, leaving the `sessions.model` column perpetually NULL.
- **LOW: Moonshot/Kimi tier misclassified**: `classify()` in `tier.rs` did not match `moonshot` or `kimi` substrings, causing Kimi K2 models to fall through to the T2 default instead of T3.

### Added

- Release notes for v0.8.5 and v0.8.6 (missing from previous releases, blocking release doc gate).
- Roadmap section 1.24: Built-in CLI Agent Skills (Claude Code + Codex CLI).

## [0.8.6] - 2026-02-28

### Security

- **CRIT: Unauthenticated rate-limit actor identity**: Removed `x-user-id` header as rate-limit actor identity — it was unauthenticated and trivially spoofable.
- **CRIT: Stable token fingerprinting**: Replaced `DefaultHasher` with SHA-256 for token fingerprinting, since `DefaultHasher` is not stable across processes and could cause cache/rate-limit bypasses.
- **HIGH: Rate-limit IP fallback**: IP extraction now uses `ConnectInfo<SocketAddr>` (real TCP peer address) instead of a hardcoded `127.0.0.1` fallback.
- **HIGH: ASCII-only identifiers**: `validate_identifier` now restricts to ASCII alphanumeric characters, closing Unicode homoglyph and normalization attacks.
- **HIGH: Memory search query cap**: `/api/memory/search` query parameter capped at 512 characters to prevent regex-based DoS.
- **HIGH: Error message sanitization**: Added SQLite schema-leaking prefixes (`no such table`, `no such column`, etc.) to the error sanitization blocklist.
- **MED: Rate-limit counter ordering**: Global rate-limit counter now incremented after per-IP/per-actor checks pass, preventing global exhaustion from blocked IPs.
- **MED: Symlink-safe directory traversal**: `collect_findings_recursive` now uses `entry.file_type()` and skips symlinks, preventing symlink-following attacks.
- **MED: WhatsApp HMAC raw byte comparison**: HMAC verification now compares raw bytes instead of hex string representations, closing timing side-channels from variable-length hex comparison.

### Fixed

- **Windows daemon error propagation**: `schtasks /Create` errors now propagate instead of being silently dropped; post-spawn verification added; `schtasks /Delete` errors during uninstall handled correctly.
- **CLI API key headers**: Added `--api-key`/`IRONCLAD_API_KEY` global CLI argument. All 22 bare `reqwest` calls replaced with `http_client()` helper that injects API key as default header.
- **Flaky test elimination**: Replaced TOCTOU ephemeral port test with RFC 5737 TEST-NET-1 address (192.0.2.1) for deterministic unreachable-port testing.
- **Bundled providers parse failure (F5)**: Changed `.unwrap_or_default()` to `.expect()` — bundled TOML is build-time data; parse failure means the binary is broken and should panic fast.
- **Update state save errors (F3)**: Three `state.save().ok()` sites now log errors before discarding, plus update state load now logs parse/read failures.
- **Legacy Windows service cleanup (F7)**: `sc.exe stop/delete` errors during legacy cleanup now logged at debug level instead of silently dropped.
- **OAuth token resolution (F8)**: `resolve_token().ok()` now logs failures, surfacing OAuth refresh errors that were previously invisible.
- **Translate request error propagation (F9)**: `translate_request` errors now return HTTP 400 instead of falling back to an empty JSON body.
- **Corrupted cost row logging (F10)**: `filter_map(|r| r.ok())` on cost query rows now logs dropped rows.
- **Embedding failure logging (F12)**: Three `embed_single().ok()` sites now log failures, making RAG degradation visible.
- **Defrag stdout write errors (F14)**: JSON stdout writes now propagate `io::Error` instead of silently dropping.
- **Session nickname update (F19)**: `update_nickname().ok()` now logs failures.
- **Recommendation inference cost (F20)**: `record_inference_cost().ok()` now logs failures.
- **Agent status query errors**: Tool call and turn queries in agent status now log errors at debug level.

### Added

- Auth middleware roundtrip tests: wrong key rejection, no-auth passthrough, POST method coverage.
- SSE streaming endpoint validation tests: empty content, oversized content, missing fields.

## [0.8.5] - 2026-02-28

### Security

- **WASM preemptive timeout (BUG-101)**: WASM plugin execution now runs on a dedicated thread with `recv_timeout`, providing true preemptive timeout instead of the previous post-hoc elapsed-time check that allowed malicious modules to run indefinitely.
- **Script runner orphan kill (BUG-102)**: Script runner now captures the child PID before `wait_with_output()` and sends `kill -9` on timeout, preventing orphan process accumulation.
- **Rate limiter memory bounds (BUG-103)**: Per-IP and per-actor rate limit maps are now capped at 10,000 and 5,000 entries respectively, preventing unbounded memory growth during distributed floods. Throttle tracking maps are also cleared on window reset.
- **Knowledge/Obsidian bounded reads (BUG-104, BUG-110)**: `DirectorySource::query()` and `parse_note()` now enforce 10 MB and 5 MB file size limits respectively, preventing OOM on oversized files.
- **Config secret allowlist (BUG-106)**: Admin config endpoint now uses an allowlist (`ALLOWED_FIELDS`) instead of a blocklist for field filtering, ensuring new secret fields are safe by default.
- **Interview turn cap (BUG-107)**: Interview sessions now enforce a 200-turn maximum to prevent unbounded memory growth within the 3600s TTL.

### Fixed

- **reqwest Client panic (BUG-105)**: `VectorDbSource::new()` and `GraphSource::new()` now return `Result` instead of panicking via `.expect()` when TLS initialization fails.
- **Signal handler crash (BUG-108)**: SIGTERM handler installation now falls back to SIGINT-only mode instead of crashing via `.expect()` in containerized environments.
- **Heartbeat unreachable panic (BUG-109)**: `interval_for_tier()` catch-all arm now returns a safe default (`interval_ms * 2`) instead of `unreachable!()`, preventing runtime panics if new `SurvivalTier` variants are added.
- **Regex recompilation (BUG-111)**: Obsidian tag and wikilink regexes are now `LazyLock` statics instead of being recompiled on every invocation.
- **Budget float precision (BUG-112)**: `record_spending()` now uses epsilon-aware comparison to avoid IEEE 754 rounding errors causing spurious over-budget rejections.
- **Sub-agent lifecycle failures (SF-15–SF-20)**: All `let _ =` patterns on `registry.register()`, `start_agent()`, `stop_agent()`, `unregister()`, and `assign_agent()` now log errors at appropriate levels.
- **API key env var diagnostics (SF-21, SF-22)**: Empty and missing API key / email password environment variables now produce warn-level log messages instead of silently returning empty strings.
- **Sub-agent list errors (SF-23)**: `list_sub_agents` DB errors now propagate at the delegation entry point and log at remaining fallback sites.
- **Skills list errors (SF-24)**: `list_skills` DB failure now logged before fallback.
- **MCP discovery failure (SF-25)**: MCP client discovery errors at startup now logged at warn level.
- **Semantic cache load failure (SF-26)**: Cache load errors now logged before fallback to empty.
- **Provider key resolution (SF-27)**: Missing provider keys for non-local providers now produce warn-level diagnostics.
- **Bundled providers parse failure (SF-28)**: TOML parse errors for bundled providers now logged.
- **Config backup restore (SF-29)**: Failed hot-reload backup restoration now logged at error level.
- **Migration SQL errors (SF-30)**: SQL execution failures during migration now surfaced as warnings.
- **Thinking indicator failures (SF-31)**: Channel thinking indicator send failures now logged at debug level across all 4 platforms.
- **Session candidates JSON (SF-32)**: Model selection candidate deserialization errors now logged.
- **Telegram API errors (SF-33)**: Typing indicator and message delete HTTP failures now logged at debug level.
- **Session counts fallback (SF-34)**: Sub-agent session count DB errors now logged before fallback.
- **Subtask JSON parse (SF-35)**: Malformed `subtasks` parameter (non-array) now produces a warning instead of silently returning empty.
- **19 additional MEDIUM silent failures (SF-36–SF-52)**: Error logging added across oauth, plugin-sdk, retrieval, digest, skills, signal, discord, whatsapp, sessions, defrag, embedding, main CLI, keystore, and obsidian modules.
- **Migration export cascade (SF-48)**: Channel export now properly reports file read failures and JSON serialization errors instead of silently producing empty output.

## [0.8.4] - 2026-02-28

### Security

- **WebSocket message size limit**: Unauthenticated WebSocket connections now enforce a 4 KiB inbound message limit and no longer echo full message bodies, closing a ~3x memory amplification DoS vector.
- **Hippocampus TOCTOU fix**: `drop_agent_table` auth check and DROP are now wrapped in a single transaction, preventing race-condition bypasses.
- **Script runner bounded reads**: Shebang detection now uses `BufReader::take(512)` instead of `read_to_string`, preventing OOM on oversized script files.

### Fixed

- **Agent amnesia on DB error (SF-2)**: `list_messages` calls in agent routes now propagate errors instead of silently returning empty history via `.unwrap_or_default()`.
- **Governor silent write failures (SF-1)**: Session expiry and compaction errors are now logged at warn/error level; `tick()` returns an accurate expired count instead of silently swallowing failures with `.ok()`.
- **Money::from_dollars NaN panic (BUG-2)**: `from_dollars` now returns `Result`, rejecting NaN and Infinity inputs instead of panicking via `assert!`.
- **Delivery queue recovery (SF-7)**: `recover_from_store` is now async with proper `.lock().await`, replacing a `try_lock()` that silently dropped recovered messages.
- **Agent loop detection enforcement (BUG-3)**: `is_looping()` is now called inside `transition()` and forces `Done` state, preventing callers from bypassing loop detection.
- **Digit-leading SQL identifiers (BUG-7)**: `validate_identifier` now rejects names starting with digits, which would produce invalid SQL.
- **Embedding API key error message (SF-4)**: Missing API key env var now returns a clear error message instead of a cryptic 401 via `.unwrap_or_default()`.
- **ANN index corruption paths (SF-6, SF-10)**: Corrupt embedding JSON is now logged and skipped; RwLock poison on write returns an error instead of silently recovering with stale data.
- **Admin dashboard false empties (SF-3)**: DB read errors in dashboard endpoints are now logged with `inspect_err` before falling back to defaults, enabling diagnosis.
- **Session tool call queries (SF-9)**: Tool call endpoints now propagate DB errors with proper HTTP 500 responses instead of returning empty arrays.
- **EventBus publish logging (SF-5)**: `let _ =` on channel send replaced with debug-level logging when no subscribers are active.
- **Delivery queue timestamp fallback (SF-11)**: Failed timestamp parse now falls back to `UNIX_EPOCH` (safe backoff) instead of `Utc::now()` (immediate retry).
- **Dead letter false empties (SF-8)**: `dead_letters_from_store` errors now logged before fallback.
- **Admin config serialization (SF-12)**: Config endpoint returns HTTP 500 on serialization failure instead of null body.
- **Efficiency report serialization (SF-13)**: Efficiency endpoint returns HTTP 500 on serialization failure instead of null body.
- **Webhook body bytes (SF-14)**: Failed body extraction now logs a warning instead of silently discarding the payload.

### Changed

- **Crate publish ordering**: Release workflow now publishes crates in correct topological dependency order with increased index propagation wait times, fixing the v0.8.3 publish failure.

## [0.8.3] - 2026-02-27

### Security

- **Auth bypass when no API key**: Requests to non-exempt API routes now fail closed when no API key is configured — only loopback connections are allowed. Previously, missing API key config silently allowed all traffic.
- **A2A replay protection**: Added nonce registry with TTL-based expiry to the A2A protocol, preventing message replay attacks within the nonce window.
- **Plugin permission enforcement**: New `strict_permissions` and `allowed_permissions` config fields for plugin policy. In strict mode, undeclared permissions are blocked; in permissive mode (default), they produce a warning.
- **Ethereum signature recovery ID**: EIP-191 signatures now include the recovery byte (v = 27 or 28), producing correct 65-byte signatures instead of 64-byte truncated ones.

### Fixed

- **UTF-8 panic in memory truncation**: Replaced unsafe byte-level string slicing with `floor_char_boundary()` to prevent panics on multi-byte characters (emoji, CJK) near the 200-char truncation point.
- **Script plugin zombie processes**: Script timeout now explicitly kills the child process and reaps it, preventing zombie accumulation.
- **Script plugin unbounded output**: stdout/stderr from plugin scripts are now capped at 10 MB via `AsyncReadExt::take()`.
- **Keystore lock ordering**: Consolidated two separate mutexes into a single `KeystoreState` mutex, eliminating potential deadlock scenarios.

### Added

- **`ironclad defrag` command**: New workspace coherence scanner with 6 passes — refs (dead reference elimination), drift (config drift detection), artifacts (orphaned file cleanup), stale (ghost state entry removal), identity (brand consistency), and scripts (script health validation). Supports `--fix` for auto-repair, `--yes` for non-interactive mode, and `--json` for machine-readable output.

## [0.8.2] - 2026-02-27

### Added

- **100+ API route integration tests**: Comprehensive test coverage for sessions, turns, interviews, feedback, skills, model selection, channels, webhooks, dead letters, admin, memory, cron, context, and approvals endpoints. Tests exercise both success and error paths including validation, 404s, auth, and edge cases. Workspace test count now at 3,316.
- **Homebrew tap distribution**: macOS/Linux users can install via `brew install robot-accomplice/tap/ironclad`.
- **Winget package distribution**: Windows users can install via Winget package manager.

### Fixed

- **29 stabilization bug fixes**: Resolved input validation gaps, API error format inconsistencies, query parameter hardening, security headers, dashboard trailing content, model persistence, cron field naming, and Windows TOML path issues discovered during exhaustive hands-on testing of v0.8.1.
- **HTML injection prevention**: Closed remaining sanitization coverage gaps in API write endpoints.
- **Dashboard SPA cleanup**: Removed duplicate trailing content after `</html>` close tag.
- **Model change persistence**: Fixed model selection not persisting across server restarts.
- **Config serialization**: Fixed TOML config serialization on Windows paths.

## [0.8.1] - 2026-02-27

### Fixed

- **40 smoke/UAT bug fixes**: Resolved 40 bugs (5 critical, 6 high, 15 medium, 14 low/UX) discovered during comprehensive smoke testing of all 85 REST routes, 32 CLI commands, and 13 dashboard pages.
- **Input validation hardening**: Added field-length limits, HTML sanitization, and null-byte rejection across all API write endpoints.
- **JSON error responses**: All API error paths now return structured `{"error": "..."}` JSON instead of plain text.
- **Memory search deduplication**: FTS memory search no longer returns duplicate entries; results are now structured with category/timestamp metadata.
- **Cron scheduler accuracy**: `next_run_at` is now persisted after computation; heartbeat no longer floods logs with virtual job IDs; jobs use actual agent IDs.
- **Cost display precision**: Floating-point noise eliminated from cost/efficiency metrics (rounded to 6 decimal places with division-by-zero guard).
- **Skills metadata**: `risk_level` is now parameterized (not hardcoded "Caution"); skills track `last_loaded_at` timestamp.
- **CLI resilience**: `ironclad check` no longer crashes with raw Rust IO errors; shows friendly messages with config path suggestions.
- **Dashboard UX**: Fixed 14 display bugs including schedule text duplication, raw-seconds uptime, missing pagination, broken status indicators, and external font dependency removal.
- **Filesystem path exposure**: Skills API no longer leaks `source_path`/`script_path` in responses.
- **Session creation response**: `POST /api/sessions` now returns the full session object instead of just the ID.
- **404 fallback handler**: Unknown API routes now return JSON `{"error": "not found"}` instead of empty 404.

### Changed

- **CI scripts use POSIX grep**: Replaced all `rg` (ripgrep) invocations with standard `grep -E`/`grep -qE` in CI scripts for broader runner compatibility.
- **Windows compilation**: Added conditional `allow(unused_mut)` for platform-gated mutation in security audit command.

## [0.8.0] - 2026-02-26

### Security

- **CORS hardening**: Removed wildcard `Access-Control-Allow-Origin: *` fallback when no API key is configured; CORS now always restricts to the configured bind address origin.
- **Wallet key zeroing**: Decrypted API keys in the keystore and child agent wallet secrets are now wrapped in `Zeroizing<String>` so key material is zeroed on drop.
- **WalletFile Debug redaction**: `WalletFile` no longer derives `Debug`; a manual impl redacts `private_key_hex` to prevent accidental key leakage in logs or panics.
- **Plaintext wallet detection**: Loading an unencrypted wallet file now emits a `SECURITY` warning at `warn!` level instead of silently succeeding.
- **Webhook signature enforcement**: WhatsApp webhook verification now rejects requests with an error when `app_secret` is unconfigured, instead of silently skipping verification.
- **OAuth token persistence errors surfaced**: `OAuthManager::persist()` now returns `Result<()>` and callers log failures at `error!` level instead of silently swallowing write errors.
- **Skill catalog path traversal prevention**: Skill download filenames from remote registries are now validated and canonicalized to prevent `../` path traversal.
- **API key URL encoding**: The `query:` auth mode now percent-encodes API keys before appending to URLs, preventing malformed requests and log leakage.
- **Script runner absolute path rejection**: `resolve_script_path` now unconditionally rejects absolute paths instead of accepting them.
- **Script file permission check**: Script runner validates that script files are not world-writable on Unix before execution.
- **Subagent name validation**: Subagent names are now restricted to max 128 characters, alphanumeric + hyphens + underscores only.
- **Plugin name/version validation**: Plugin manifest validation now enforces character restrictions on plugin names and versions matching tool name rules.
- **Audit log key redaction**: Keystore audit log entries now redact key names to first 3 characters instead of logging full key identifiers.
- **x402 recipient address validation**: Payment authorization now validates that recipient addresses match Ethereum address format (0x + 40 hex chars).
- **JSON merge depth limit**: `update_config` recursive merge is now bounded to 10 levels of nesting to prevent stack overflow.
- **Error message sanitization**: `sanitize_error_message` now strips content after common sensitive prefixes (file paths, SQLite errors, stack traces).
- **Decided-by field sanitization**: Approval decision `decided_by` field is now limited to 256 characters with control characters stripped.

### Fixed

- **Telegram invalid-token resilience**: Telegram `404/401` poll failures are now classified as likely invalid/revoked bot-token errors with explicit repair guidance and adaptive backoff to reduce noisy tight-loop logging.
- **Subagent runtime activation sync**: Taskable subagents are now auto-started at boot and kept in sync with create/update/toggle/delete operations, fixing the `enabled > 0, running = 0` stall where configured subagents stayed idle.
- **FTS duplicate row accumulation**: `store_semantic` and `store_working` now delete existing FTS entries before re-inserting, preventing unbounded duplicate growth in `memory_fts` on upserts.
- **SSE stream UTF-8 corruption**: `SseChunkStream` now uses proper incremental UTF-8 decoding instead of `from_utf8_lossy`, preserving multi-byte characters split across HTTP chunks.
- **SSE buffer unbounded growth**: SSE chunk stream buffer is now capped at 10 MB to prevent unbounded memory growth from long SSE lines.
- **Heartbeat interval recovery**: Heartbeat daemon interval now recovers to the original configured value when the survival tier returns to Normal, instead of permanently remaining at the degraded rate.
- **AgentCardRefresh task activation**: `HeartbeatTask::AgentCardRefresh` is now included in `default_tasks()` instead of being a dead variant.
- **Hippocampus identifier consistency**: Table name validation in `create_agent_table` no longer allows hyphens, matching `validate_identifier` behavior.
- **Negative hours SQL comment injection**: `query_transactions` now clamps `hours` to positive values, preventing negative values from producing SQL comments.
- **PRAGMA identifier quoting**: `has_column` now quotes table names in `PRAGMA table_info` statements.
- **Cron lease identity verification**: `release_lease` now requires the `lease_holder` parameter and verifies ownership before releasing.
- **Coverage gate alignment**: Local `justfile` coverage threshold now matches CI at 80% minimum.
- **`just run-release` binary name**: Fixed reference from `ironclad-server` to `ironclad`.
- **Smoke test default port**: `run-smoke.sh` default port corrected from 8787 to 18789.
- **CORS fallback logging**: Invalid CORS origin parse now logs a warning and falls back to `127.0.0.1` loopback instead of silently becoming wildcard `*`.
- **Crypto function error propagation**: `derive_key`, `encrypt_wallet_data` in wallet now return `Result` instead of panicking with `expect`.
- **CapacityTracker mutex resilience**: All `expect("mutex poisoned")` calls replaced with `unwrap_or_else(|e| e.into_inner())` for graceful recovery.
- **Rate limit / approval mutex resilience**: Same mutex poison recovery applied to policy engine and approval manager.
- **Cron lease/run error logging**: `acquire_lease`, `record_run`, and `release_lease` errors are now logged at `warn` level instead of silently discarded.
- **Interval expression UTF-8 safety**: `parse_interval_expr_to_ms` now uses `char_indices()` for correct byte-offset slicing of multi-byte characters.
- **TOML serialization error propagation**: `generate_operator_toml` and `generate_directives_toml` now return `Result<String>` instead of silently returning empty strings.
- **Floating-point tier threshold**: `SurvivalTier::from_balance` uses 0.999 epsilon for the `hours_below_zero` check to handle floating-point rounding.

### Added

- **v0.8.0 zero-regression release gate**: Added canonical `just test-v080-go-live` orchestration and release-blocking CI/release jobs for workspace tests, integration/regression batteries, bounded soak/fuzz checks, CLI+web UAT smoke, and release-doc/provenance consistency checks.
- **WASM execution timeout enforcement**: WASM plugin execution now tracks elapsed time against the configured `execution_timeout_ms` and logs warnings when exceeded.
- **WASM memory bounds validation**: WASM input writes check memory size before writing; output reads validate `ptr + len` against module memory bounds.
- **Browser evaluate length limit**: `BrowserAction::Evaluate` rejects expressions exceeding 100,000 characters.
- **Email body size limit**: Email adapter truncates message bodies exceeding 1 MB.
- **A2a session establishment check**: Added `is_established()` method and documentation for session key typestate.
- **A2a rate window eviction**: Rate limit windows now evict stale entries (>1 hour idle) when exceeding 1,000 tracked peers.
- **InboundMessage platform sanitization**: Added `sanitize_platform()` to strip control characters and enforce 64-char limit.
- **YieldEngine field encapsulation**: All fields made private with getter methods.
- **TreasuryPolicy field encapsulation**: All fields made private with constructor and getter methods.
- **Zero-amount deposit/withdraw rejection**: `YieldEngine::deposit()` and `withdraw()` now reject amounts <= 0.
- **Plugin registry unregister**: Added `unregister()` method to fully remove plugin entries.
- **Script shebang validation**: Extensionless script files now require a recognized shebang line.
- **Docker HEALTHCHECK**: Dockerfile now includes a health check against `/api/health`.
- **Docker build reproducibility**: Dockerfile now uses `--locked`, MSRV-pinned Rust image, and dependency layer caching.
- **Release CI supply-chain hardening**: `cross` installation pinned to versioned release instead of git HEAD.

### Changed

- **WhatsApp client initialization**: `reqwest::Client` builder now uses `expect()` instead of `unwrap_or_default()` to surface TLS initialization failures.
- **CDP client initialization**: Same `expect()` change applied to browser CDP HTTP client.
- **Semantic search scan limit**: `search_similar` now includes `LIMIT 10000` to bound memory usage pending AnnIndex integration.
- **SemanticCache thread safety documentation**: Documented `&mut self` requirement and external synchronization expectations.

## [0.7.1] - 2026-02-25

### Fixed

- **Windows daemon startup reliability**: Replaced the broken `sc.exe` service launch path (which caused `StartService FAILED 1053`) with a managed detached user-process daemon flow on Windows.
- **Windows binary update failure mode**: `ironclad update binary` now explicitly blocks in-process self-update on Windows and prints safe manual upgrade steps, avoiding opaque `cargo install` executable move failures.
- **Dashboard JS bleed-through**: Dashboard HTML rendering now trims to the canonical document boundary, preventing stray trailing script bytes from being rendered in the UI.
- **Internal proxy regression lock-down**: Ironclad now migrates legacy `127.0.0.1:8788/<provider>` URLs to canonical in-process routing targets at startup, persists the migration safely, and removes runtime dependence on an external loopback proxy listener.
- **Dashboard/provider boundary hardening**: `/api/models/available` now reports explicit in-process proxy mode metadata so the dashboard remains server-mediated and does not rely on direct local proxy access.
- **Loopback proxy deprecation gate**: `0.7.x` now emits explicit deprecation guidance when migrating legacy `127.0.0.1:8788/<provider>` URLs, and `0.8.0+` is wired to fail fast on legacy loopback provider URLs with upgrade guidance.
- **v0.8.0 release definition**: Added `docs/releases/v0.8.0.md` gate coverage for removing legacy loopback proxy support from runtime behavior and shipped examples.
- **Telegram silent no-reply hardening**: Channel ingress now records receive/error telemetry in dedicated poll/webhook paths, and Telegram processing failures proactively trigger a user-visible fallback reply instead of failing silently.

## [0.7.0] - 2026-02-25

### Added

- **Subagent contract enforcement**: Added explicit `subagent` vs `model-proxy` role validation, fixed-skills persistence/validation, and strict rejection of personality payloads for taskable subagents.
- **Model-selection forensics pipeline**: Added persistent `model_selection_events` storage, turn-linked forensics APIs (`GET /api/turns/{id}/model-selection`, `GET /api/models/selections`), and live dashboard views for candidate evaluation details.
- **Streaming turn traceability**: `POST /api/agent/message/stream` now emits stable `turn_id` values from stream start through completion and records per-turn model-selection audits for streamed responses.
- **Subagent ubiquitous-language architecture doc**: Added `docs/architecture/subagent-ubiquitous-language.md` with canonical terminology, gap audit, and dataflow diagrams.

### Changed

- **Roster and status semantics**: `/api/roster`, `/api/agent/status`, and dashboard agent views now distinguish taskable subagents from model proxies and report taskable counts with clearer operator-facing terminology.
- **Subagent model assignment options**: Added support for `auto` (router-controlled) and `orchestrator` (primary-agent-assigned) model modes for taskable subagents, including runtime model resolution behavior.
- **Context forensics UX**: Context Explorer now supports live stream-turn handoff and direct forensic drill-down using active `turn_id` metadata.

## [0.6.1] - 2026-02-24

### Fixed

- **Release integrity follow-up**: Merged post-tag regression fixes from the 0.6.0 release branch into `develop`, including web peer-scope identity validation, dashboard WebSocket token encoding, and release-gate compile/test stabilization.
- **Session creation stability**: Restored explicit default agent scope behavior in DB session creation paths to avoid `500` failures in session lifecycle APIs/tests.
- **Routing test alignment**: Updated router integration expectations to reflect current fallback behavior when primary providers are breaker-blocked.

## [0.6.0] - 2026-02-24

### Added

- **Capacity headroom telemetry**: New `GET /api/stats/capacity` endpoint exposes per-provider headroom, utilization, and sustained-pressure flags for operator visibility.
- **Capacity-aware circuit preemption**: Circuit breakers now accept soft capacity pressure signals and expose preemptive `half_open` state before hard failure trips.
- **Session scope backfill migration**: Added `012_session_scope_backfill_unique.sql` to normalize legacy sessions to explicit scope and enforce unique active scoped sessions.
- **Safe markdown rendering in dashboard sessions**: Session chat and Context Explorer now render markdown with strict URL sanitization and no raw HTML execution.

### Changed

- **Routing quality now capacity-weighted**: `select_for_complexity()` scores candidates by model quality and provider headroom, rather than binary near-capacity fallback behavior.
- **Inference feedback loop now records capacity usage**: both non-stream and stream response paths record provider token/request usage and update capacity pressure signals.
- **Session scoping defaults to explicit agent scope**: `find_or_create()` now uses `agent` scope by default and channel/web paths pass scoped keys for peer/group isolation.
- **Channel session affinity**: Channel dedup and session selection now use resolved chat/channel identity instead of platform-only sender affinity.
- **Heartbeat now runs SessionGovernor**: stale sessions are expired with compaction draft capture; optional hourly rotation is triggered when `session.reset_schedule` is configured.

## [0.5.0] - 2026-02-23

### Added

- **Addressability Filter**: Composable filter chain for group chat addressability detection. Agent only responds when mentioned by name, replied to, or in a DM. Configurable via `[addressability]` config section with alias names support.
- **Response Transform Pipeline**: Three-stage pipeline applied to all LLM responses -- `ReasoningExtractor` (captures `<think>` blocks), `FormatNormalizer` (whitespace/fence cleanup), `ContentGuard` (injection defense). Replaces the previous inline `scan_output` approach.
- **Flexible Network Binding**: Interface-based binding (`bind_interface`), optional TLS via `axum-server` with rustls, and `advertise_url` for agent card generation.
- **Approval Workflow Loop Integration**: Agent pauses on gated tool calls, publishes `pending_approval` events via WebSocket, and resumes after admin approve/deny. Dashboard "Approvals" panel with real-time updates.
- **Browser as Agent Tool**: `BrowserTool` adapter wrapping the 12-action `ironclad-browser` crate, registered in `ToolRegistry`. Tool schemas injected into system prompt so the LLM can request browser actions.
- **Context Observatory**: Full turn inspector and analytics suite:
  - Turn recording with `context_snapshots` table capturing token allocation, memory tier breakdown, complexity level, and model for every LLM call
  - Turn & Context API: `GET /api/sessions/{id}/turns`, `GET /api/turns/{id}`, `GET /api/turns/{id}/context`, `GET /api/turns/{id}/tools`
  - Dashboard per-message context expansion (token allocation bar, memory breakdown, reasoning trace, tool calls)
  - Context Explorer tab with session selector, turn timeline, and aggregate charts
  - Heuristic context analyzer with 12 per-turn rules and 10 session-aggregate rules across Budget, Memory, Prompt, Tools, Cost, and Quality categories
  - LLM-powered deep analysis stub for on-demand qualitative context evaluation
  - Prompt efficiency metrics per model: output density, budget utilization, memory ROI, cache hit rate, context pressure, cost attribution
  - Efficiency dashboard with model comparison cards, time series charts, period selector, and auto-generated cost optimization tips
  - Outcome grading: 1-5 star ratings on assistant responses via `turn_feedback` table, with quality-adjusted metrics (cost per quality point, quality by complexity, memory impact analysis)
  - Behavioral recommendations engine: ~14 heuristic rules across 7 categories (query crafting, model selection, session management, memory leverage, cost optimization, tool usage, configuration) with evidence and estimated impact
- **Streaming LLM Responses**: `SseChunkStream` adapter for token-by-token streaming. `POST /api/agent/message/stream` SSE endpoint. WebSocket forwarding via EventBus. Dashboard incremental rendering with typing indicator.
- **New reference documents**: `docs/CONFIGURATION.md`, `docs/CLI.md`, `docs/API.md`, `docs/DEPLOYMENT.md`, `docs/ENV.md`

### Changed

- All 10 crate READMEs updated to v0.5.0 with expanded descriptions and key types
- All 10 `lib.rs` files now have `//!` crate-level doc comments
- 10 new dataflow diagrams added to `ironclad-dataflow.md` (approval, browser, context, transform, streaming, addressability, observatory, plugin SDK, OAuth, channel lifecycle)
- 6 new sequence diagrams added to `ironclad-sequences.md` (approval, streaming, turn recording, grading, TLS, CDP)
- All 6 C4 component diagrams updated with ~40 previously undocumented modules
- Documentation standards added to CONTRIBUTING.md
- `cargo doc` CI gate added with `-D warnings` to prevent future documentation drift

## [0.4.3] - 2026-02-23

### Added

- Slash commands for agent chat: `/model`, `/models`, `/breaker`, `/retry` for runtime LLM control
- Runtime model override via `/model set <model>` — temporarily forces a specific model, bypassing routing
- Circuit breaker status and reset via `/breaker` and `/breaker reset [provider]` slash commands
- Breaker-aware model routing — `select_for_complexity` and `select_cheapest_qualified` now skip providers with tripped circuit breakers
- Pre-flight API key check in `infer_with_fallback` — cloud providers with no configured key are skipped before sending a doomed request
- Dashboard settings inputs show a dimmed "none" placeholder instead of literal "null" for empty fields

### Fixed

- Credit/billing errors now permanently trip the circuit breaker (no auto-recovery to HalfOpen) — providers with exhausted credits are never probed again until explicitly reset via `/breaker reset`
- Dashboard "Save to keystore" button now sends `Content-Type: application/json` header — previously failed with "Expected request with 'Content-Type: application/json'"
- Settings form no longer renders `"null"` as a literal value in input fields; empty fields display a styled placeholder and save as `null`

### Changed

- Merged "Roster" and "Agents" into a single "Agents" page with tabbed Roster/List views
- Removed CLI typing sound effects (`start_typing_sound` / `SoundHandle`) from banner rendering

## [0.4.2] - 2026-02-23

### Fixed

- `ironclad daemon start` now verifies the service is actually running after `launchctl load` — previously reported "Daemon started" even when the service crashed immediately
- `ironclad daemon install` resolves the config path to absolute before embedding in the plist — previously used the relative path which launchd couldn't resolve
- Captures launchctl stderr and checks `LastExitStatus` / PID to give actionable error messages on daemon start failure

## [0.4.1] - 2026-02-23

### Added

- `ironclad daemon start|stop|restart` subcommands for full daemon lifecycle management
- Interactive prompt after `ironclad daemon install` asking whether to start immediately
- `--start` flag on `ironclad daemon install` for non-interactive use
- Dashboard keystore management: save/remove provider API keys from the settings page
- Session nicknames in dashboard sessions table with click-to-copy session ID

### Fixed

- Replaced stale `[providers.local]` (localhost:8080) with `[providers.moonshot]` in bundled and registry provider configs
- Added `moonshot/kimi-k2.5` to dashboard known-models list for settings autocomplete
- `ironclad daemon install` now actually offers to load the service (previously only wrote the plist/unit file)
- `ironclad daemon uninstall` now stops the running service before removing the file
- `ironclad daemon status` distinguishes between "not installed" and "installed but not running"
- Registry URL restored to correct `roboticus.ai/registry` path (not subdomain)
- Empty env vars no longer falsely reported as "configured" in key status checks

### Security

- `delete_provider_key` endpoint now validates provider exists before allowing keystore deletion
- Unified key resolution via `KeySource` enum eliminates 3 duplicated cascade implementations
- `resolve_provider_key` returns `Option<String>` instead of silently sending empty auth headers
- Replace secret-looking test placeholders to prevent false GitGuardian alerts

## [0.4.0] - 2026-02-23

### Added

- Signal channel adapter backed by signal-cli JSON-RPC daemon (`ironclad-channels::signal`)
- Unified thinking indicator (🤖🧠…) for all chat channels (Telegram, WhatsApp, Discord, Signal)
- Configurable `thinking_threshold_seconds` on `[channels]` — estimated latency gate for thinking indicator (default: 30s)
- `send_typing` and `send_ephemeral` on WhatsApp and Discord adapters
- Latency estimator based on model tier, input length, and circuit-breaker state
- LLM fallback chain: `infer_with_fallback` helper retries across configured providers on transient errors
- Permanent error detection in delivery queue — 403/401/400 and "bot blocked" errors dead-letter immediately
- Config auto-discovery: `ironclad start` checks `~/.ironclad/ironclad.toml` when no `--config` flag is given
- Obsidian vault integration module with read, search, and write tools
- GitHub Actions release workflow for cross-platform binaries and crates.io publishing

### Changed

- `thinking_threshold_seconds` moved from per-channel (`TelegramConfig`) to `ChannelsConfig` level
- Channel message processing is now platform-agnostic via `send_typing_indicator` / `send_thinking_indicator` helpers
- Delivery queue `mark_failed` checks for permanent errors before scheduling retries
- Channel router `send_to` and `drain_retry_queue` skip retry enqueue for permanent errors
- Circuit breaker test updated to reflect fallback-first behavior

### Fixed

- LLM inference no longer returns a static error when the primary provider is down — falls through to configured fallbacks
- Telegram bot no longer retries messages to chats it was removed from (permanent error dead-lettering)

## [0.3.0] - 2026-02-23

### Security

- Plugin sandbox: validate tool names against allowlist; reject path-traversal payloads; add `shutdown_all` for graceful teardown
- Browser restrictions: block `file://`, `javascript:`, `data:` URI schemes in CDP navigation; harden Chrome launch flags
- Session role validation: reject messages with roles outside `{user, assistant, system, tool}`
- Channel message authority: trusted sender IDs config for elevated `ChannelAuthority`
- WhatsApp webhook signature verification via HMAC-SHA256
- Docker: run as non-root `ironclad` user
- Wallet: encrypt private keys with machine-derived passphrase; never store plaintext
- API key `#[serde(skip_serializing)]` prevents accidental serialization leakage

### Fixed

- Telegram adapter now processes all updates in a batch, not just the first
- Cron worker dispatches jobs instead of unconditionally marking success
- Cron expressions use the `cron` crate for full syntax support (ranges, lists, steps)
- Per-IP rate-limit HashMap evicted on window reset, preventing unbounded growth
- Interview sessions capped at 100 with 1-hour TTL; expired sessions evicted
- `Cargo.lock` committed; CI builds use `--locked` for reproducible builds
- Graceful shutdown handler (SIGINT + SIGTERM) via `with_graceful_shutdown()`
- Duplicate migration version numbers renumbered to unique sequential IDs
- Migrations wrapped in transactions for atomicity
- SQL `LIKE` patterns escape user-supplied wildcards
- Memory query endpoints clamp limit to 1000

### Changed

- Deduplicated `Optional<T>` trait across 5 DB modules; use `rusqlite::OptionalExtension`
- `SessionStatus` and `MessageRole` enums added for future type-safe migration
- Regex allocation in `decode_common_encodings` hoisted to static `LazyLock`
- Silent `.ok()` calls in `ingest_turn()` replaced with `tracing::warn!` logging
- Reusable `reqwest::Client` stored in `Wallet` for connection pooling
- A2A sessions made private with TTL eviction and 256-session cap
- Plugin registry releases lock before tool execution (`Arc<Mutex<Box<dyn Plugin>>>`)
- `CdpSession::set_timeout` now functional (was a documented no-op)
- Daemon logs written to `~/.ironclad/logs/` instead of world-readable `/tmp/`
- Deduplicated `collect_string_values` across policy rules

### Added

- Pre-commit hook for fast format checks (`hooks/pre-commit`)

## [0.2.0] - 2026-02-21

Initial release with core agent runtime, memory tiers, wallet integration,
channel adapters, browser automation, plugin SDK, and scheduling.

## [0.1.0] - 2026-02-22

### Added

- Initial Project Roboticus baseline for Ironclad.
- Multi-crate Rust workspace foundation (runtime crates + integration test crate).
- Core SQLite persistence layer with schema/migrations and operational defaults.
- Early HTTP API, CLI surface, and embedded dashboard scaffolding.
- Initial architecture and reference documentation set.

### Changed

- Prepared packaging/publish metadata for early release workflows.

### Fixed

- Early release stabilization fixes for binary packaging, startup wiring, and quality gates.
