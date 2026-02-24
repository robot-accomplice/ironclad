# Ironclad Roadmap

*Growth areas organized by effort and impact. Each item notes whether it builds on existing code or is greenfield.*

---

## Tier 1 — Wire the Last Mile

Capabilities where the core code exists but isn't fully connected. High impact, low-to-medium effort.

### 1.1 Streaming LLM Responses ✅

**Status**: Implemented in 0.5.0. `POST /api/agent/message/stream` returns an SSE stream of tokens as they arrive from the provider. `forward_request()` supports streaming mode via `reqwest` byte streams. Partial responses are not cached; caching occurs only on stream completion. WebSocket subscribers receive real-time token events.

---

### 1.2 Approval Workflow API ✅

**Status**: Implemented in 0.5.0. Routes: `GET /api/approvals`, `POST /api/approvals/:id/approve`, `POST /api/approvals/:id/deny`. Pending approvals are pushed via WebSocket. The agent loop pauses on gated tool calls and resumes on approval or denial. `approval_requests` table tracks full lifecycle with timeout enforcement.

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

### 1.9 Session Scoping and Lifecycle

**Current state**: Sessions are keyed solely by `agent_id` — one session per agent via `find_or_create()`. No peer isolation (all Telegram users share the same session), no group-vs-DM distinction, no auto-expiry or scheduled rotation. Stale sessions accumulate indefinitely.

**Target**: Per-peer and per-group session isolation with configurable lifecycle policies. Cross-channel identity linking via shared peer IDs.

**Builds on**: `sessions.rs`, `schema.rs`, `HeartbeatTask` enum, `context.rs` compaction pipeline.

**Scope**: Introduce a type-safe `SessionScope` enum (`Agent`, `Peer { peer_id, channel }`, `Group { group_id, channel }`) as a composite key in the `sessions` table via migration `004_session_scoping.sql`. Update `find_or_create()` to scope by peer — the same Telegram user and WhatsApp user get separate sessions, but the memory system links them via the shared `peer_id` for cross-channel recall. Add a `SessionGovernor` heartbeat task: configurable TTL per scope type, compaction-on-archive (runs `build_compaction_prompt()` before marking inactive rather than discarding context), and scheduled rotation. Config: `[session]` section with `ttl`, `reset_schedule`, and `scope_mode`.

---

### 1.10 Addressability Filter ✅

**Status**: Implemented in 0.5.0. `AddressabilityFilter` trait with three composable implementations: `MentionFilter` (configurable name/alias patterns), `ReplyFilter` (responds when directly replied to), and `ConversationFilter` (responds in active threads). Composed via `FilterChain` — any match triggers dispatch. DMs bypass filtering. Config: `[agent.addressability]` with `mention_names`, `respond_to_replies`, `track_threads`.

---

### 1.11 Context Checkpoint

**Current state**: The agent rebuilds its full context from the database on every boot — querying memory tiers, retrieving embeddings, assembling the system prompt. This cold-start path adds latency before the agent is responsive.

**Target**: Instant agent readiness on boot via a transactional checkpoint that captures the agent's compiled context state.

**Builds on**: `schema.rs`, `build_context()`, `MemoryRetriever`, boot sequence in `lib.rs`.

**Scope**: Add a `checkpoints` table to the schema. Define a `ContextCheckpoint` struct (with `serde`) containing the compiled system prompt, top-k memory summaries, active task list, and recent conversation digest. Write checkpoints every N turns (configurable) and on graceful shutdown — crash recovery loses at most N turns of checkpoint state while raw data remains in the DB. On boot, `load_checkpoint()` provides instant agent readiness while background retrieval warms the full context asynchronously. Checkpoints are versioned for forward compatibility. Config: `[context.checkpoint]` with `interval_turns` and `enabled`.

---

### 1.12 Response Transform Pipeline ✅

**Status**: Implemented in 0.5.0. `ResponsePipeline` with `ResponseTransform` trait. Ships three transforms: `ReasoningExtractor` (extracts `<think>`/`<reasoning>` blocks and logs them to `turns.reasoning_trace`), `FormatNormalizer` (consistent response structure across providers), and `ContentGuard` (reflected injection defense via existing output scanning). Transforms are ordered and configurable via `[models.response_pipeline]`.

---

### 1.14 Context Observatory ✅

**Status**: Implemented in 0.5.0. The Context Observatory provides runtime visibility into context assembly efficiency, inference cost attribution, and output quality.

**Components**:
- `ironclad-db/efficiency.rs` — per-model efficiency analytics: output density, budget utilization, memory ROI, system prompt weight, cache hit rate, context pressure rate, cost attribution (system prompt / memories / history), wasted budget tracking
- `turn_feedback` table — stores outcome grades (thumbs up/down + comment) per turn; feeds quality metrics with complexity breakdown and memory impact analysis
- REST API — `GET /api/stats/efficiency`, `GET /api/recommendations`, `POST /api/recommendations/generate` (LLM-powered deep analysis)
- Trend tracking — sliding-window detection (improving/stable/declining) for cost and quality metrics

---

### 1.15 Sessions Chat Markdown Rendering

**Current state**: The sessions chat UI renders assistant/user messages as plain text. Markdown syntax (headings, lists, code fences, links, inline code, blockquotes) appears unformatted, which reduces readability for long-form answers and technical responses.

**Target**: First-class Markdown rendering in Sessions chat with safe HTML sanitization and readable defaults for prose and code blocks.

**Builds on**: `crates/ironclad-server/src/dashboard_spa.html`, session message APIs, existing chat rendering flow.

**Scope**: Add a Markdown parser + sanitizer in the dashboard chat pipeline. Render common Markdown constructs (headings, emphasis, lists, links, tables, blockquotes, fenced code blocks) and preserve plain-text fallback for malformed input. Add syntax highlighting for code fences and copy-to-clipboard actions per code block. Ensure external links use safe target/rel attributes and that raw HTML/script content is never executed.

**Implementation checklist**:
- **Libraries**: Evaluate and lock a parser/sanitizer pair (`marked` + `DOMPurify` or `markdown-it` + `sanitize-html`) plus lightweight syntax highlighting for fenced code blocks.
- **Security constraints**: Strip or escape all raw HTML by default, forbid inline event handlers/scripts, enforce safe link policies (`target="_blank"` + `rel="noopener noreferrer"`), and keep protocol allowlists to `https:`, `http:`, and `mailto:`.
- **Rendering behavior**: Support headings, emphasis, ordered/unordered lists, blockquotes, tables, inline code, and fenced code blocks; retain plaintext fallback when parsing fails.
- **UX polish**: Add copy-to-clipboard on code blocks, preserve line breaks where expected, and ensure dark/light theme readability for rendered Markdown elements.
- **Acceptance criteria**: Markdown test fixtures render correctly in Sessions chat, malicious payload fixtures are neutralized, no regression in existing chat message loading/perf, and snapshot/UI tests cover key Markdown constructs.

**Phased delivery**:
- **Phase A (MVP, M)**: Markdown parse + sanitize + render for core elements (headings, lists, links, inline/fenced code, blockquotes) with plaintext fallback.
- **Phase B (Code UX, S)**: Syntax highlighting and copy-to-clipboard for fenced code blocks, plus visual polish for dark/light themes.
- **Phase C (Hardening, M)**: Security regression fixtures (XSS/script/link payloads), rendering snapshot tests, and performance checks on long chat transcripts.

---

### 1.13 Capacity-Aware Routing

**Current state**: Token counts are tracked per turn (`tokens_in`/`tokens_out` in `format.rs`, stored in `turns`). Per-IP API rate limiting exists (`rate_limit.rs`). But there is no per-provider token-rate tracking — the router has no visibility into provider saturation and cannot pre-route around congestion. Hitting a 429 triggers the circuit breaker reactively.

**Target**: Proactive capacity-aware routing that deprioritizes saturated providers before hitting rate limits, distributing traffic to providers with available headroom.

**Builds on**: `ironclad-llm/src/router.rs`, `ironclad-llm/src/circuit.rs`, `ProviderConfig`.

**Scope**: Add a `CapacityTracker` per provider with sliding-window TPM (tokens-per-minute) and RPM (requests-per-minute) counters. Each tracker exposes a `headroom()` score (0.0 = saturated, 1.0 = idle). Integrate as a `CapacitySignal` in the `ModelRouter` — `select_for_complexity()` multiplies the quality score by headroom so saturated providers are deprioritized before hitting 429. Traffic naturally flows to providers with capacity. The `CapacityTracker` also feeds the existing `CircuitBreakerRegistry` — a provider at >90% capacity for a sustained period gets a preemptive half-open state. Config: `[models.capacity]` with per-provider `tpm_limit` and `rpm_limit`.

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

### 2.7 WASM Plugin Runtime

**Current state**: Plugin SDK uses script-based execution (spawn process, capture stdout). Functional but each tool call pays process-spawn overhead and has limited sandboxing.

**Target**: Plugins compiled to WASM run in a `wasmtime` sandbox with memory limits, capability restrictions, and zero process-spawn overhead.

**Research basis**: Section 7.2 of research-alternatives.md.

**Scope**: Add `wasmtime` dependency. Define a WASM ABI for tool execution (JSON in, JSON out). `WasmPlugin` implements the `Plugin` trait. Maintain script-based plugins as a fallback for tools that need filesystem or network access (WASM sandbox restricts these by default).

---

### 2.8 Prompt Compression

**Current state**: Context assembly uses progressive loading (L0–L3) and structural dedup. No token-level pruning.

**Target**: Remove low-importance tokens from prompts using perplexity-based scoring. 2–20x compression with <5% quality loss.

**Research basis**: LLMLingua / LongLLMLingua, Section 5.1 of research-alternatives.md.

**Scope**: Run a small local model (or use the primary local model) to score token importance via perplexity. Remove tokens below a threshold before sending to the inference model. Most impactful for long conversation histories and large tool descriptions. Gate behind config: `cache.prompt_compression = true`.

---

### 2.9 Declarative Agent Manifests

**Current state**: `SubagentRegistry` tracks child agent metadata (name, capabilities, status) but has no mechanism for defining what a specialist agent is — its personality, allowed tools, model preferences, or scheduling rules. Defining a new specialist requires code changes.

**Target**: Specialist agents defined as TOML manifest files — declarative, validated at boot, extensible without code changes. Users create new specialists by writing a config file.

**Builds on**: `ironclad-core/src/config.rs` (TOML parsing, validation), `ironclad-agent/src/skills.rs` (`SkillLoader` pattern), `SubagentRegistry`, `ToolRegistry`.

**Scope**: Define an `AgentManifest` schema: personality fields, capability declarations, model tier preferences, tool scope restrictions (whitelist), memory budget overrides, and optional cron triggers. Implement `ManifestLoader` (analogous to `SkillLoader`) with schema validation and SHA-256 change detection for hot-reload. Each specialist gets a scoped `SessionScope::Agent` session and a restricted `ToolRegistry` containing only the tools listed in its manifest. Manifests live in a configurable `agents/` directory. Example: `agents/morning-briefing.toml` declares capabilities `["summarization", "scheduling"]`, preferred model tier `T2`, and a cron trigger. Wire to the orchestration system (3.3) for coordinator-driven dispatch.

---

### 2.10 Structured Workspace System

**Current state**: The personality system provides 4 TOML files (OS/FIRMWARE/OPERATOR/DIRECTIVES) that define the agent's identity and behavioral rules. No structured mechanism exists for operational context — goals, security boundaries, integration metadata, or task tracking state.

**Target**: A structured, validated, version-tracked workspace that gives the agent persistent operational context beyond personality. Separates structured state (validated TOML) from unstructured reference material (indexed documents).

**Builds on**: `ironclad-core/src/personality.rs`, `soul_history` table, config validation.

**Scope**: Extend the personality system with a `[workspace]` config section pointing to a workspace directory. Define TOML schemas for workspace document types: `goals.toml` (short/medium/long-term goals with status tracking), `security.toml` (red lines, sensitive paths, breach protocols), `integrations.toml` (platform connections, data flows — validated at boot). Documents are versioned in the `soul_history` table so changes are tracked. On boot, the system diffs current vs. previous workspace state and surfaces changes to the agent. Unstructured documents (markdown notes, reference material) go in a `workspace/docs/` subdirectory and are indexed by the knowledge source system (2.11) for RAG retrieval.

---

### 2.11 Knowledge Source Trait

**Current state**: The RAG pipeline retrieves from internal memory tiers only (episodic, semantic, procedural, relationship, working). No mechanism exists to ingest or query external knowledge — filesystem documents, git repositories, vector databases, or graph stores are invisible to the agent.

**Target**: A trait-based knowledge source system that integrates external data into the existing RAG pipeline. Local sources are indexed into Ironclad's storage; remote sources are queried federatively at retrieval time. The `MemoryRetriever` treats external knowledge as another tier alongside internal memory.

**Builds on**: `ironclad-agent/src/retrieval.rs` (chunking, hybrid search), `ironclad-llm/src/embedding.rs`, `ironclad-db/src/embeddings.rs`, `ironclad-db/src/ann.rs` (HNSW index).

**Scope**: Define a `KnowledgeSource` trait with methods `scan()`, `ingest()`, `watch()`, and `query()`. Ship four implementations:

- **`DirectorySource`** — watches a filesystem directory for markdown/text/code files, incrementally indexes new and changed files via inotify/kqueue. Content is chunked via `chunk_text()`, embedded via `EmbeddingClient`, and stored in the `embeddings` table with source metadata.
- **`GitSource`** — indexes a git repository, re-indexes on new commits. Tracks file history for provenance metadata on retrieved chunks.
- **`VectorDbSource`** — connects to external vector databases (Qdrant, Weaviate, Milvus, Chroma) as federated retrieval backends. Queries are dispatched at retrieval time alongside the local HNSW index, with results merged by score. Enables purpose-built vector infrastructure for large corpora (millions of embeddings) where the local index would bottleneck.
- **`GraphSource`** — connects to graph databases (Neo4j, SurrealDB, or any Bolt/HTTP-compatible store) for relationship-aware knowledge retrieval. Supports Cypher queries for traversal-based context assembly — "what entities are related to X within N hops?" Enriches RAG results with structural context: related entities, dependency chains, and causal relationships that flat vector similarity cannot capture.

Local sources (Directory, Git) ingest into Ironclad's own storage. Remote sources (VectorDb, Graph) are queried federatively at retrieval time and merged into `MemoryRetriever` scoring. Config: `[knowledge.sources]` array of `{ type, path/url, pattern, poll_interval, auth }`.

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

## Tier 3 — Frontier

Ambitious capabilities that push the architecture into new territory. High effort, high potential.

### 3.1 Compile-Time Agent Safety (Typestates)

**Current state**: Agent lifecycle states (`Setup`, `Waking`, `Running`, `Sleeping`, `Dead`) are runtime enums. Policy evaluation is a runtime check.

**Target**: Use Rust's type system to make illegal state transitions compile errors. A `Tool<Unevaluated>` cannot be executed — only `Tool<Allowed>` can.

**Research basis**: Section 7.1 of research-alternatives.md.

**Scope**: Refactor `AgentLoop` to use typestate pattern. Introduce phantom type parameters on `ToolCallRequest` that carry policy evaluation results. Financial limits as const generics on treasury types. This is a deep refactor with compounding safety benefits.

---

### 3.2 MCP (Model Context Protocol) Integration

**Current state**: No MCP support. Custom A2A protocol exists for agent-to-agent communication.

**Target**: Expose Ironclad's tools and resources via MCP, and consume external MCP servers as tool providers. Makes Ironclad interoperable with the MCP ecosystem (IDE integrations, external data sources, third-party tools).

**Scope**: Implement MCP server mode (expose tool registry, memory search, session management as MCP resources and tools). Implement MCP client mode (discover and call tools from configured MCP servers). Add to config: `[mcp]` section with server/client settings.

---

### 3.3 Multi-Agent Orchestration

**Current state**: Single agent with A2A protocol for peer communication. `SubagentRegistry` tracks child agent metadata but doesn't orchestrate workflows.

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

---

### 3.6 Voice Channels

**Current state**: Text-only channels (Telegram, WhatsApp, Discord, WebSocket). No voice input or output capability.

**Target**: Voice input/output via Telegram voice messages, WhatsApp audio, and a WebRTC channel for the dashboard. TTS output as a standalone near-term deliverable before full WebRTC.

**Scope**: Speech-to-text via `whisper-rs` (native Rust, local-first) with cloud STT as opt-in fallback. Text-to-speech as a separately shippable milestone: support local TTS models (Piper, Coqui) as the default, consistent with the local-first philosophy, with cloud TTS (provider APIs) as opt-in. Config: `[channels.voice]` with `stt_model`, `tts_model`, `tts_voice`. Stream audio via WebRTC for real-time voice conversation on the dashboard. Store transcripts in session history alongside audio references.

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

**Status**: Implemented in 0.5.0. `[network]` config section with `bind_address`, `bind_interface`, and optional `advertise` list. Ironclad binds to whatever interface the operator specifies — loopback (default), LAN, Tailscale, WireGuard, or `0.0.0.0`. Optional mTLS via `[network.tls]` with `cert`, `key`, `ca` for untrusted network deployments.

---

### 3.13 Zero-Trust Global Remote UI Access

**Current state**: The dashboard can be exposed on non-loopback interfaces (`3.12`) and API auth exists, but remote access is still mostly perimeter-based. There is no dedicated zero-trust access layer for secure global UI access with strong identity, device trust, and session-hardening defaults.

**Target**: Secure global remote access to the Ironclad UI by default. Treat every network as untrusted and require cryptographic identity, strong authentication, hardened sessions, and explicit authorization for all UI/API traffic.

**Builds on**: `3.12 Flexible Network Binding`, API auth/session routes, TLS support, existing policy and audit infrastructure.

**Scope**: Add a dedicated remote access security layer with defense-in-depth: mandatory TLS (with optional mTLS for operator/admin roles), OIDC/SAML SSO + enforced MFA + short-lived tokens, device trust (key-bound sessions and optional passkey/WebAuthn), per-route RBAC for dashboard/API actions, IP reputation + geo-anomaly detection with adaptive challenge/deny, rate-limit and WAF hooks for auth surfaces, strict CSRF/CORS/cookie hardening, signed session rotation and revocation, comprehensive audit trails for auth/admin actions, and a "remote-lockdown" mode that defaults to deny-by-default except explicit allowlists. Ship with threat-model documentation, security runbooks, and hardened production presets.

---

## Summary

| # | Item | Tier | Builds On | Effort |
|---|------|------|-----------|--------|
| 1.1 | ~~Streaming LLM responses~~ ✅ | 1 | LLM client, WebSocket, channels | ~~Medium~~ Done |
| 1.2 | ~~Approval workflow API~~ ✅ | 1 | ApprovalManager (complete) | ~~Low~~ Done |
| 1.3 | ~~Browser as agent tool~~ ✅ | 1 | ironclad-browser (complete) | ~~Low~~ Done |
| 1.4 | Discord WebSocket gateway | 1 | Discord adapter (mostly done) | Low |
| 1.5 | ~~Embedding provider integration~~ ✅ | 1 | embeddings.rs, ProviderConfig | ~~Medium~~ Done |
| 1.6 | Multimodal messages + voice transcription | 1 | WhatsApp media parsing, format.rs, whisper-rs | Medium |
| 1.7 | ~~Memory-augmented agent pipeline~~ ✅ | 1 | MemoryBudgetManager, build_context, hybrid_search, 1.5 | ~~High~~ Done |
| 1.8 | Email channel adapter | 1 | ChannelAdapter trait, OAuthManager | Medium |
| 1.9 | Session scoping and lifecycle | 1 | sessions.rs, HeartbeatTask, compaction | Medium |
| 1.10 | ~~Addressability filter~~ ✅ | 1 | ChannelAdapter trait, InboundMessage | ~~Low~~ Done |
| 1.11 | Context checkpoint | 1 | schema.rs, build_context, MemoryRetriever | Medium |
| 1.12 | ~~Response transform pipeline~~ ✅ | 1 | format.rs, injection defense, turns | ~~Low-Medium~~ Done |
| 1.14 | ~~Context Observatory~~ ✅ | 1 | efficiency.rs, turn_feedback, turns, inference_costs | Done |
| 1.15 | Sessions chat Markdown rendering | 1 | dashboard_spa.html, session message rendering | Low |
| 1.13 | Capacity-aware routing | 1 | ModelRouter, CircuitBreakerRegistry | Medium |
| 2.1 | ML-based model routing | 2 | Heuristic router, RouterBackend trait | High |
| 2.2 | Accuracy-target routing | 2 | Router infrastructure | High |
| 2.3 | Tiered inference pipeline | 2 | Fallback chain, local model config | Medium |
| 2.4 | Speculative execution | 2 | Tool registry, tokio tasks | Medium |
| 2.5 | Service revenue & inbound payments | 2 | Wallet, treasury, A2A | High |
| 2.6 | Multi-provider cost arbitrage | 2 | ProviderConfig, router | Medium |
| 2.7 | WASM plugin runtime | 2 | Plugin SDK, ToolDef | High |
| 2.8 | Prompt compression | 2 | Context assembly, tier.rs | Medium |
| 2.9 | Declarative agent manifests | 2 | Config, SkillLoader, SubagentRegistry | High |
| 2.10 | Structured workspace system | 2 | personality.rs, soul_history | Medium |
| 2.11 | Knowledge source trait | 2 | retrieval.rs, EmbeddingClient, HNSW | High |
| 2.12 | Episodic digest system | 2 | memory.rs, compaction, retrieval | Medium |
| 2.13 | Hippocampus — self-describing schema map | 2 | schema.rs, Tool trait, policy engine | High |
| 3.1 | Compile-time agent safety | 3 | Agent loop, policy engine | High |
| 3.2 | MCP integration | 3 | Tool registry, config | High |
| 3.3 | Multi-agent orchestration | 3 | SubagentRegistry, A2A, 2.9 | High |
| 3.4 | Agent spawning + wallet provisioning | 3 | SubagentRegistry, wallet | High |
| 3.5 | Advanced RAG infrastructure (4/5 done ✅) | 3 | 1.5, 1.7, embeddings.rs | ~~High~~ Remaining: doc ingestion |
| 3.6 | Voice channels (TTS + STT + WebRTC) | 3 | Channel adapters, whisper-rs, local TTS | High |
| 3.7 | UniRoute model vectors | 3 | Provider registry, router | High |
| 3.8 | Game-theoretic cascades | 3 | Fallback chain, inference_costs logs | Medium |
| 3.9 | Storage backend trait | 3 | ironclad-db, schema.rs | High |
| 3.10 | Cryptographic device identity | 3 | Wallet keypairs, A2A mutual auth | High |
| 3.11 | Agent discovery protocol | 3 | Agent card, A2A handshake, DNS-SD | Medium |
| 3.12 | ~~Flexible network binding~~ ✅ | 3 | Server bind, auth.rs | ~~Low~~ Done |
| 3.13 | Zero-trust global remote UI access | 3 | 3.12, auth/session stack, policy/audit infra | High |
