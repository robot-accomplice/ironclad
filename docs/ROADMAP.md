# Ironclad Roadmap

*Growth areas organized by effort and impact. Each item notes whether it builds on existing code or is greenfield.*

---

## Tier 1 — Wire the Last Mile

Capabilities where the core code exists but isn't fully connected. High impact, low-to-medium effort.

### 1.1 Streaming LLM Responses ✅

**Status**: Implemented in 0.5.0. `POST /api/agent/message/stream` returns an SSE stream of tokens as they arrive from the provider. `forward_request()` supports streaming mode via `reqwest` byte streams. Partial responses are not cached; caching occurs only on stream completion. WebSocket subscribers receive real-time token events.

---

### 1.2 Approval Workflow API ✅

**Status**: Implemented in 0.5.0. Routes: `GET /api/approvals`, `POST /api/approvals/{id}/approve`, `POST /api/approvals/{id}/deny`. Pending approvals are pushed via WebSocket. The agent loop pauses on gated tool calls and resumes on approval or denial. `approval_requests` table tracks full lifecycle with timeout enforcement.

---

### 1.3 Browser as Agent Tool ✅

**Status**: Implemented in 0.5.0. `BrowserTool` wraps CDP actions (navigate, click, type, screenshot, evaluate) as `Tool` trait methods. Registered in `ToolRegistry` under the `general` category. Policy: `RiskLevel::Caution` by default, `Dangerous` for `Evaluate` (arbitrary JS execution). The agent can autonomously browse the web during the ReAct loop.

---

### 1.4 Discord WebSocket Gateway

**Current state**: Discord adapter handles message parsing, REST send, guild allowlists, rate limiting, and chunking. Missing: persistent WebSocket connection to the Discord Gateway for receiving real-time events.

**Target**: Full bidirectional Discord integration — receive messages in real-time without polling.

**Builds on**: `ironclad-channels/discord.rs`.

**Scope**: Implement Gateway identify, heartbeat, and dispatch event handling. Resume/reconnect on disconnect. Wire `MESSAGE_CREATE` events into the existing `parse_inbound()` path.

---

### 1.5 Embedding Provider Integration ✅

**Status**: Implemented. `ironclad-llm/embedding.rs` provides `EmbeddingClient` with support for OpenAI, Ollama, and Google embedding APIs. Configuration via `embedding_path`, `embedding_model`, and `embedding_dimensions` on `ProviderConfig`. N-gram fallback when no provider is configured. Integrated into `LlmService` via `resolve_embedding_config()`.

---

### 1.6 Multimodal Message Handling

**Current state**: WhatsApp adapter parses image/video/audio/document types but converts them to text placeholders (`[image:id] caption`). The LLM client has no vision or multimodal support. Voice messages are silently ignored.

**Target**: Forward images to vision-capable models. Transcribe voice messages to text. Store media references in session history. Display images in dashboard.

**Builds on**: WhatsApp media parsing, Telegram media API, `ironclad-llm/format.rs` (format translation already handles content arrays).

**Scope**: Download media from channel APIs to a configurable `media_dir` with content-addressed filenames and automatic size-based cleanup policy. Construct multimodal content blocks (`image_url` for OpenAI, inline `image` for Anthropic). Gate behind a config flag (`models.multimodal = true`). Extend `UnifiedMessage` to carry binary content parts. Add Whisper-compatible speech-to-text for voice messages — prefer native Rust via `whisper-rs` for local-first transcription, with cloud STT API as an opt-in fallback. Store transcripts in session history alongside the original audio reference.

---

### 1.7 Memory-Augmented Agent Pipeline ✅

**Status**: Implemented. The `agent_message` handler now:

1. Generates a query embedding via `EmbeddingClient`
2. Calls `MemoryRetriever::retrieve()` for 5-tier hybrid retrieval (FTS5 + vector cosine)
3. Loads conversation history from `sessions::list_messages()`
4. Assembles context via `build_context(level, system_prompt, memories, history)`
5. After response: background `ingest_turn()` + embedding generation for the assistant response

---

### 1.8 Email Channel Adapter

**Current state**: Ironclad supports Telegram, WhatsApp, Discord, and WebSocket channels. No email channel exists — the agent cannot send or receive email.

**Target**: Full bidirectional email integration. The agent can receive emails, participate in threaded conversations, and send replies — all through the existing `ChannelAdapter` infrastructure.

**Builds on**: `ChannelAdapter` trait, `ChannelRouter`, `OAuthManager` from `ironclad-llm/oauth.rs`, injection defense pipeline.

**Scope**: Implement `EmailAdapter` in `ironclad-channels/src/email.rs`. Native IMAP inbound via `async-imap` with IDLE push for real-time delivery. Native SMTP outbound via `lettre`. OAuth2 for Gmail using the existing `OAuthManager`; app-password support for local mail bridges. Thread-aware session mapping — email threads map to Ironclad sessions via `Message-ID` / `In-Reply-To` headers, giving the agent conversational continuity across email chains. DKIM signature verification on inbound for anti-spoofing (feeds into the injection defense pipeline). Config: `[channels.email]` section with provider presets.

---

### 1.9 Session Scoping and Lifecycle ✅

**Status**: Implemented in 0.6.0. Scoped sessions now support `Agent`, `Peer`, and `Group` keys with lifecycle controls. Runtime paths (web, stream, channel) resolve scope before `find_or_create()`, stale sessions are expired by `SessionGovernor`, and optional scheduled rotation is wired through heartbeat when `session.reset_schedule` is set.

**Target**: Per-peer and per-group session isolation with configurable lifecycle policies. Cross-channel identity linking via shared peer IDs.

**Builds on**: `sessions.rs`, `schema.rs`, `HeartbeatTask` enum, `context.rs` compaction pipeline.

**Scope**: `SessionScope` is active in runtime routing, dedup is chat-aware in channel paths, and migration `012_session_scope_backfill_unique.sql` normalizes legacy unscoped rows (`scope_key IS NULL`) to explicit `agent` scope while enforcing one active session per `(agent_id, scope_key)`.

---

### 1.10 Addressability Filter ✅

**Status**: Implemented in 0.5.0. `MessageFilter` trait with `MentionFilter`, `ReplyFilter`, and `ConversationFilter` composed through `FilterChain`. Runtime uses `default_addressability_chain(agent_name)` in channel paths; DMs bypass filtering.

---

### 1.11 Context Checkpoint

**Current state**: The agent rebuilds its full context from the database on every boot — querying memory tiers, retrieving embeddings, assembling the system prompt. This cold-start path adds latency before the agent is responsive.

**Target**: Instant agent readiness on boot via a transactional checkpoint that captures the agent's compiled context state.

**Builds on**: `schema.rs`, `build_context()`, `MemoryRetriever`, boot sequence in `lib.rs`.

**Scope**: Add a `checkpoints` table to the schema. Define a `ContextCheckpoint` struct (with `serde`) containing the compiled system prompt, top-k memory summaries, active task list, and recent conversation digest. Write checkpoints every N turns (configurable) and on graceful shutdown — crash recovery loses at most N turns of checkpoint state while raw data remains in the DB. On boot, `load_checkpoint()` provides instant agent readiness while background retrieval warms the full context asynchronously. Checkpoints are versioned for forward compatibility. Config: `[context.checkpoint]` with `interval_turns` and `enabled`.

---

### 1.12 Response Transform Pipeline ✅

**Status**: Implemented in 0.5.0. `ResponsePipeline` with `ResponseTransform` trait. Ships three transforms: `ReasoningExtractor` (extracts `<think>`/`<reasoning>` blocks and logs them to `turns.reasoning_trace`), `FormatNormalizer` (consistent response structure across providers), and `ContentGuard` (reflected injection defense via existing output scanning). Transforms are currently applied through the default pipeline wiring.

---

### 1.13 Context Observatory ✅

**Status**: Implemented in 0.5.0. The Context Observatory provides runtime visibility into context assembly efficiency, inference cost attribution, and output quality.

**Components**:

- `ironclad-db/efficiency.rs` — per-model efficiency analytics: output density, budget utilization, memory ROI, system prompt weight, cache hit rate, context pressure rate, cost attribution (system prompt / memories / history), wasted budget tracking
- `turn_feedback` table — stores outcome grades (thumbs up/down + comment) per turn; feeds quality metrics with complexity breakdown and memory impact analysis
- REST API — `GET /api/stats/efficiency`, `GET /api/recommendations`, `POST /api/recommendations/generate` (LLM-powered deep analysis)
- Trend tracking — sliding-window detection (improving/stable/declining) for cost and quality metrics

---

### 1.14 Capacity-Aware Routing ✅

**Status**: Implemented in 0.6.0. Capacity usage is now recorded on inference success, routing uses headroom-weighted selection, sustained pressure can preemptively mark providers `half_open`, and operators can inspect capacity via `GET /api/stats/capacity` plus dashboard metrics UI.

**Target**: Proactive capacity-aware routing that deprioritizes saturated providers before hitting rate limits, distributing traffic to providers with available headroom.

**Builds on**: `ironclad-llm/src/router.rs`, `ironclad-llm/src/circuit.rs`, `ProviderConfig`.

**Scope**: `CapacityTracker` now emits provider stats (`headroom`, utilization, near-capacity, sustained-hot), router scoring multiplies quality by headroom, and breaker pressure signaling allows proactive lane-shifting before hard failure.

---

### 1.15 Sessions Markdown Rendering ✅

**Status**: Implemented in 0.6.0. Sessions and Context Explorer render markdown via a safe client-side renderer with strict URL sanitization (`http`, `https`, `mailto`, relative/hash only), no raw HTML execution, and preserved fallback behavior for blocked links.

---

### 1.16 Durable Channel Delivery Queue

**Current state**: Channel retry handling is primarily process-memory based. Retries can be lost during crash/restart windows and operators lack a durable backlog for forensic replay.

**Target**: Durable outbound delivery with persisted retry queue, dead-letter handling, and boot-time recovery replay.

**Builds on**: `ironclad-channels` delivery router/retry flow, existing SQLite persistence, channel health metrics.

**Scope**: Persist retryable outbound events to DB with attempt count, next-attempt timestamp, and terminal failure reason. Add dead-letter queue inspection endpoint/CLI command, replay controls, and idempotent delivery keys to avoid duplicate sends after recovery.

---

### 1.17 Production-Grade Abuse Protection

**Current state**: Global rate limiting exists, but trusted proxy boundaries and distributed enforcement are not first-class across all deployment topologies.

**Target**: Hardened anti-abuse controls for internet-facing deployments with trusted proxy semantics, actor-aware quotas, and horizontally safe limit enforcement.

**Builds on**: global rate limiter middleware, auth layer, deployment/network configuration.

**Scope**: Add explicit trusted-proxy CIDR handling for client IP extraction, token/user-level quotas alongside IP limits, optional shared limiter backend for multi-instance deployments, and dashboard/API observability for throttle events and top offenders.

**Release target**: `v0.9.1`. Required before any internet-facing deployment; deliberately sequenced after `v0.9.0` core to avoid scope overload while keeping it on the near-term critical path.

---

### 1.18 Cron-Conformant Session Rotation

**Current state**: Session lifecycle controls exist, but reset scheduling semantics should strictly match configured cron intent in all cases.

**Target**: True cron-conformant rotation behavior with clear operator guarantees and test coverage.

**Builds on**: session governor, scheduler heartbeat tasks, `session.reset_schedule` contract.

**Scope**: Evaluate reset schedules using the configured cron expression directly, add conformance tests for edge schedules/timezones, and document migration notes for any behavior change from legacy timing semantics.

---

### 1.19 `agent-browser` External Runtime Support

**Current state**: Browser automation is implemented via Ironclad's native browser tooling (`1.3`) and does not yet support the external `agent-browser` runtime/CLI contract.

**Target**: Add optional `agent-browser` compatibility so browser actions can run through the external runtime while preserving Ironclad policy controls, provenance, and auditability.

**Builds on**: `1.3 Browser as agent tool`, tool registry, command safety/policy engine, runtime config + dashboard observability.

**Scope**: Introduce a browser backend abstraction with a new `agent-browser` backend implementation. Map the core action set (`open`, `snapshot`, `click`, `fill`, `wait`, `get text`, `screenshot`) to `agent-browser` commands with structured result parsing (`--json` mode). Add config for binary path, provider/session/profile defaults, domain allowlist, and output-size limits. Preserve existing safety rails (`RiskLevel`, approvals where required) and emit clear provenance for backend selection/fallback events. Add integration tests with a mock `agent-browser` binary and release-smoke checks for backend availability.

**Release posture**: Discovery/design can proceed now; implementation is planned for `v0.9.x` and is not part of locked `v0.8.0` release gates.

---

### 1.20 Homebrew & Winget Package Manager Distribution

**Current state**: Ironclad distributes pre-built binaries via GitHub Releases (5 targets), crates.io (source install), Docker image, and platform installer scripts (`install.sh`, `install.ps1`). Users on macOS/Linux must use installer scripts or `cargo install`; Windows users must use the PowerShell script or `cargo install`. No native package manager integration exists.

**Target**: One-command installation via native package managers — `brew install ironclad` (macOS/Linux) and `winget install RobotAccomplice.Ironclad` (Windows). Auto-updated on each release with zero manual intervention.

**Builds on**: Existing `release.yml` binary matrix, SHA256 checksum pipeline, and GitHub Releases infrastructure.

**Scope**: Create `robot-accomplice/homebrew-ironclad` tap repository with a pre-built binary formula covering macOS (x86_64/aarch64) and Linux (x86_64/aarch64). Submit initial `RobotAccomplice.Ironclad` manifest to `microsoft/winget-pkgs` for the Windows zip artifact (portable installer type). Add two new jobs to `release.yml`: `update-homebrew` (pushes updated formula with version + SHA256 to the tap on each release) and `update-winget` (submits a winget-pkgs manifest update PR via `winget-releaser`). Update README installation section to prioritize `brew` and `winget` commands. Requires two new GitHub secrets: `HOMEBREW_TAP_PAT` (push access to tap repo) and `WINGET_TOKEN` (public_repo scope for winget-pkgs PRs).

**Release posture**: Planned for `v0.9.x`. Homebrew tap can be created and populated immediately; winget initial submission requires manual review by Microsoft. Automation jobs activate on next tagged release.

---

### 1.21 Integrations Management (CLI + Dashboard)

**Current state**: Channel integrations (Telegram, WhatsApp, Discord, email) are configured by manually editing `ironclad.toml` and the encrypted keystore. There is no unified CLI or dashboard surface for connecting, testing, inspecting status, or troubleshooting integrations. Operators must read logs to diagnose issues like expired tokens or misconfigured webhook secrets.

**Target**: First-class integration management across CLI and dashboard — connect, test, inspect, and troubleshoot channel integrations without touching config files or logs directly.

**Builds on**: `ironclad-channels` adapters, `ChannelRouter`, encrypted keystore, channel health metrics, dashboard SPA.

**Scope**:

- **CLI flows**: `ironclad integrations list` (status summary per channel), `ironclad integrations test <channel>` (send a health-check probe and report token validity, connectivity, and permissions), `ironclad integrations connect <channel>` (guided setup wizard — prompts for token, stores in keystore, writes config, runs test), `ironclad integrations disconnect <channel>` (removes token from keystore, disables in config).
- **Dashboard UI**: Integrations panel showing per-channel health (connected/degraded/disconnected), last successful message timestamp, error counts, and one-click test button. Connection wizard with token input, validation feedback, and activation toggle.
- **Channel metadata API**: `GET /api/integrations` returning per-channel status (enabled, healthy, last_error, last_message_at, config summary). `POST /api/integrations/<channel>/test` to run a live probe.
- **Context awareness**: Expose active channel identity in the agent's runtime context so the agent knows which channel a message arrived on and can adapt its response format accordingly (e.g., markdown for Telegram, plain text for SMS).
- **Diagnostic surface**: Surface recent channel errors and warnings inline (not buried in logs) — token expiry alerts, rate-limit warnings, webhook misconfiguration hints.

**Release posture**: CLI-first; dashboard UI can follow in a subsequent release. Context awareness is a low-effort high-value sub-item that can ship independently.

---

### 1.22 Built-in Introspection Skill

**Current state**: The agent has no way to query its own runtime state. Channel identity, session scope, memory budget usage, channel health, treasury balance, and active peer sessions are either baked into the system prompt (causing context bloat) or entirely invisible to the agent. The `ToolContext` struct carries only `session_id`, `agent_id`, `authority`, and `workspace_root` — no channel, no runtime metadata.

**Target**: A built-in introspection skill that lets the agent query its own runtime state on demand, without any of that state consuming context tokens until the agent actually needs it. The design principle is **info on demand, not info by default** — the agent calls a tool when it needs context rather than having it pre-injected into every system prompt.

**Builds on**: `Tool` trait, `ToolRegistry`, `ToolContext`, `SessionScope`, `ChannelRouter`, `AppState`.

**Scope**:

- **`get_runtime_context` tool**: Returns session channel (parsed from `scope_key`), session scope type (peer/group/agent), peer/group ID, agent name and ID, active channels list, and server uptime. This is the highest-priority sub-item — it directly addresses the channel awareness gap where the agent cannot determine which channel it's speaking over.
- **`get_memory_stats` tool**: Returns memory budget usage per tier (working, episodic, semantic, procedural, relationship), total entry counts, and embedding store stats.
- **`get_channel_health` tool**: Returns per-channel connection status, message counts, last error, last activity timestamp — surfacing the data from `ChannelRouter` health tracking.
- **`get_treasury_status` tool**: Returns current balance, daily spend, budget remaining, and yield position if active.
- **`get_active_peers` tool**: Returns current A2A peer sessions with connection state.
- **`ToolContext` extension**: Add `channel: Option<String>` to `ToolContext` so any tool (not just introspection) can access the originating channel. Parse from `SessionScope::scope_key()` at context construction time in the agent message handler.
- **Extensibility pattern**: Each introspection tool is a separate `Tool` impl registered in `ToolRegistry`, making it easy to add new introspection surfaces without modifying existing tools. The agent discovers available introspection tools through the standard tool discovery mechanism.

**Design principle**: This pattern — lightweight, on-demand runtime queries — should be the default approach for any agent-visible metadata. Pre-injecting state into system prompts wastes context budget on information the agent may never need. Tools are zero-cost until called.

**Release posture**: `get_runtime_context` is the minimum viable ship (solves channel awareness). Other introspection tools can follow incrementally. Low effort, high impact.

---

### 1.23 Context Budget Tuning

**Current state**: `ComplexityLevel::L0` carries a 4,000-token budget. Duncan's soul (OS.toml + FIRMWARE.toml) consumes ~1,500 tokens of that, leaving only ~2,500 for memories and conversation history. All casual Telegram messages score complexity 0.04–0.06, which maps to L0 unconditionally. This causes personality degradation under sustained low-complexity chat — the soul is present but history is brutally truncated, so the agent loses conversational continuity and its responses feel generic.

**Target**: Three adjustments to ensure the agent's personality and conversational quality survive low-complexity interactions:

1. **Raise L0 token budget** (4,000 → 8,000) — simple constant change in `context.rs::token_budget()`. Doubles headroom for soul + history at minimal cost, since L0 conversations are short and the model can handle 8k easily.
2. **Soul-size-aware complexity floor** — `build_context()` measures the system prompt token count and bumps the effective budget floor so the soul never consumes more than ~40% of the total context. Large personality files automatically get more room.
3. **Channel conversation minimum L1** — Messages arriving via channel adapters (Telegram, Discord, WhatsApp) get a minimum of `ComplexityLevel::L1` (8,000 tokens), since channel conversations carry implicit context expectations. Only direct API calls can land on L0.

**Builds on**: `context.rs` (`determine_level`, `token_budget`, `build_context`), `InboundMessage.platform`, complexity scorer.

**Design note**: These are stopgap measures. The long-term fix is the introspection skill (1.22), which removes the need to pre-load state into the context at all. Once introspection tools are available, the agent can operate on smaller budgets by querying state on demand.

**Release posture**: Parked for `v0.9.0`. Not part of the locked `v0.8.0` release gates.

---

## Tier 2 — New Capabilities

Features that require significant new code but have clear implementation paths. Medium-to-high effort.

### 2.1 ML-Based Model Routing

**Current state**: Heuristic classifier (weighted message length, tool calls, depth) produces a 0.0–1.0 complexity score. Functional but blunt.

**Target**: Logistic regression on prompt embeddings (~11μs overhead) that learns from actual usage which queries need strong vs. weak models. Achieves ~60% cost savings with <5% quality degradation.

**Research basis**: RouteLLM (LMSYS), Section 3.1 of research-alternatives.md.

**Scope**: Train a small classifier on preference data (which model produced better answers). Ship as a serialized model loaded at boot. `HeuristicBackend` becomes one of two `RouterBackend` implementations. Config: `models.routing.mode = "ml"` activates the trained classifier.

---

### 2.2 Accuracy-Target Routing

**Current state**: The router picks models based on query complexity. The user has no way to say "I need 95% quality for this task."

**Target**: Accept per-request accuracy targets (τ) and use Lagrangian optimization to minimize cost while maintaining the specified quality floor.

**Research basis**: PROTEUS, Section 3.2 of research-alternatives.md. 89.8% cost savings while maintaining specified quality thresholds.

**Scope**: Extend `UnifiedRequest` with an optional `quality_target: f64` field. Build a model quality database from logged inference outcomes. Router solves the constrained optimization: minimize cost subject to expected quality ≥ τ.

---

### 2.3 Tiered Inference Pipeline

**Current state**: All queries go through the same path — cache check, then a single LLM call.

**Target**: Three-layer response pipeline with automatic escalation:

1. Cache hit → ~5ms
2. Local model (e.g., qwen3:8b) with confidence check → ~200ms
3. Cloud model escalation (only if local model is uncertain) → ~2s

**Research basis**: Section 7.4 of research-alternatives.md. 70% of queries answered locally.

**Scope**: Add a confidence evaluator after local model responses (token probability, response length, self-reported uncertainty). If confidence < threshold, escalate to the next model in the fallback chain. Track escalation rates in `inference_costs` for tuning.

---

### 2.4 Speculative Execution

**Current state**: The agent loop is strictly sequential: send prompt → wait for response → evaluate tool calls → execute tools → next turn.

**Target**: While waiting for an LLM response, speculatively pre-fetch results for likely tool calls (read-only tools only). When the LLM requests a pre-fetched tool, return instantly.

**Research basis**: Section 7.3 of research-alternatives.md. Expected 30–50% latency reduction for predictable tool sequences.

**Scope**: After sending an inference request, analyze conversation context to predict likely tool calls. Spawn speculative `tokio::spawn` tasks for read-only tools (file read, HTTP GET, memory lookup). Maintain a speculation cache keyed by tool name + args. Discard on turn completion.

---

### 2.5 Service Revenue & Inbound Payments

**Current state**: The wallet handles outbound payments only (x402, yield). No mechanism to receive USDC for services rendered.

**Target**: The agent can advertise capabilities, quote prices, accept USDC payments, and deliver services — completing the self-sustaining economic loop.

**Research basis**: Section 6.3 of research-alternatives.md.

**Scope**: Define a service catalog (config-based). Implement payment verification (monitor USDC transfers to the agent's address). Create a `ServiceManager` that tracks requests, payments, and deliveries. Expose via A2A protocol and a new `/api/services` endpoint. Wire to the treasury for accounting.

---

### 2.6 Multi-Provider Cost Arbitrage

**Current state**: The router picks models by complexity. Provider pricing is not a routing factor.

**Target**: Real-time pricing awareness. Route to the cheapest provider that meets quality requirements for each query.

**Research basis**: Section 6.4 of research-alternatives.md.

**Scope**: Add `cost_per_million_tokens` to `ProviderConfig`. The router's `select_for_complexity()` considers both quality score and estimated cost. Log actual costs in `inference_costs` to refine estimates. Optionally poll provider pricing APIs for dynamic rates.

---

### 2.7 WASM Plugin Runtime ✅

**Status**: Implemented. `ironclad-agent/src/wasm.rs` provides a capability-gated WASM runtime with module load/compile validation, memory limits, JSON-oriented execution, and sandboxed plugin lifecycle management.

---

### 2.8 Prompt Compression

**Current state**: Context assembly uses progressive loading (L0–L3) and structural dedup. No token-level pruning.

**Target**: Remove low-importance tokens from prompts using perplexity-based scoring. 2–20x compression with <5% quality loss.

**Research basis**: LLMLingua / LongLLMLingua, Section 5.1 of research-alternatives.md.

**Scope**: Run a small local model (or use the primary local model) to score token importance via perplexity. Remove tokens below a threshold before sending to the inference model. Most impactful for long conversation histories and large tool descriptions. Gate behind config: `cache.prompt_compression = true`.

---

### 2.9 Declarative Agent Manifests ✅

**Status**: Implemented. `ironclad-agent/src/manifest.rs` includes `ManifestLoader` with schema validation and hot-reload/hash tracking for TOML-defined specialist manifests.

---

### 2.10 Structured Workspace System ✅

**Status**: Implemented. Workspace state APIs and runtime context plumbing are in place (`/api/workspace/state` plus workspace metadata integration), giving operators and the dashboard a structured workspace view.

---

### 2.11 Knowledge Source Trait ✅

**Status**: Implemented. `ironclad-agent/src/knowledge.rs` defines `KnowledgeSource` plus concrete sources (`DirectorySource`, `GitSource`, `VectorDbSource`, `GraphSource`) with registry integration; `ironclad-agent/src/obsidian.rs` also implements `KnowledgeSource` for vault-backed retrieval.

---

### 2.12 Episodic Digest System

**Current state**: Memory tiers persist raw data (turns, facts, procedures, relationships). When a session ends or is compacted, the raw history is archived but no coherent summary is produced. On the next session start, the agent has no concise record of where it left off — it must re-derive context from scattered memory fragments.

**Target**: Automated session digests that capture agent state, key decisions, unresolved tasks, and learned facts at session boundaries. Integrated into the memory retrieval system with decay-weighted relevance.

**Builds on**: `ironclad-agent/src/memory.rs` (turn classification), `ironclad-agent/src/context.rs` (compaction), `ironclad-agent/src/retrieval.rs` (hybrid retrieval), `episodic_memory` table.

**Scope**: At session boundaries (close, compaction, TTL expiry), the compaction pipeline generates an `EpisodicDigest` — a structured summary of agent state, key decisions, unresolved tasks, and learned facts. Stored in the `episodic_memory` table with a `digest` flag and elevated retrieval priority. On next session start, `MemoryRetriever` automatically surfaces the most recent digest as high-priority context — no manual intervention required. Digests are decay-weighted: the most recent has maximum relevance, older digests fade unless their content matches the current query via the hybrid search. The agent doesn't need to "remember to save state" — the system does it automatically at every session boundary.

---

### 2.13 Hippocampus — Self-Describing Schema Map

**Current state**: The database has 28 system tables, but the agent has no awareness of what data structures exist, how they're organized, or how to query them. The agent cannot create its own data structures to organize knowledge for domain-specific tasks.

**Target**: A living schema map (the "hippocampus") that gives the agent introspective awareness of its own data architecture and the ability to create, extend, and manage its own tables at runtime — self-modifying data architecture with guardrails.

**Builds on**: `ironclad-db/src/schema.rs` (table definitions), `ironclad-agent/src/tools.rs` (`Tool` trait), `ironclad-agent/src/policy.rs` (risk gating), `ironclad-schedule/src/tasks.rs` (`MetricSnapshot` for stats refresh).

**Scope**: Add a `hippocampus` table to the schema. Every table in the database is registered with:

- **`table_name`** — the qualified name
- **`owner`** — `system` for built-in tables, or an agent's unique identifier for agent-created tables
- **`schema_ddl`** — the `CREATE TABLE` statement, kept in sync automatically
- **`description`** — human/agent-readable explanation of the table's purpose
- **`query_patterns`** — documented query patterns (common SELECTs, JOINs, FTS5 usage) so the agent knows *how* to use the table
- **`relationships`** — foreign key and logical relationships to other tables
- **`access_level`** — `read_only` (system tables), `read_write` (agent-owned), `internal` (hidden from agent)
- **`row_count`** / **`last_modified`** — approximate stats refreshed by `MetricSnapshot` heartbeat task

Agent-created tables: Implement `CreateTable`, `AlterTable` (add columns only), and `DropTable` tools in the `ToolRegistry`. The `CreateTable` tool enforces naming — agent-created tables are prefixed with the agent's unique identifier (e.g., `ag_duncan_research_notes`, `ag_briefing_daily_summaries`). The tool validates the schema (no reserved column names, no system table collisions), executes the DDL, registers the table in the hippocampus with `owner = agent_id`, and optionally creates indexes from declared query patterns. `DropTable` only works on tables the agent owns. The policy engine gates table creation behind `RiskLevel::Caution` with limits: max tables per agent, max total size, no `PRAGMA` or `ATTACH` access.

On first boot, `initialize_db()` populates the hippocampus with all system tables. On subsequent boots, a consistency check verifies the hippocampus matches the actual schema. The `build_context()` function can inject a compact hippocampus summary (table names + one-line descriptions) into the system prompt, giving the agent ambient awareness of its storage capabilities.

---

### 2.14 Skills Catalog (CLI + Dashboard)

**Current state**: Skills are loaded from local directories and managed via direct file operations, API calls, and ad hoc toggles. There is no first-party catalog UX for browsing Ironclad-produced skills or batch-installing curated sets.

**Target**: A first-party skills catalog for both CLI and dashboard that lets users browse published Ironclad skills, multi-select items, and perform one-shot download, install, and activation workflows.

**Builds on**: `SkillLoader`/reload pipeline, existing skills API routes, plugin/skill toggle flows, dashboard SPA controls.

**Scope**: Add a signed catalog manifest endpoint (or bundled index) containing skill metadata (name, description, version, tags, compatibility, checksums). Implement CLI flows such as `ironclad skills catalog list`, `search`, and multi-select install/activate (`--select` / interactive prompt). Add dashboard catalog UI with filters, multi-select, and a single "Install + Activate" action. Enforce integrity verification before install (checksum/signature), idempotent installs, explicit upgrade prompts, and rollback on partial failure. Persist install status in the skills registry and surface activation state consistently across CLI and dashboard.

---

### 2.15 Ops Telemetry + Release Provenance Gate

**Current state**: Runtime visibility and model-level efficiency analytics are strong, but release readiness and platform-level observability remain partly workflow-driven.

**Target**: Ship-grade operational telemetry and release provenance by default.

**Builds on**: CI pipelines, smoke tests, existing logging/health APIs, release workflows.

**Scope**: Add mandatory release smoke gates, structured deployment readiness checks, artifact provenance/signing metadata, and operator-facing telemetry exports suitable for SLO monitoring. Include runbook-level verification criteria before tag/publish.

---

### 2.16 Matrix Channel Adapter (E2EE Day One)

**Current state**: Ironclad supports Telegram, WhatsApp, Discord, and WebSocket channels. Matrix is not yet available as a channel adapter.

**Target**: Full bidirectional Matrix integration for DMs and rooms with day-one end-to-end encryption support and secure event handling.

**Builds on**: `ChannelAdapter` trait, `ChannelRouter`, durable outbound delivery/retry pipeline, existing channel health and observability surfaces.

**Scope**: Implement `MatrixAdapter` in `ironclad-channels` with encrypted inbound/outbound text flows, room membership handling, and reliable sync token/cursor lifecycle management. Add device/session key lifecycle support (bootstrap, secure persistence, rotation, recovery-safe resume) and map Matrix room/thread context into Ironclad session routing semantics. Extend server runtime wiring, channel status exposure, and configuration with a new `[channels.matrix]` block. Align delivery failures with retry/dead-letter controls and add Matrix-specific permanent/transient error classification. This is a roadmap backlog expansion and is not part of the locked 0.8.0 execution set.

---

### 2.18 Compliance-First Self-Funding Portfolio + Profit Taxation

**Current state**: Ironclad has wallet/treasury controls, transaction tracking, and a service lifecycle foundation, but no first-class self-funding package optimized for legal, low-friction income generation. There is also no built-in mechanism to automatically redirect a configurable share of realized bot profit to the user's Ironclad-affiliated wallet.

**Target**: A realistic, low-overhead self-funding system that helps agents maintain strong runtime capacity while remaining compliance-first. Include an adjustable taxation paradigm that applies to net realized profit per completed paid job and automatically redirects the taxed amount to the user's affiliated wallet.

**Builds on**: `ServiceManager`, wallet/treasury (`ironclad-wallet`), `transactions` ledger, A2A/service endpoints, specialist routing and subagent capability matching.

**Scope**: Ship a curated self-funding catalog of legal, repeatable service archetypes (monitoring/alerts, recurring summaries, narrow specialist deliverables) with transparent pricing templates. Add profit accounting primitives: `net_realized_profit = earned_revenue - attributable_costs` (inference, fulfillment payouts, network/settlement fees) at job completion. Add config and controls for `self_funding.tax.enabled`, `self_funding.tax.rate` (0.0-1.0), `self_funding.tax.destination_wallet`, and per-service opt-in/out. On each completed paid job, compute tax from net realized profit, create an auditable settlement record, and transfer/allocate funds to the configured affiliated wallet. Enforce safety rails: max transfer caps, minimum reserve floor, invalid-address rejection, and explicit no-op behavior when profit <= 0. Expose dashboard/API observability for gross revenue, attributable costs, net profit, tax paid, after-tax retained earnings, and payout history.

---

### 2.19 Model Metascore Routing Profiles

**Current state**: Routing decisions are spread across heuristics (complexity, fallback order, capacity, and partial pricing) without a unified per-model profile that can be matched against task-level requirements.

**Target**: Maintain a first-class profile for every model and compute a task-specific metascore so routing/orchestration can deterministically prefer the best-fit model for each task.

**Builds on**: `ModelRouter`, provider registry/config, capacity/cost telemetry, fallback orchestration, and future ML routing backends.

**Scope**:

- Define a `ModelProfile` schema per model with weighted attributes:
  - known efficacy by task type (coding, summarization, extraction, planning, etc.)
  - cost (token and request economics)
  - availability/reliability (uptime, breaker state, recent error rate)
  - locality/privacy posture (local-only, region-bound, remote cloud)
  - resource requirements (VRAM/RAM/context-window/runtime constraints)
  - extensible fields for future criteria (`tbd`)
- Define a `TaskRequirements` schema that captures hard constraints (privacy/locality, budget cap, latency target, minimum quality target) and soft preferences.
- Compute `metascore(model, task)` from profile × requirements with transparent scoring/explanations and deterministic tie-breakers.
- Integrate metascore selection into both direct routing and multi-model orchestration paths, with fallback if top candidate is unavailable.
- Persist profile snapshots and outcomes so metascore weights can be tuned over time from observed quality/cost/latency data.
- Expose operator controls for policy weights and audit visibility into why one model was selected over another.
- Add dashboard explainability panels that show per-task match reasoning (criteria scores, constraint passes/fails, tie-break path).
- Add a simulation mode so operators can run "what would be selected?" scenarios against alternative task requirements and weight profiles before enabling active routing changes.

**Model efficacy assessment workstream (planning-only)**:

Reference template: `docs/MODEL_RUNNER_EVAL_TEMPLATE.md` (v1).
Scoring contract reference: `docs/evals/METASCORE_V1_SPEC.md` (spec-only).

- Define a canonical task taxonomy and eval sets per class: coding, summarization, extraction, planning, tool-use, classification, translation, and safety/refusal behavior.
- Establish scorecards per task class with explicit metrics (accuracy/pass@k, factuality/hallucination rate, schema adherence, latency-to-quality curve, and retry sensitivity).
- Run multi-pass evaluations per model/profile variant (temperature, context length, tool-on/tool-off) and store confidence bands instead of single-point scores.
- Maintain benchmark freshness windows (rolling monthly smoke evals + quarterly deep evals) so efficacy scores decay when stale.
- Add operator-defined "mission critical" slices (domain prompts + failure tests) that can override global efficacy priors for routing.
- Feed final efficacy outputs into `ModelProfile.known_efficacy_by_task_type` with provenance links to eval run IDs, dataset versions, and scoring policy revisions.

---

### 2.20 Voice Channels

**Promoted from Tier 3 (was 3.6).** Voice interaction is becoming a baseline expectation for agent runtimes; promoting to Tier 2 reflects user demand and the maturity of local STT/TTS tooling.

**Current state**: Text-only channels (Telegram, WhatsApp, Discord, WebSocket). No voice input or output capability.

**Target**: Voice input/output via Telegram voice messages, WhatsApp audio, and a WebRTC channel for the dashboard. TTS output as a standalone near-term deliverable before full WebRTC.

**Builds on**: `ChannelAdapter` trait, channel router, dashboard SPA, existing audio codec infrastructure in channel adapters (WhatsApp/Telegram already handle voice message payloads).

**Scope**: Speech-to-text via `whisper-rs` (native Rust, local-first) with cloud STT as opt-in fallback. Text-to-speech as a separately shippable milestone: support local TTS models (Piper, Coqui) as the default, consistent with the local-first philosophy, with cloud TTS (provider APIs) as opt-in. Config: `[channels.voice]` with `stt_model`, `tts_model`, `tts_voice`. Stream audio via WebRTC for real-time voice conversation on the dashboard. Store transcripts in session history alongside audio references.

**Phasing**: Ship TTS output first (lowest complexity, immediate UX impact). STT for inbound voice messages second. WebRTC dashboard voice third.

---

### 2.21 Skill Registry Protocol

**Current state**: Skills are loaded from local directories. `2.14 Skills Catalog` adds CLI/dashboard browse and install for first-party skills, but there is no protocol for community-contributed skill distribution. As agent runtimes mature, ecosystem breadth becomes a key differentiator — a registry protocol enables community participation without centralizing control.

**Target**: A fetchable skill registry protocol that enables community skill distribution without requiring Ironclad to host a centralized marketplace.

**Builds on**: `SkillLoader`/`SkillRegistry`, `2.14 Skills Catalog` (CLI + dashboard), skill manifest format, integrity verification.

**Scope**: Define a `SkillRegistryIndex` JSON schema: skill metadata (name, version, description, author, tags, compatibility range, checksum, source URL). Implement `ironclad skills search <query>` and `ironclad skills install <name>` CLI flows that resolve against one or more configured registry URLs. Registries can be static JSON files (GitHub repo, S3 bucket), Git repositories, or HTTP endpoints. Default registry ships as a curated GitHub repo of verified community skills. Add signature verification for registry entries (reuse existing wallet/ECDSA infrastructure). Support multiple registries with priority ordering and conflict resolution. Extend dashboard catalog UI (`2.14`) to surface community registry results alongside first-party skills. Config: `[skills.registries]` with URL list, trust levels, and auto-update policy.

**Non-goals for v1**: Hosted marketplace, payment for skills, skill review/curation workflow (manual curation via PR to the registry repo is sufficient initially).

---

## Tier 3 — Frontier

Ambitious capabilities that push the architecture into new territory. High effort, high potential.

### 3.1 Compile-Time Agent Safety (Typestates)

**Current state**: Agent lifecycle states (`Setup`, `Waking`, `Running`, `Sleeping`, `Dead`) are runtime enums. Policy evaluation is a runtime check.

**Target**: Use Rust's type system to make illegal state transitions compile errors. A `Tool<Unevaluated>` cannot be executed — only `Tool<Allowed>` can.

**Research basis**: Section 7.1 of research-alternatives.md.

**Scope**: Refactor `AgentLoop` to use typestate pattern. Introduce phantom type parameters on `ToolCallRequest` that carry policy evaluation results. Financial limits as const generics on treasury types. This is a deep refactor with compounding safety benefits.

---

### 3.2 MCP (Model Context Protocol) Integration ✅

**Status**: Implemented. `ironclad-agent/src/mcp.rs` provides MCP server/client primitives; server runtime exposes MCP endpoints under `/api/runtime/mcp` with client discovery/disconnect controls and tool/resource export wiring.

---

### 3.3 Multi-Agent Orchestration (Partially Complete)

**Current state**: Partially implemented in 0.7.0. Subagent role contracts (`subagent` vs `model-proxy`), roster semantics, model-assignment modes (`auto`/`commander`), and turn-linked model-selection forensics are live. Full workflow orchestration patterns are still pending.

**Target**: Internal multi-agent patterns — specialist sub-agents for code review, research, financial analysis — orchestrated by a coordinator agent using graph-based workflows.

**Prerequisite**: 2.9 (Declarative Agent Manifests). Orchestration patterns operate on manifest-declared capabilities rather than hardcoded specialist names — the coordinator matches subtask requirements to specialist capability declarations, so adding a new specialist is a config file, not a code change.

**Scope**: Extend `SubagentRegistry` to manage actual agent instances (each with its own session, tools, and optionally its own wallet). Define orchestration patterns: sequential, parallel fan-out/fan-in, and handoff. Coordinator agent routes subtasks to specialists based on capability matching against manifest declarations. Specialist resolution is dynamic — the coordinator queries available manifests for capability overlap with the subtask requirements.

---

### 3.4 Agent Spawning with Wallet Provisioning

**Current state**: `SubagentRegistry` handles lifecycle metadata. No wallet provisioning, no autonomous child agent execution.

**Target**: An agent can spawn a child agent, provision it with a fraction of its treasury, delegate a task, and reclaim funds on completion or timeout.

**Scope**: Generate a child wallet (derived from parent wallet or fresh keypair). Transfer USDC to child via the parent's treasury. Child inherits a restricted config (reduced caps, limited tool access, time-bounded). Parent monitors child's progress and reclaims remaining funds on completion.

---

### 3.5 Advanced RAG Infrastructure (Partially Complete ✅)

**Status**: Items 1–4 implemented. Item 5 (document ingestion pipeline) remains.

**Completed**:

1. ✅ **Binary embedding storage** — `embedding_blob BLOB` column in `embeddings` table, with `embedding_to_blob()` / `blob_to_embedding()` conversion. JSON fallback preserved for backward compatibility. ~4x storage reduction.
2. ✅ **HNSW ANN index** — `ironclad-db/ann.rs` wraps `instant-distance` (pure Rust). Built from DB at startup when `memory.ann_index = true`. Falls back to brute-force cosine scan when disabled or corpus is small.
3. ✅ **Content chunking** — `chunk_text()` in `ironclad-agent/retrieval.rs`. Overlapping chunks (512 tokens, 64-token overlap) for granular embedding and retrieval.
4. ✅ **Persistent semantic cache** — `ironclad-db/cache.rs` with `save_cache_entry()` / `load_cache_entries()` / `evict_expired_cache()`. Server loads cache from SQLite on boot, flushes every 5 minutes via background task.

**Remaining**:
5. **Document ingestion pipeline** — Ingest external documents (PDF, markdown, code files) into the memory system. Parse, chunk, embed, and store for retrieval. Extends the agent's knowledge base beyond conversation history.

**Release target**: Document ingestion (item 5) is targeted for `v0.10.0`. With items 1–4 complete, the retrieval infrastructure is production-ready — ingestion is the natural next step to let operators feed domain knowledge directly into the memory system rather than relying solely on conversation history.

---

### 3.6 Voice Channels

**Moved to Tier 2 as 2.20.** See `2.20 Voice Channels` for current spec.

---

### 3.7 UniRoute Model Vectors

**Current state**: Model routing uses a heuristic or (future) trained classifier that must be retrained for new models.

**Target**: Represent each model as a feature vector derived from its capabilities, pricing, and benchmark performance. Route among unseen models without retraining.

**Research basis**: UniRoute, Section 3.3 of research-alternatives.md.

**Scope**: Define a model feature schema (context window, pricing tiers, benchmark scores, supported modalities). Build feature vectors from the provider registry. Train a meta-router that selects models by vector similarity to the query's requirements. Automatically adapts when new models are added to the registry.

---

### 3.8 Game-Theoretic Cascade Optimization

**Current state**: Fallback chain is configured statically (`models.fallbacks`). No analysis of when cascading helps vs. hurts.

**Target**: Automatically determine the optimal cascade strategy per query type — sometimes skipping the weak model entirely is cheaper than trying it first.

**Research basis**: Section 3.4 of research-alternatives.md.

**Scope**: Log cascade outcomes (did the weak model succeed? what was the latency cost of trying?). Compute expected utility for cascade vs. direct routing per query class. Dynamically switch strategy based on accumulated data.

---

### 3.9 Storage Backend Trait

**Current state**: `ironclad-db` is SQLite-only via `rusqlite` (bundled). SQLite is the right default — zero-ops, embedded, single-file backup, WAL mode for concurrent reads. But multi-agent clusters and distributed deployments may need multi-writer access that SQLite's single-writer model cannot provide.

**Target**: Abstract the database layer behind a trait so SQLite remains the default while alternative backends (PostgreSQL) are available as an opt-in escape hatch for deployments that genuinely need concurrent writers.

**Builds on**: `ironclad-db/src/lib.rs` (`Database` struct), `schema.rs`, all DB consumer modules.

**Scope**: Define a `StorageBackend` trait abstracting connection management, query execution, and transaction semantics. `SqliteBackend` wraps the current `Arc<Mutex<Connection>>`. `PostgresBackend` (via `sqlx`) is an optional feature flag (`--features postgres`). Schema definitions are shared; backend-specific SQL is isolated behind the trait. For replication without leaving SQLite, prefer Litestream (WAL streaming to S3) — it preserves the zero-ops philosophy while adding durability. PostgreSQL is an escape hatch, not the recommended path. Config: `[database]` section with `backend = "sqlite"` (default) or `backend = "postgres"`.

---

### 3.10 Cryptographic Device Identity

**Current state**: Ironclad runs as a single-device process. No device identity concept exists — there is no way to pair multiple devices, sync session state across machines, or verify that a connecting client is a trusted device.

**Target**: Zero-trust device identity and pairing built on existing cryptographic infrastructure. Each device gets a keypair; pairing uses mutual authentication; synced state is encrypted in transit.

**Builds on**: `ironclad-wallet/src/wallet.rs` (ECDSA keypair generation, AES-256-GCM encryption), `ironclad-channels/src/a2a.rs` (ECDSA/ECDH mutual auth, session encryption).

**Scope**: Derive device identity from the existing wallet infrastructure — each device gets an ECDSA keypair (same `k256` stack used by the wallet). Generate on first boot, persist encrypted alongside the wallet key. Device pairing reuses the A2A protocol's mutual authentication flow: ECDSA challenge-response for identity verification, ECDH for session key derivation, AES-256-GCM for encrypted state sync. A paired device can sync session state over an encrypted channel. This adds zero new cryptographic dependencies — it composes existing primitives. Config: `[devices]` section with pairing mode and sync policy.

---

### 3.11 Agent Discovery Protocol

**Current state**: The A2A protocol requires knowing the peer agent's URL upfront. The `/.well-known/agent.json` agent card is served and refreshed by the `AgentCardRefresh` heartbeat task, but there is no mechanism for agents to find each other without a manually-configured URL.

**Target**: DNS-based agent discovery that makes agent cards findable across networks, with mDNS fallback for zero-config LAN scenarios. Discovered agents are verified via existing mutual authentication before being trusted.

**Builds on**: `/.well-known/agent.json` endpoint, `AgentCardRefresh` heartbeat task, A2A handshake (ECDSA mutual auth).

**Scope**: Agents publish DNS `SRV` records (`_ironclad._tcp`) and `TXT` records with capability hashes. Discovery clients resolve via DNS-SD — works across networks, through NATs, and is firewall-friendly. For LAN scenarios, fall back to mDNS (via `mdns-sd` crate) as a zero-config option. Discovered agents are verified via the existing ECDSA mutual authentication before being added to the `discovered_agents` table. The agent card already exists and is refreshed by heartbeat — discovery just makes it findable. Config: `[discovery]` section with `dns_sd`, `mdns`, and `advertise` settings.

---

### 3.12 Flexible Network Binding ✅

**Status**: Implemented in 0.5.0. Operators can bind the server to localhost, LAN, overlay network interfaces, or `0.0.0.0` via server bind configuration (`[server].bind`) with authentication safeguards for non-loopback exposure.

---

### 3.13 Zero-Trust Global Remote UI Access

**Current state**: The dashboard can be exposed on non-loopback interfaces (`3.12`) and API auth exists, but remote access is still mostly perimeter-based. There is no dedicated zero-trust access layer for secure global UI access with strong identity, device trust, and session-hardening defaults.

**Target**: Secure global remote access to the Ironclad UI by default. Treat every network as untrusted and require cryptographic identity, strong authentication, hardened sessions, and explicit authorization for all UI/API traffic.

**Builds on**: `3.12 Flexible Network Binding`, API auth/session routes, TLS support, existing policy and audit infrastructure.

**Scope**: Add a dedicated remote access security layer with defense-in-depth: mandatory TLS (with optional mTLS for operator/admin roles), OIDC/SAML SSO + enforced MFA + short-lived tokens, device trust (key-bound sessions and optional passkey/WebAuthn), per-route RBAC for dashboard/API actions, IP reputation + geo-anomaly detection with adaptive challenge/deny, rate-limit and WAF hooks for auth surfaces, strict CSRF/CORS/cookie hardening, signed session rotation and revocation, comprehensive audit trails for auth/admin actions, and a "remote-lockdown" mode that defaults to deny-by-default except explicit allowlists. Ship with threat-model documentation, security runbooks, and hardened production presets.

---

## Robot Intel Feed (Planning-Only)

Competitive learnings captured as **robot subroutines** for roadmap shaping. These are planning artifacts only and do not override the current bug-hammer release policy.

### Subroutine A — Explainability Cockpit + Simulation Bay

**Signal**: operator trust increases when routing decisions are inspectable and testable before activation.

**Plan fit**: fold directly into `2.19 Model metascore routing profiles`.

**Scope addendum**:

- Dashboard panel showing why a model matched a task (criteria contributions + constraint pass/fail + tie-break trace).
- Simulation bay for "what-if" scenarios (task requirements and weight profile changes) with side-by-side current vs simulated pick.
- Audit trail for simulation runs and policy changes.

### Subroutine B — Tamper-Evident Runtime Ledger

**Signal**: security posture is stronger when action history is cryptographically verifiable.

**Plan fit**: candidate Tier 3 security-hardening item, builds on existing audit/logging infrastructure.

**Scope draft**:

- Hash-linked event chain across approvals, tool calls, outbound channel sends, and admin policy mutations.
- Verifier command/API for integrity checks and incident forensics.
- Retention and export rules for operator compliance workflows.

### Subroutine C — Capability Kernel Hardening

**Signal**: agent safety improves when capabilities are declared and metered as first-class runtime constraints.

**Plan fit**: extends existing policy engine and sandbox direction without feature-sprawl.

**Scope draft**:

- Capability declarations bound to tools/subroutines.
- Runtime meter visibility (fuel/time/budget) in operator telemetry.
- Deny-by-default profile for sensitive subroutine classes.

### Subroutine D — Migration Docking Protocol

**Signal**: adoption accelerates when migration from adjacent agent runtimes is reversible and observable.

**Plan fit**: medium-priority Tier 2/3 operability track.

**Scope draft**:

- Dry-run import manifests with explicit transform maps.
- Reversible migration checkpoints.
- Compatibility scorecard for skills/sessions/config artifacts.

### Subroutine I — Channel Pairing Handshake Bus

**Signal**: bot-channel trust improves when unknown users are onboarded via explicit one-time pairing handshakes instead of implicit allowlist drift.

**Plan fit**: extends abuse-protection and remote-access hardening with operator-friendly identity onboarding.

**Scope draft**:

- One-time pairing code flow for DM onboarding across supported channels.
- Approve/revoke/list controls in CLI + API with audit events.
- Expiry, replay protection, and rate-limiting for pairing attempts.

### Subroutine J — Terminal Substrate Matrix

**Signal**: autonomy is safer when command execution can be routed to isolated substrates (local/container/remote) under explicit policy.

**Plan fit**: aligns with capability kernel hardening and mission runtime safety controls.

**Scope draft**:

- Terminal backend abstraction contract with policy-driven substrate selection.
- Per-subroutine execution sandbox requirements (local vs isolated vs remote).
- Persistent workspace semantics + lifecycle controls per substrate class.

### Subroutine K — Interrupt-and-Redirect Contract

**Signal**: long-running autonomous flows need deterministic interrupt semantics so operators can safely redirect mission state mid-flight.

**Plan fit**: direct dependency for autonomous mission runtime and recursive escalation safety.

**Scope draft**:

- Unified interrupt protocol for CLI, API, and messaging channels.
- Deterministic cancellation propagation through subroutine trees.
- Interrupt checkpoints that preserve resumable mission context.

### Subroutine L — Connector Reliability Profiles

**Signal**: integration breadth only matters when connector reliability, degradation behavior, and retry semantics are explicit and observable.

**Plan fit**: strengthens orchestration control tower and continuous mission runtime.

**Scope draft**:

- Reliability profile per connector (health state, retry budget, backoff class, delivery guarantees).
- Operator SLO surfaces and alert thresholds by connector.
- Mission planner awareness of connector risk when selecting execution paths.

### Subroutine M — Procedural Memory Forge

**Signal**: agents become materially more capable when successful multi-step trajectories are promoted into reusable subroutines with governance.

**Plan fit**: bridges memory systems, skills catalog, and mission orchestration.

**Scope draft**:

- Promote validated task trajectories into candidate skills/subroutines.
- Review gates (auto-suggest, operator-approve, publish-local) with provenance history.
- Retrieval/ranking logic that prefers proven procedural assets for matching mission patterns.

### Subroutine N — Memory Conveyor Inference Dock

**Signal**: model reach expands dramatically when low-VRAM operators can run frontier-class checkpoints through layer-streamed inference docks.

**Plan fit**: extends `2.19 Model metascore routing profiles` and local-runtime install/setup tracks without replacing high-throughput engines.

**Scope draft**:

- Add AirLLM-class host profile as a low-VRAM execution substrate with explicit "latency-heavy, locality-strong" capability flags.
- Detect Python runtime + `airllm` availability in setup/install flows and offer optional bootstrap only when no better local host is already configured.
- Record runtime traits for routing (cold-start overhead, token latency class, disk I/O sensitivity, supported model families, compression mode).
- Feed traits into metascore orchestration so model selection can favor this dock for privacy/locality-constrained tasks while penalizing latency-sensitive paths.
- Add dashboard explainability reasons and simulation toggles ("why selected AirLLM dock" + side-by-side route outcomes against SGLang/vLLM).
- Define safety rails: max-context guards, disk-space preflight checks, and operator warnings for interactive session expectations.

### Subroutine O — Agentic Loop Stability Pilot (`Qwen3.5-35B-A3B`)

**Signal**: local agents become far more usable when tool-calling and loop stability are reliable at low active-parameter budgets.

**Plan fit**: direct accelerator for `2.19 Model metascore routing profiles` by adding a strong local candidate for `tool_use` and multi-step orchestration tasks.

**Scope draft**:

- Add `qwen3.5-35b-a3b` as a priority benchmark candidate across local runners (SGLang-first, with vLLM/Ollama where compatible).
- Expand task-efficacy slices for agent loops: tool schema adherence, retry/recovery behavior, loop completion rate, and long-horizon drift rate.
- Capture and surface loop-stability deltas in explainability panels ("selected due to higher loop stability/tool success on this task class").
- Add simulation scenarios that compare this profile against existing local/cloud choices under privacy-first and latency-first policy presets.
- Define readiness gates for promotion into default local-routing recommendations (minimum tool success + loop completion thresholds).

### Mission-Kernel Freeze Guard

For the current major-version bug-hammer cycle:

- Discovery, specs, and roadmap shaping for these subroutines are allowed.
- Implementation remains deferred until the freeze is lifted.

---

## Autonomous Mission Kernel (Planning-Only)

Outcome-first autonomy as a coordinated **robot mission runtime**. This is a roadmap integration target, not an active implementation track during the bug-hammer major version.

### Subroutine E — Outcome Compiler + Mission Graph

**Current state**: users still compose many multi-step workflows manually across skills, scheduling, and model choices.

**Target**: accept an operator-defined outcome and compile it into a mission graph of tasks/subtasks with explicit dependencies, constraints, and success criteria.

**Builds on**: `2.19 Model metascore routing profiles`, scheduler, skills catalog, channel delivery durability.

**Scope draft**:

- Parse outcome intents into typed `TaskRequirements` + hard constraints.
- Produce a deterministic mission DAG with retry/fallback semantics.
- Persist mission state for restart-safe execution and auditability.

### Subroutine F — Recursive Specialist Escalation Kernel

**Current state**: subagent orchestration exists but recursive helper delegation is not a first-class policy-governed kernel primitive.

**Target**: allow a subroutine to spawn helper subroutines when blocked, with strict policy rails.

**Builds on**: `3.3 Multi-agent orchestration (partial)`, policy engine, approvals, rate/budget controls.

**Scope draft**:

- Escalation policy contract (depth, breadth, wall-clock, and budget caps).
- Approval gates for sensitive escalation classes.
- Deterministic unwind behavior when escalation trees fail.

### Subroutine G — Continuous Mission Runtime

**Current state**: scheduling exists, but long-horizon mission supervision and intervention semantics are fragmented.

**Target**: run autonomous mission loops for days/weeks with durable checkpoints, health states, and explicit intervention controls.

**Builds on**: scheduler heartbeat, session rotation contracts, durable queues, telemetry/release gates.

**Scope draft**:

- Mission lifecycle states (`queued`, `running`, `degraded`, `blocked`, `complete`).
- Checkpointed execution with restart-safe continuation.
- Operator controls for pause/resume/replan/terminate.

### Subroutine H — Orchestration Control Tower

**Current state**: observability is distributed across endpoints and dashboards; orchestration reasoning is not yet unified in one operator cockpit.

**Target**: a single control-tower surface that explains mission execution, model matches, and intervention options in real time.

**Builds on**: `2.19` explainability/simulation, dashboard SPA, admin API surfaces, context observatory.

**Scope draft**:

- Mission graph visualization with live status overlays.
- Per-subtask "why this model" reasoning and constraint traces.
- Simulation bay for what-if missions and policy-weight changes.
- Burn-rate telemetry (cost, latency, error, queue depth) with alert thresholds.

### Integration Mapping (No Scope Bloat)

- `2.19` remains the model-selection substrate (metascore + explainability + simulation).
- `3.3` remains the orchestration substrate (specialist delegation patterns).
- Scheduler/queues remain execution substrate (durability + long-horizon runs).
- New subroutines define composition contracts between those existing layers; they do not imply immediate net-new feature execution.

### Freeze Guard

For the current major-version bug-hammer cycle:

- Planning/specification for Subroutines E–H is allowed.
- Feature execution remains deferred until freeze exit criteria are met.

---

## Next Significant Release Selection (Kickoff)

Targeting a **0.8.0 candidate slate** with a bias toward reliability, operability, and user-facing distribution.

### Selection Criteria

- Must close a visible user workflow gap or high-impact reliability risk.
- Must be testable with clear acceptance criteria in one release cycle.
- Must avoid deep architectural rewrites that would starve release throughput.

### Proposed 0.8.0 Candidate Set

1. **2.14 Skills Catalog (CLI + Dashboard)** — marquee UX feature (browse, multi-select, install, activate).
2. **1.16 Durable Channel Delivery Queue** — message reliability and restart safety.
3. **1.17 Production-Grade Abuse Protection** — hardening for internet-facing deployments.
4. **1.18 Cron-Conformant Session Rotation** — eliminate lifecycle surprise/ambiguity.

### Stretch Candidate (if capacity permits)

- **2.15 Ops Telemetry + Release Provenance Gate** (land at least smoke gating + provenance baseline).

### Self-Funding Strategy (Visibility Track)

- **Strategic item**: `2.18 Compliance-First Self-Funding Portfolio + Profit Taxation`.
- **Positioning**: Keep out of the locked `v0.8.0` reliability core, but run discovery/design in parallel so implementation can start in `v0.9.x` without blocking current release throughput.
- **Execution intent**: Prioritize legal, low-overhead recurring service models first (monitoring, digests, narrow specialist deliverables), then layer adjustable per-job net-profit taxation redirected to the user's affiliated wallet.
- **Readiness gates before implementation**: finalize taxable profit accounting contract (`net_realized_profit`), destination wallet validation rules, reserve-floor + transfer-cap safety policy, and payout audit surfaces.

---

### v0.8.0 Execution Order (Locked)

Implementation sequence for this release:

1. **1.16 Durable Channel Delivery Queue**
2. **1.18 Cron-Conformant Session Rotation**
3. **1.17 Production-Grade Abuse Protection**
4. **2.14 Skills Catalog (CLI + Dashboard)**
5. **2.15 Ops Telemetry + Release Provenance Gate** *(stretch only)*

### v0.8.0 Acceptance Gates (Release Blocking)

- **1.16 Durable Queue**
  - Survives process restart with replayed pending deliveries.
  - Uses idempotent delivery keys to avoid duplicate sends after recovery.
  - Exposes dead-letter inspection + replay controls (API/CLI).
- **1.18 Cron-Conformant Rotation**
  - Evaluates resets directly from configured cron expressions.
  - Passes edge-schedule and timezone conformance tests.
  - Includes migration notes if semantics differ from legacy timing behavior.
- **1.17 Abuse Protection**
  - Validates trusted-proxy CIDR handling and real client IP extraction.
  - Enforces IP + token/user quotas with operator-visible metrics.
  - Verifies horizontally safe limiter behavior for multi-instance topology.
- **2.14 Skills Catalog**
  - CLI supports list/search/install/activate workflows.
  - Dashboard supports browse/filter/multi-select + install-and-activate.
  - Enforces integrity checks and rollback on partial failure.
- **2.15 Stretch**
  - Adds mandatory smoke checks for release/tag pipeline.
  - Emits verifiable provenance/signing metadata.
- **Legacy loopback proxy removal**
  - `127.0.0.1:8788/<provider>` legacy provider URLs are fully unsupported in runtime and examples for `v0.8.0`.
  - Startup/config validation fails fast with explicit operator migration guidance to direct provider base URLs.

### v0.8.0 Cut Policy

- First cut: **2.15** stretch item.
- If additional cut is required: reduce **2.14** to **CLI-first catalog** and defer dashboard catalog UI to 0.8.x.
- Do **not** cut reliability core (`1.16`, `1.17`, `1.18`) in this release profile.

### v0.8.0 Documentation Gate

Before marking 0.8.0 as complete:

- Update user/operator command docs in `docs/CLI.md`.
- Update deployment and hardening guidance in `docs/DEPLOYMENT.md`.
- Update architecture/dataflow docs in `docs/architecture/ironclad-dataflow.md`.
- Update release status and shipped scope in this roadmap file.
- Ensure release docs/checklists explicitly gate legacy loopback proxy removal (`docs/releases/v0.8.0.md`).

---

## Summary

Effort sizing legend: `S = 1-2 days`, `M = 3-5 days`, `L = 1-2 weeks`.

| # | Item | Tier | Builds On | Effort |
| --- | ------ | ------ | ----------- | -------- |
| 1.1 | ~~Streaming LLM responses~~ ✅ | 1 | LLM client, WebSocket, channels | ~~Medium~~ Done |
| 1.2 | ~~Approval workflow API~~ ✅ | 1 | ApprovalManager (complete) | ~~Low~~ Done |
| 1.3 | ~~Browser as agent tool~~ ✅ | 1 | ironclad-browser (complete) | ~~Low~~ Done |
| 1.4 | Discord WebSocket gateway | 1 | Discord adapter (mostly done) | Low |
| 1.5 | ~~Embedding provider integration~~ ✅ | 1 | embeddings.rs, ProviderConfig | ~~Medium~~ Done |
| 1.6 | Multimodal messages + voice transcription | 1 | WhatsApp media parsing, format.rs, whisper-rs | Medium |
| 1.7 | ~~Memory-augmented agent pipeline~~ ✅ | 1 | MemoryBudgetManager, build_context, hybrid_search, 1.5 | ~~High~~ Done |
| 1.8 | Email channel adapter | 1 | ChannelAdapter trait, OAuthManager | Medium |
| 1.9 | ~~Session scoping and lifecycle~~ ✅ | 1 | sessions.rs, HeartbeatTask, compaction | ~~Medium~~ Done |
| 1.10 | ~~Addressability filter~~ ✅ | 1 | ChannelAdapter trait, InboundMessage | ~~Low~~ Done |
| 1.11 | Context checkpoint | 1 | schema.rs, build_context, MemoryRetriever | Medium |
| 1.12 | ~~Response transform pipeline~~ ✅ | 1 | format.rs, injection defense, turns | ~~Low-Medium~~ Done |
| 1.13 | ~~Context Observatory~~ ✅ | 1 | efficiency.rs, turn_feedback, turns, inference_costs | Done |
| 1.14 | ~~Capacity-aware routing~~ ✅ | 1 | ModelRouter, CircuitBreakerRegistry | ~~Medium~~ Done |
| 1.15 | ~~Sessions markdown rendering~~ ✅ | 1 | dashboard SPA rendering, session/context views | ~~Low-Medium~~ Done |
| 1.16 | Durable channel delivery queue | 1 | channels delivery router, DB persistence | Medium |
| 1.17 | Production-grade abuse protection | 1 | rate limiter, auth, deployment config | Medium |
| 1.18 | Cron-conformant session rotation | 1 | SessionGovernor, scheduler heartbeat | Medium |
| 1.19 | `agent-browser` external runtime support | 1 | 1.3 Browser tool, policy engine, runtime config | Medium |
| 1.20 | Homebrew & Winget package manager distribution | 1 | release.yml, GitHub Releases, SHA256 pipeline | Medium |
| 1.21 | Integrations management (CLI + dashboard) | 1 | Channel adapters, keystore, channel health, dashboard SPA | Medium |
| 1.22 | Built-in introspection skill | 1 | Tool trait, ToolRegistry, ToolContext, SessionScope, ChannelRouter | Low |
| 1.23 | Context budget tuning | 1 | context.rs, token_budget, build_context, complexity scorer | Low |
| 2.1 | ML-based model routing | 2 | Heuristic router, RouterBackend trait | High |
| 2.2 | Accuracy-target routing | 2 | Router infrastructure | High |
| 2.3 | Tiered inference pipeline | 2 | Fallback chain, local model config | Medium |
| 2.4 | Speculative execution | 2 | Tool registry, tokio tasks | Medium |
| 2.5 | Service revenue & inbound payments | 2 | Wallet, treasury, A2A | High |
| 2.6 | Multi-provider cost arbitrage | 2 | ProviderConfig, router | Medium |
| 2.7 | ~~WASM plugin runtime~~ ✅ | 2 | Plugin SDK, ToolDef | ~~High~~ Done |
| 2.8 | Prompt compression | 2 | Context assembly, tier.rs | Medium |
| 2.9 | ~~Declarative agent manifests~~ ✅ | 2 | Config, SkillLoader, SubagentRegistry | ~~High~~ Done |
| 2.10 | ~~Structured workspace system~~ ✅ | 2 | personality.rs, soul_history | ~~Medium~~ Done |
| 2.11 | ~~Knowledge source trait~~ ✅ | 2 | retrieval.rs, EmbeddingClient, HNSW | ~~High~~ Done |
| 2.12 | Episodic digest system | 2 | memory.rs, compaction, retrieval | Medium |
| 2.13 | Hippocampus — self-describing schema map | 2 | schema.rs, Tool trait, policy engine | High |
| 2.14 | Skills catalog (CLI + dashboard) | 2 | SkillLoader, skills API, dashboard SPA | Medium |
| 2.15 | Ops telemetry + release provenance gate | 2 | CI workflows, smoke tests, release pipeline | Medium |
| 2.16 | Matrix channel adapter (E2EE day one) | 2 | ChannelAdapter, ChannelRouter, durable delivery queue | High |
| 2.17 | Apertus native local onboarding (SGLang-first) | 2 | install scripts, setup wizard, local host detection, model discovery | Medium |
| 2.18 | Compliance-first self-funding portfolio + profit taxation | 2 | ServiceManager, wallet/treasury, transactions ledger, specialist routing | High |
| 2.19 | Model metascore routing profiles | 2 | ModelRouter, provider registry/config, cost/capacity telemetry, orchestration | High |
| 2.20 | Voice channels (promoted from 3.6) | 2 | Channel adapters, whisper-rs, local TTS | High |
| 2.21 | Skill registry protocol | 2 | SkillLoader, 2.14 skills catalog, skill manifests, wallet/ECDSA | Medium |
| 3.1 | Compile-time agent safety | 3 | Agent loop, policy engine | High |
| 3.2 | ~~MCP integration~~ ✅ | 3 | Tool registry, config | ~~High~~ Done |
| 3.3 | Multi-agent orchestration (partial) | 3 | SubagentRegistry, A2A, 2.9 | High |
| 3.4 | Agent spawning + wallet provisioning | 3 | SubagentRegistry, wallet | High |
| 3.5 | Advanced RAG infrastructure (4/5 done ✅) | 3 | 1.5, 1.7, embeddings.rs | ~~High~~ Remaining: doc ingestion |
| 3.6 | ~~Voice channels~~ → promoted to 2.20 | ~~3~~ 2 | Channel adapters, whisper-rs, local TTS | High |
| 3.7 | UniRoute model vectors | 3 | Provider registry, router | High |
| 3.8 | Game-theoretic cascades | 3 | Fallback chain, inference_costs logs | Medium |
| 3.9 | Storage backend trait | 3 | ironclad-db, schema.rs | High |
| 3.10 | Cryptographic device identity | 3 | Wallet keypairs, A2A mutual auth | High |
| 3.11 | Agent discovery protocol | 3 | Agent card, A2A handshake, DNS-SD | Medium |
| 3.12 | ~~Flexible network binding~~ ✅ | 3 | Server bind, auth.rs | ~~Low~~ Done |
| 3.13 | Zero-trust global remote UI access | 3 | 3.12, auth/session stack, policy/audit infra | High |
