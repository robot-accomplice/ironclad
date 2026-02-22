# Ironclad Roadmap

*Growth areas organized by effort and impact. Each item notes whether it builds on existing code or is greenfield.*

---

## Tier 1 — Wire the Last Mile

Capabilities where the core code exists but isn't fully connected. High impact, low-to-medium effort.

### 1.1 Streaming LLM Responses

**Current state**: `LlmClient` buffers the entire LLM response before returning it. Users see nothing until the full response arrives.

**Target**: Stream tokens to the user in real-time via WebSocket and channel adapters as they arrive from the provider.

**Builds on**: `ironclad-llm/client.rs` (reqwest already supports streaming), `ironclad-server/ws.rs` (EventBus broadcast), channel adapters.

**Scope**: Modify `forward_request()` to return a `Stream<Item = Bytes>` instead of `Value`. Propagate through the agent loop to WebSocket subscribers and channel adapters. Handle partial-response caching (only cache on completion).

---

### 1.2 Approval Workflow API

**Current state**: `ApprovalManager` in `ironclad-agent/approvals.rs` has full lifecycle (create request, approve, deny, timeout, cleanup) with tests. No API routes expose it.

**Target**: HTTP endpoints and channel-based approval prompts so operators can approve gated tool calls from Telegram, WhatsApp, or the dashboard.

**Builds on**: `ApprovalManager`, channel adapters, WebSocket push.

**Scope**: Add routes (`GET /api/approvals`, `POST /api/approvals/:id/approve`, `POST /api/approvals/:id/deny`). Push pending approvals via WebSocket and optionally via Telegram/WhatsApp inline keyboards. Wire the agent loop to pause on gated tools and resume on approval.

---

### 1.3 Browser as Agent Tool

**Current state**: `ironclad-browser` is a complete CDP automation crate (navigate, click, type, screenshot, evaluate, read page). It has server REST routes. But it is not registered as a `Tool` in the agent's `ToolRegistry`, so the agent cannot use it autonomously during the ReAct loop.

**Target**: The agent can autonomously decide to browse the web — research, verify information, fill forms, take screenshots.

**Builds on**: `ironclad-browser` crate, `ironclad-agent/tools.rs` Tool trait.

**Scope**: Implement `BrowserTool` (wraps Browser actions as Tool trait methods). Register in `ToolRegistry` under the `general` category. Policy: `RiskLevel::Caution` by default, `Dangerous` for `Evaluate` (arbitrary JS execution).

---

### 1.4 Discord WebSocket Gateway

**Current state**: Discord adapter handles message parsing, REST send, guild allowlists, rate limiting, and chunking. Missing: persistent WebSocket connection to the Discord Gateway for receiving real-time events.

**Target**: Full bidirectional Discord integration — receive messages in real-time without polling.

**Builds on**: `ironclad-channels/discord.rs`.

**Scope**: Implement Gateway identify, heartbeat, and dispatch event handling. Resume/reconnect on disconnect. Wire `MESSAGE_CREATE` events into the existing `parse_inbound()` path.

---

### 1.5 Embedding Provider Integration

**Current state**: `ironclad-db/embeddings.rs` has full vector storage, cosine similarity search, and hybrid (FTS + vector) search. But no code actually calls an embedding API — the semantic cache uses a local n-gram hash instead of neural embeddings.

**Target**: A multi-provider `EmbeddingClient` that can generate real embeddings via OpenAI, Ollama, or Google APIs, with graceful n-gram fallback when no provider is configured.

**Builds on**: `ironclad-db/embeddings.rs` (storage/search), `ironclad-llm/cache.rs` (lookup paths), `ProviderConfig` (provider registry).

**Scope**:

1. **Config** — Add `embedding_path`, `embedding_model`, and `embedding_dimensions` to `ProviderConfig` in `ironclad-core/config.rs`. Update `bundled_providers.toml`:
   - Ollama: `/api/embed`, `nomic-embed-text`, 768 dims
   - OpenAI: `/v1/embeddings`, `text-embedding-3-small`, 1536 dims
   - Google: `/v1beta/models/{model}:embedContent`, `text-embedding-004`, 768 dims
   - Local-only providers: no embedding path (fall back to n-gram)

2. **EmbeddingClient** — New `ironclad-llm/embedding.rs`:
   - `embed(texts: &[&str]) -> Vec<Vec<f32>>` (batch)
   - `embed_single(text: &str) -> Vec<f32>`
   - `fallback_ngram(text: &str, dim: usize) -> Vec<f32>`
   - Per-provider format translation (OpenAI vs. Ollama vs. Google request shapes)
   - Respects circuit breaker state for the embedding provider

3. **Wire into LlmService** — Add `embedding: Option<EmbeddingClient>` to `LlmService`, initialized from the provider whose name matches `memory.embedding_provider` config.

**Implementation plan**: [robust_embedding_rag](../.cursor/plans/robust_embedding_rag_831ff5b3.plan.md) Phase 1.

---

### 1.7 Memory-Augmented Agent Pipeline

**Current state**: The `agent_message` handler builds a flat `[system, user]` prompt. It does **not** call `build_context()`, `MemoryBudgetManager`, `hybrid_search()`, or `ingest_turn()`. All of this code exists and is tested — none of it is wired into the message path. This is the single largest disconnect in the codebase.

**Target**: Every agent turn retrieves relevant memories (5 tiers, budget-managed), assembles progressive context, and ingests the completed turn for future retrieval. The pipeline becomes:

```text
User Message → Injection Check → Session Lookup → Generate Query Embedding
→ Hybrid Search (FTS + Vector) → MemoryBudgetManager → build_context()
→ Cache Lookup → LLM Call → Output Safety → Store Response
→ ingest_turn() + Generate Embeddings → Return Response
```

**Builds on**: `MemoryBudgetManager` (complete), `build_context()` (complete), `hybrid_search()` (complete), `ingest_turn()` (complete), `EmbeddingClient` (from 1.5).

**Scope**:

1. **MemoryRetriever orchestrator** — New `ironclad-agent/retrieval.rs`. Queries all 5 memory tiers within the token budget allocated by `MemoryBudgetManager`. Accepts a query embedding for vector search and falls back to FTS-only when no embedding is available. Returns a formatted string ready for context injection.

2. **Wire retrieval into agent_message** — Between cache lookup and prompt assembly: generate query embedding, call `MemoryRetriever::retrieve()`, load conversation history from `sessions::get_messages()`, replace the flat `[system, user]` construction with `build_context(level, system_prompt, memories, history)`.

3. **Wire post-turn ingestion** — After storing the assistant response: call `ingest_turn()` to classify the turn and extract memories, generate an embedding for the response, store via `store_embedding()` for future retrieval.

4. **Conversation history** — Load the last N messages from the session's turn history and pass them to `build_context()` as the history parameter (currently not loaded at all).

**Implementation plan**: [robust_embedding_rag](../.cursor/plans/robust_embedding_rag_831ff5b3.plan.md) Phase 2.

---

### 1.6 Multimodal Message Handling

**Current state**: WhatsApp adapter parses image/video/audio/document types but converts them to text placeholders (`[image:id] caption`). The LLM client has no vision or multimodal support.

**Target**: Forward images to vision-capable models. Store image references in session history. Display images in dashboard.

**Builds on**: WhatsApp media parsing, Telegram media API, `ironclad-llm/format.rs` (format translation already handles content arrays).

**Scope**: Download media from channel APIs to temp storage. Construct multimodal content blocks (`image_url` for OpenAI, inline `image` for Anthropic). Gate behind a config flag (`models.multimodal = true`). Extend `UnifiedMessage` to carry binary content parts.

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

**Scope**: Extend `SubagentRegistry` to manage actual agent instances (each with its own session, tools, and optionally its own wallet). Define orchestration patterns: sequential, parallel fan-out/fan-in, and handoff. Coordinator agent routes subtasks to specialists based on capability matching.

---

### 3.4 Agent Spawning with Wallet Provisioning

**Current state**: `SubagentRegistry` handles lifecycle metadata. No wallet provisioning, no autonomous child agent execution.

**Target**: An agent can spawn a child agent, provision it with a fraction of its treasury, delegate a task, and reclaim funds on completion or timeout.

**Scope**: Generate a child wallet (derived from parent wallet or fresh keypair). Transfer USDC to child via the parent's treasury. Child inherits a restricted config (reduced caps, limited tool access, time-bounded). Parent monitors child's progress and reclaims remaining funds on completion.

---

### 3.5 Advanced RAG Infrastructure

**Current state**: After 1.5 and 1.7, the agent has real embeddings and a wired retrieval pipeline. But embeddings are stored as JSON text (slow, large), similarity search is O(n) brute-force, long documents aren't chunked, and the semantic cache is in-memory only.

**Target**: Production-grade RAG with binary vector storage, O(log n) approximate nearest neighbor search, content chunking, persistent semantic cache, and document ingestion.

**Builds on**: `EmbeddingClient` (1.5), `MemoryRetriever` (1.7), `embeddings.rs`, `semantic_cache` table.

**Scope**:

1. **Binary embedding storage** — Switch `embeddings` from JSON `TEXT` to `BLOB` (raw little-endian `f32` bytes via `byteorder` or `zerocopy`). ~4x storage reduction, no JSON parsing on every row during search. Migration to convert existing data.

2. **HNSW approximate nearest neighbor index** — Optional in-memory HNSW index using `instant-distance` (pure Rust, no C deps). Built at startup from the embeddings table, incrementally updated on new inserts. Falls back to brute-force scan when disabled or corpus is small (<1000 entries). Config: `[memory] ann_index = true/false`.

3. **Content chunking** — `Chunker` in `ironclad-agent/retrieval.rs`. Splits long content into overlapping chunks (512 tokens, 64-token overlap). Each chunk gets its own embedding and entry in the embeddings table, linked to the parent via `source_id`. Used during `ingest_turn()` for long responses and for document ingestion.

4. **Persistent semantic cache** — The `semantic_cache` table exists in the schema but `SemanticCache` is purely in-memory. Add `save_to_db()` / `load_from_db()` methods. Load on startup, periodically flush new entries, expire stale entries. Store real embeddings (from `EmbeddingClient`) instead of n-gram vectors for L2 semantic lookup.

5. **Document ingestion pipeline** — Ingest external documents (PDF, markdown, code files) into the memory system. Parse, chunk, embed, and store for retrieval. Extends the agent's knowledge base beyond conversation history.

**Implementation plan**: [robust_embedding_rag](../.cursor/plans/robust_embedding_rag_831ff5b3.plan.md) Phases 3–4.

---

### 3.6 Voice Channels

**Current state**: Text-only channels (Telegram, WhatsApp, Discord, WebSocket).

**Target**: Voice input/output via Telegram voice messages, WhatsApp audio, and a WebRTC channel for the dashboard.

**Scope**: Speech-to-text (Whisper API or local whisper.cpp). Text-to-speech (provider TTS API or local model). Stream audio via WebRTC for real-time voice conversation. Store transcripts in session history.

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

## Summary

| # | Item | Tier | Builds On | Effort |
|---|------|------|-----------|--------|
| 1.1 | Streaming LLM responses | 1 | LLM client, WebSocket, channels | Medium |
| 1.2 | Approval workflow API | 1 | ApprovalManager (complete) | Low |
| 1.3 | Browser as agent tool | 1 | ironclad-browser (complete) | Low |
| 1.4 | Discord WebSocket gateway | 1 | Discord adapter (mostly done) | Low |
| 1.5 | Embedding provider integration | 1 | embeddings.rs, ProviderConfig | Medium |
| 1.6 | Multimodal messages | 1 | WhatsApp media parsing, format.rs | Medium |
| 1.7 | Memory-augmented agent pipeline | 1 | MemoryBudgetManager, build_context, hybrid_search, 1.5 | High |
| 2.1 | ML-based model routing | 2 | Heuristic router, RouterBackend trait | High |
| 2.2 | Accuracy-target routing | 2 | Router infrastructure | High |
| 2.3 | Tiered inference pipeline | 2 | Fallback chain, local model config | Medium |
| 2.4 | Speculative execution | 2 | Tool registry, tokio tasks | Medium |
| 2.5 | Service revenue & inbound payments | 2 | Wallet, treasury, A2A | High |
| 2.6 | Multi-provider cost arbitrage | 2 | ProviderConfig, router | Medium |
| 2.7 | WASM plugin runtime | 2 | Plugin SDK, ToolDef | High |
| 2.8 | Prompt compression | 2 | Context assembly, tier.rs | Medium |
| 3.1 | Compile-time agent safety | 3 | Agent loop, policy engine | High |
| 3.2 | MCP integration | 3 | Tool registry, config | High |
| 3.3 | Multi-agent orchestration | 3 | SubagentRegistry, A2A | High |
| 3.4 | Agent spawning + wallet provisioning | 3 | SubagentRegistry, wallet | High |
| 3.5 | Advanced RAG infrastructure | 3 | 1.5, 1.7, embeddings.rs | High |
| 3.6 | Voice channels | 3 | Channel adapters | High |
| 3.7 | UniRoute model vectors | 3 | Provider registry, router | High |
| 3.8 | Game-theoretic cascades | 3 | Fallback chain, inference_costs logs | Medium |
