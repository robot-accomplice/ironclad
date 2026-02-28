# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- **Subagent model assignment options**: Added support for `auto` (router-controlled) and `commander` (primary-agent-assigned) model modes for taskable subagents, including runtime model resolution behavior.
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
