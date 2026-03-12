# Wiring Validation Report — v0.8.0

> Historical audit snapshot. Findings here describe the codebase at audit time and may be superseded by later remediation.

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Per-edge contract assessment across crate dependency graph.

## Methodology

For each dependency edge (producer -> consumer), the following 5-point checklist was evaluated:

1. **Traits** -- Are all required methods implemented? Any `todo!()` or `unimplemented!()`?
2. **Errors** -- Does the consumer handle all error variants? Any silent swallowing?
3. **State** -- Does the consumer assume config fields/state the producer guarantees?
4. **Concurrency** -- Does the consumer respect Send/Sync/locking contracts?
5. **Lifecycle** -- Does the consumer depend on producer initialization order?

Rating: PASS = no issues, CONCERN = potential issue worth noting, BREAK = contract violation.

## Edge Summary

| # | Producer -> Consumer | Traits | Errors | State | Concurrency | Lifecycle | Status |
|---|---------------------|--------|--------|-------|-------------|-----------|--------|
| 1 | core -> db | PASS | PASS | PASS | PASS | PASS | PASS |
| 2 | core -> llm | PASS | PASS | PASS | PASS | PASS | PASS |
| 3 | core -> agent | PASS | PASS | PASS | PASS | PASS | PASS |
| 4 | core -> wallet | PASS | PASS | PASS | PASS | PASS | PASS |
| 5 | core -> channels | PASS | PASS | PASS | PASS | PASS | PASS |
| 6 | core -> schedule | PASS | PASS | PASS | PASS | PASS | PASS |
| 7 | core -> plugin-sdk | PASS | PASS | PASS | PASS | PASS | PASS |
| 8 | core -> browser | PASS | PASS | PASS | PASS | PASS | PASS |
| 9 | core -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 10 | db -> agent | PASS | CONCERN | PASS | PASS | PASS | CONCERN |
| 11 | db -> channels | PASS | PASS | PASS | PASS | PASS | PASS |
| 12 | db -> wallet | PASS | PASS | PASS | PASS | PASS | PASS |
| 13 | db -> schedule | PASS | PASS | PASS | PASS | PASS | PASS |
| 14 | db -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 15 | llm -> agent | PASS | PASS | PASS | PASS | PASS | PASS |
| 16 | llm -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 17 | agent -> schedule | PASS | PASS | PASS | PASS | PASS | PASS |
| 18 | agent -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 19 | wallet -> schedule | PASS | PASS | PASS | PASS | PASS | PASS |
| 20 | wallet -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 21 | schedule -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 22 | channels -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 23 | plugin-sdk -> server | PASS | PASS | PASS | PASS | PASS | PASS |
| 24 | browser -> server | PASS | PASS | PASS | PASS | PASS | PASS |

**Totals: 24 edges validated, 23 PASS, 1 CONCERN, 0 BREAK**

## Detailed Findings

### Edge 1: core -> db

**Imports**: `IroncladError`, `Result` (used in all 18 db modules)

- **Traits**: No trait contracts. db uses core's error/result types directly. No `todo!()` or `unimplemented!()` in any db module.
- **Errors**: All rusqlite errors converted to `IroncladError::Database(e.to_string())` via explicit `map_err`. No silent swallowing in production code. The `.ok()` calls in `sessions.rs:129,152` and `delivery_queue.rs:26,29` are intentional patterns (find-or-create, timestamp parsing fallback).
- **State**: `Database::new()` takes a path string; no config struct dependency beyond what core provides.
- **Concurrency**: `Database` wraps `Arc<Mutex<Connection>>` which is auto `Send + Sync`. The `conn()` method uses poisoned-mutex recovery (`unwrap_or_else(|e| e.into_inner())`), which is appropriate for SQLite WAL mode.
- **Lifecycle**: `Database::new()` calls `schema::initialize_db()` internally -- no external ordering requirement.

### Edge 2: core -> llm

**Imports**: `IroncladConfig`, `IroncladError`, `Result`, `ApiFormat`, `ModelTier`, `config::ProviderConfig`, `config::RoutingConfig`, `config::CircuitBreakerConfig`, `config::TierAdaptConfig`, `config::MemoryConfig`

- **Traits**: No trait contracts between core and llm. All types are data structs.
- **Errors**: `LlmService::new()` propagates all errors with `?`. `SseChunkStream::poll_next` correctly creates `IroncladError::Llm` and `IroncladError::Network` variants.
- **State**: All config field accesses (`config.cache.*`, `config.circuit_breaker`, `config.models.*`, `config.providers`, `config.memory`) verified to exist in `IroncladConfig`. One concern: `CircuitBreakerConfig::credit_cooldown_seconds` is defined but never read by `circuit.rs` (already tracked as BUG-057).
- **Concurrency**: `LlmService` is used behind `Arc<tokio::sync::RwLock<>>` in server. All fields are `Send + Sync`.
- **Lifecycle**: `LlmService::new()` depends only on `&IroncladConfig`, not on Database or other services. Initialization order is correct.

### Edge 3: core -> agent

**Imports**: `IroncladError`, `Result`, `RiskLevel`, `InputAuthority`, `PolicyDecision`, `SurvivalTier`, `ModelTier`, `SkillManifest`, `InstructionSkill`, `SkillTrigger`, `config::ObsidianConfig`, `config::MemoryConfig`, `config::SessionConfig`, `config::ApprovalsConfig`, `config::SkillsConfig`, `config::McpTransport`, `config::DigestConfig`, `input_capability_scan`

- **Traits**: No trait contracts. All types are data enums/structs.
- **Errors**: All cross-crate errors properly propagated with `?` or logged with `tracing::warn!`. The `memory::ingest_turn()` function returns `()` rather than `Result` -- this is a deliberate design choice since memory ingestion is best-effort and should not fail the main agent turn.
- **State**: All config field accesses verified. `is_safe_for_speculation()` correctly pattern-matches `RiskLevel::Safe`.
- **Concurrency**: Agent types used behind `Arc<RwLock<>>` in server where needed.
- **Lifecycle**: Agent initialization does not depend on specific service init order beyond having config available.

### Edge 4: core -> wallet

**Imports**: `IroncladError`, `Result`, `config::IroncladConfig`, `config::WalletConfig`, `config::TreasuryConfig`, `config::YieldConfig`

- **Traits**: No trait contracts.
- **Errors**: `WalletService::new()` propagates with `?`. `Wallet::load_or_generate()` properly maps IO/crypto errors to `IroncladError::Wallet`.
- **State**: All config field accesses verified (`config.wallet`, `config.treasury`, `config.r#yield`).
- **Concurrency**: `WalletService` is wrapped in `Arc<>` in server/schedule. All fields (`Wallet`, `TreasuryPolicy`, `YieldEngine`) are `Send + Sync` (verified: contains String, PathBuf, reqwest::Client, etc.).
- **Lifecycle**: `WalletService::new()` is async (generates HD wallet if needed) but depends only on `&IroncladConfig`.

### Edge 5: core -> channels

**Imports**: `IroncladError`, `Result`, `config::A2aConfig`

- **Traits**: `ChannelAdapter` trait requires `Send + Sync`. All 6 implementations (Telegram, WhatsApp, Discord, Signal, Email, WebSocket) verified.
- **Errors**: All network/API errors converted to `IroncladError::Channel` or `IroncladError::Network`. No silent swallowing.
- **State**: A2A config access verified. Channel adapters depend on per-channel config structs (TelegramConfig, etc.) which are Optional fields in `ChannelsConfig`.
- **Concurrency**: All adapters use `Mutex` for internal state (message buffers, sequence numbers). Discord adapter uses `expect("mutex poisoned")` which is acceptable since it would only panic on a logic bug.
- **Lifecycle**: Channel adapters are created during bootstrap after config is loaded.

### Edge 6: core -> schedule

**Imports**: `SurvivalTier`, `config::SessionConfig`

- **Traits**: No trait contracts.
- **Errors**: Not applicable -- schedule only reads config types.
- **State**: `SurvivalTier` used for heartbeat interval adjustment. `SessionConfig` passed to `SessionGovernor`. Both are simple data types.
- **Concurrency**: No shared state with core.
- **Lifecycle**: No dependency on core initialization beyond config availability.

### Edge 7: core -> plugin-sdk

**Imports**: `Result`, `RiskLevel`, `IroncladError`

- **Traits**: `Plugin` trait defined in plugin-sdk requires `Send + Sync`. `ScriptPlugin` implements it. `ToolDef` uses `RiskLevel` from core.
- **Errors**: `ScriptPlugin::execute_tool` returns `Result<ToolResult>`. Errors properly constructed as `IroncladError::Tool`.
- **State**: `RiskLevel` mapping in `ScriptPlugin::tools()` correctly maps `dangerous: bool` to `RiskLevel::Dangerous` / `RiskLevel::Caution`.
- **Concurrency**: `Plugin: Send + Sync` bound is explicit and enforced.
- **Lifecycle**: No ordering dependency.

### Edge 8: core -> browser

**Imports**: `IroncladError`, `Result`, `config::BrowserConfig`

- **Traits**: No trait contracts.
- **Errors**: `Browser::start()` properly constructs `IroncladError::Tool` for all failure modes (no targets, no page target, no WebSocket URL).
- **State**: `BrowserConfig` fields (`enabled`, `headless`, `cdp_port`, `executable_path`) all accessed correctly.
- **Concurrency**: `Browser` uses `tokio::sync::RwLock` for internal state. `SharedBrowser` is `Arc<Browser>`.
- **Lifecycle**: `Browser::new()` is a simple constructor. `Browser::start()` is the async initializer called after construction.

### Edge 9: core -> server

**Imports**: `IroncladConfig`, `IroncladError`, `Result`, `Keystore`, various config sub-structs, domain types

- **Traits**: No trait contracts.
- **Errors**: `bootstrap_with_config_path` propagates all errors with `?`. Non-critical failures (nickname backfill, sub-agent registration) are logged as warnings and continue.
- **State**: Server reads all config sections. `config.validate()` is called before bootstrap.
- **Concurrency**: `IroncladConfig` is `Clone` and passed by value to bootstrap. `Keystore` is wrapped in `Arc`.
- **Lifecycle**: Server is the top-level orchestrator. Initialization order is: config -> db -> llm -> wallet -> plugins -> browser -> channels -> server bind. This ordering is correct.

### Edge 10: db -> agent (CONCERN)

**Imports**: `Database`, `sessions::*`, `memory::*`, `embeddings::*`, `ann::AnnIndex`, `efficiency::*`

- **Traits**: No trait contracts.
- **Errors**: `memory::ingest_turn()` returns `()` and logs all db errors with `tracing::warn!`. This is intentional (best-effort memory), but means 5 separate `store_*` calls silently degrade if db is locked or full. `digest::persist()` and `governor::tick()` properly return `Result`.
- **State**: Agent assumes `Database` is initialized (schema created). This is guaranteed by `Database::new()` calling `schema::initialize_db()`.
- **Concurrency**: `Database.conn()` uses poisoned-mutex recovery. Agent code never holds a lock across await points (all db calls are synchronous within a single `conn()` lock acquisition).
- **Lifecycle**: Agent modules receive `&Database` as a parameter -- no initialization order concern.

**CONCERN**: `ingest_turn()` silently degrades on 5 independent db operations. If the SQLite database is corrupted or disk is full, all memory storage will silently fail with only log warnings. This is by design (best-effort memory) but worth documenting. Not a wiring break.

### Edge 11: db -> channels

**Imports**: `Database`, `delivery_queue as dq_store`

- **Traits**: No trait contracts.
- **Errors**: `DeliveryProcessor::process_pending_deliveries()` handles delivery failures with retry logic. Errors propagated correctly.
- **State**: Channels depend on `delivery_queue` table existing, which is created in `schema::initialize_db()`.
- **Concurrency**: `ChannelRouter` stores `Database` (Clone, Arc-based). No lock contention.
- **Lifecycle**: `ChannelRouter::with_store(db.clone())` called during bootstrap after db init.

### Edge 12: db -> wallet

- **Traits**: No trait contracts. Wallet does not import db directly.
- Note: Wallet reads config from core, not db state. No wiring edge here despite Cargo dependency.
  The Cargo dependency exists because `ironclad-wallet/Cargo.toml` declares `ironclad-db.workspace = true`, but code-level analysis shows wallet modules do not import `ironclad_db` (only `ironclad_core`). The Cargo dependency may be vestigial or used for transitive features.

### Edge 13: db -> schedule

**Imports**: `Database`, `cron::*`, `metrics::*`, `sessions::*`

- **Traits**: No trait contracts.
- **Errors**: `run_cron_worker()` logs all db errors and continues the loop. `execute_cron_job()` returns `("error", Some(msg))` tuples for db failures, which get recorded via `cron::record_run()`. All error paths handled.
- **State**: Schedule depends on `cron_jobs`, `cron_runs`, `transactions`, `metric_snapshots`, `sessions` tables. All created in `schema::initialize_db()`.
- **Concurrency**: `Database` passed by value (Clone) to cron worker. No lock contention.
- **Lifecycle**: Cron worker receives `Database` after bootstrap; no ordering concern.

### Edge 14: db -> server

**Imports**: `Database`, plus all db modules used transitively through other crates

- **Traits**: No trait contracts.
- **Errors**: Bootstrap propagates `Database::new()` errors. Route handlers use `?` on db operations.
- **State**: Server calls `Database::new()` which handles all schema initialization.
- **Concurrency**: Single `Database` instance cloned to all components. `Arc<Mutex<Connection>>` provides thread safety.
- **Lifecycle**: Database is the first service initialized (step 4 in bootstrap, after config/logging).

### Edge 15: llm -> agent

**Imports**: `format::UnifiedMessage`

- **Traits**: No trait contracts. `UnifiedMessage` is a data struct.
- **Errors**: Not applicable -- only type import.
- **State**: `UnifiedMessage` fields used correctly in `context.rs` and `governor.rs`.
- **Concurrency**: `UnifiedMessage` is `Clone + Debug` (data struct).
- **Lifecycle**: No dependency.

### Edge 16: llm -> server

**Imports**: `LlmService`, `SemanticCache`, `ModelRouter`, `CircuitBreakerRegistry`, etc.

- **Traits**: No trait contracts.
- **Errors**: `LlmService::new()` propagated with `?`. Streaming errors handled correctly.
- **State**: Server constructs `LlmService` from `&IroncladConfig`.
- **Concurrency**: `LlmService` wrapped in `Arc<tokio::sync::RwLock<>>` in server routes.
- **Lifecycle**: LLM initialized after config, before agent loop.

### Edge 17: agent -> schedule

**Imports**: `governor::SessionGovernor`

- **Traits**: No trait contracts.
- **Errors**: `governor.tick()` returns `Result`, handled with match in heartbeat loop.
- **State**: `SessionGovernor` constructed with `SessionConfig` from core.
- **Concurrency**: `SessionGovernor` used synchronously within heartbeat loop.
- **Lifecycle**: Governor constructed inside heartbeat `run()` function.

### Edge 18: agent -> server

**Imports**: `retrieval::MemoryRetriever`, `discovery::DiscoveryRegistry`, `device::DeviceManager`, `mcp::McpClientManager`, `mcp::McpServerRegistry`, `subagents::SubagentRegistry`, `governor::SessionGovernor`, various types

- **Traits**: No trait contracts.
- **Errors**: All agent operations in server routes use `?` propagation or explicit error handling.
- **State**: Agent types constructed during bootstrap with appropriate configs.
- **Concurrency**: All shared agent types wrapped in `Arc<RwLock<>>`.
- **Lifecycle**: Agent components initialized during bootstrap in correct order.

### Edge 19: wallet -> schedule

**Imports**: `WalletService` (via `Arc`)

- **Traits**: No trait contracts.
- **Errors**: `wallet.wallet.get_usdc_balance().await.unwrap_or(0.0)` -- network failure defaults to 0 balance. `yield_engine.get_a_token_balance().await.ok().unwrap_or(0.0)` -- same pattern. These are acceptable for a heartbeat context (fallback to safe zero).
- **State**: Wallet service must be initialized before heartbeat starts.
- **Concurrency**: `Arc<WalletService>` -- all wallet methods are `&self`.
- **Lifecycle**: Wallet initialized before heartbeat daemon in bootstrap.

### Edge 20-24: wallet/schedule/channels/plugin-sdk/browser -> server

All follow the same pattern:
- Types constructed during bootstrap
- Wrapped in `Arc` or `Arc<RwLock>` for shared access
- Errors propagated with `?`
- No trait contract violations
- Correct initialization ordering in bootstrap

## Cross-Cutting Observations

### Pattern: Consistent Error Wrapping
All crates use `IroncladError` variants correctly:
- db uses `IroncladError::Database`
- llm uses `IroncladError::Llm` and `IroncladError::Network`
- channels uses `IroncladError::Channel` and `IroncladError::Network`
- wallet uses `IroncladError::Wallet`
- schedule uses `IroncladError::Schedule`
- browser uses `IroncladError::Tool`
- agent uses appropriate variants based on context

### Pattern: No `todo!()` or `unimplemented!()`
Zero instances found across all 10 crates in non-test code.

### Pattern: Controlled Unwrap Usage
All `unwrap()`/`expect()` calls in production code fall into safe categories:
- Regex compilation on compile-time-known patterns (agent/injection.rs, obsidian.rs)
- HMAC/HKDF operations that cannot fail per spec (prompt.rs, wallet.rs, a2a.rs)
- HTTP client builders (knowledge.rs, whatsapp.rs, cdp.rs)
- Post-guard-check safe unwraps (sessions.rs:558, wasm.rs:225, whatsapp.rs:232)
- Static error body serialization (auth.rs, rate_limit.rs)

### Pattern: Poisoned Mutex Recovery
Both `Database::conn()` and `TelegramAdapter::recv()` use `unwrap_or_else(|e| e.into_inner())` for poisoned mutex recovery. This is appropriate for SQLite connections and message buffers where the data is still valid even after a panic.

### Vestigial Dependency: db in wallet Cargo.toml
`ironclad-wallet/Cargo.toml` declares `ironclad-db.workspace = true` but no wallet source file imports `ironclad_db`. This may be a leftover from a previous version or used for transitive feature resolution.

## New Bugs Filed

| ID | Edge | Category | Severity | Description |
|----|------|----------|----------|-------------|
| BUG-059 | db -> wallet | wiring concern | Low | `ironclad-wallet/Cargo.toml` declares `ironclad-db` dependency but no source file imports `ironclad_db`; vestigial Cargo dependency adds unnecessary compile-time coupling |
| BUG-060 | db -> agent | wiring concern | Low | `agent::memory::ingest_turn()` returns `()` and silently degrades on 5 independent db `store_*` calls; if SQLite is corrupted/full, all memory storage fails with only log warnings; consider returning a count of successful stores |
