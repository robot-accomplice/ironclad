# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
