# Research: Bot Construction Approaches & Alternative Technologies

*Comprehensive survey of known and novel approaches to autonomous agent construction.*

---

## 1. Established Agent Frameworks (2025-2026)

### 1.1 — LangGraph (LangChain Inc.)

**Architecture**: Directed graphs where nodes are actions and edges define control flow. Supports both DAGs and cyclic graphs.

**Strengths**:

- Production standard for complex stateful workflows
- Human-in-the-loop checkpointing
- Built-in observability via LangSmith
- Deterministic routing with error recovery

**Weaknesses**:

- Python-only (inherits Python's performance characteristics)
- Heavy abstraction layer adds latency and debugging complexity
- LangChain dependency chain is massive (hundreds of transitive dependencies)
- Not designed for autonomous/self-sustaining agents

**Relevance to Ironclad**: Graph-based workflow patterns are useful for tool orchestration, but the Python runtime and heavy abstraction are dealbreakers for an efficiency-first design.

### 1.2 — CrewAI

**Architecture**: Role-based multi-agent collaboration. Agents defined with roles, goals, and backstories organized into crews.

**Strengths**:

- Lowest learning curve
- Intuitive team coordination
- Good for straightforward multi-agent scenarios

**Weaknesses**:

- Limited to role-based patterns
- No financial autonomy or self-sustaining capabilities
- Python-only
- No built-in persistence beyond task context

**Relevance to Ironclad**: The role-based delegation pattern is useful for subagent delegation, but CrewAI's scope is too narrow for a full autonomous agent runtime.

### 1.3 — Microsoft Agent Framework (AutoGen + Semantic Kernel merger)

**Architecture**: Unified framework combining AutoGen's multi-agent orchestration with Semantic Kernel's enterprise runtime. Graph-based workflows with sequential, concurrent, handoff, and group chat patterns.

**Strengths**:

- Enterprise-ready: observability, approvals, security, durability
- MCP and A2A protocol support built-in
- .NET and Python support
- Release Candidate status (stable API)

**Weaknesses**:

- .NET or Python only (no Rust, no single-binary deployment)
- Enterprise-focused (heavy infrastructure assumptions)
- No financial autonomy or crypto integration
- No self-modification capabilities

**Relevance to Ironclad**: MCP integration patterns and graph-based workflow design are worth studying. The enterprise guardrails (approvals, observability) inform policy engine design.

### 1.4 — Pydantic AI

**Architecture**: Type-safe agent framework using Pydantic models for structured inputs/outputs.

**Strengths**:

- Strong type safety at runtime (via Pydantic validation)
- Clean structured output handling
- Production-oriented

**Weaknesses**:

- Python-only
- Limited orchestration capabilities
- No persistence or memory system
- No financial/crypto features

**Relevance to Ironclad**: The typed output approach directly maps to Rust's type system, where we can achieve the same guarantees at compile time rather than runtime.

---

## 2. Rust-Native Agent Frameworks (2026)

### 2.1 — AutoAgents (Rust)

**Benchmarks** (2026 comprehensive benchmark):

- Memory: 1,046 MB peak (vs 4,718 MB for GraphBit JS/TS) — 4.5x improvement
- Latency: 5,714 ms average (vs 8,425 ms for GraphBit) — 1.5x improvement
- Throughput: 4.97 req/s (vs 3.14 req/s for GraphBit) — 1.6x improvement
- At scale (50 instances): ~51 GB total (vs ~230 GB) — 4.5x improvement

**Architecture**: Tokio-based async runtime with typed tool interfaces.

### 2.2 — AgentSDK (Rust)

**Benchmarks**:

- Idle memory: 12 MB (vs LangChain 218 MB) — 18x improvement
- Cold start: 2.3 ms (vs LangChain 108 ms) — 47x improvement
- Throughput: 8,400 req/s (vs LlamaIndex 920 req/s) — 9x improvement

**Architecture**: Minimal core with plugin system. Focused on raw performance.

### 2.3 — Tokio-FSM

**Purpose**: Compile-time validated, zero-overhead async finite state machines for Tokio.

**Key features**:

- Declarative state machine definition via macros
- Compile-time validation of state transitions (directed graph analysis)
- Zero-cost: no runtime engine overhead, tight match loops
- Stack-pinned timeouts (zero heap allocation)

**Relevance to Ironclad**: Perfect for modeling agent lifecycle states (setup → waking → running → sleeping → dead) and child lifecycle (spawning → provisioning → alive → unhealthy → dead).

### 2.4 — Tokio-Agent

**Purpose**: Elixir-inspired agent pattern for Tokio — manages state within a task.

**Key features**:

- `Handle` (async) and `BlockingHandle` (sync) for state interaction
- Lock-free resource management
- Compatible with blocking I/O (SQLite)

**Relevance to Ironclad**: Useful pattern for managing per-agent state without shared mutexes.

---

## 3. ML-Based Model Routing

### 3.1 — RouteLLM (LMSYS)

**Concept**: Machine learning classifier that predicts whether a query needs a strong or weak model. Routes simple queries to cheap models, complex queries to expensive ones.

**Performance**:

- Router overhead: 11 microseconds per classification
- Cost savings: ~60% with <5% quality degradation
- If 70% of queries use GPT-5-mini ($0.25) instead of GPT-5.2 ($1.75): 60% input cost savings

**Architecture**: Small classifier model trained on preference data. Can use:

- Logistic regression on embeddings (fastest, ~11μs)
- Small neural network (more accurate, ~1ms)
- LLM-as-judge (most accurate, ~100ms)

**Relevance to Ironclad**: Research informed routing design; **actual implementation uses a heuristic classifier** (weighted message length, tool calls, depth) — no ONNX or ML runtime.

### 3.2 — PROTEUS (2026)

**Concept**: Accepts accuracy targets (τ) as runtime input using Lagrangian optimization. Routes across any set of models to minimize cost while maintaining specified quality floors.

**Performance**: 89.8% cost savings while maintaining specified quality thresholds across the full accuracy spectrum.

**Relevance to Ironclad**: The accuracy-target approach is novel — let the user specify "I need 95% quality" and the system optimizes cost automatically.

### 3.3 — UniRoute (2026)

**Concept**: Handles routing to previously unseen LLMs by representing each model as a feature vector. Can route among 30+ unknown models without retraining.

**Relevance to Ironclad**: Future-proofing — as new models launch, UniRoute-style vectors let the router adapt without retraining.

### 3.4 — Game-Theoretic Routing (2026)

**Concept**: Optimal static routing policies based on expected utility and model rankings. Determines when cascading (try cheap model first, escalate if bad) helps vs hurts.

**Key finding**: Cascading is only beneficial when the weak model's expected quality is above a threshold. Below that, direct routing to the strong model is optimal.

**Relevance to Ironclad**: Informs the design of fallback chains — sometimes it's cheaper to skip the weak model entirely.

---

## 4. Semantic Caching

### 4.1 — GPTCache (Open Source)

**Architecture**: Embedding-based cache with configurable similarity threshold.

**How it works**:

1. Compute embedding of incoming prompt
2. Search cache for similar embeddings (cosine similarity > threshold)
3. If hit: return cached response
4. If miss: forward to LLM, cache response

**Performance**: 15-30% cache hit rate for conversational agents with repetitive queries.

### 4.2 — Local Embedding for Cache Keys

**Approach**: Use a small local embedding model (e.g., all-MiniLM-L6-v2 via ONNX, 22 MB) to compute cache keys without hitting an external API.

**Latency**: <5ms for embedding computation locally.

**Relevance to Ironclad**: **Actual implementation**: in-memory HashMap (`SemanticCache`) with exact hash, tool TTL, and n-gram semantic similarity — no ONNX, no SQLite cache at runtime.

### 4.3 — Hierarchical Caching

**Levels**:

1. **Exact match** (hash of full prompt) — O(1) lookup, 100% precision
2. **Semantic match** (embedding similarity) — O(log n) lookup, ~95% precision
3. **Tool result cache** (deterministic tools with TTL) — per-tool caching

**Relevance to Ironclad**: Implement all three levels. Exact match handles identical repeated queries. Semantic match handles paraphrases. Tool caching handles status checks.

---

## 5. Prompt Compression

### 5.1 — LLMLingua / LongLLMLingua

**Concept**: Remove tokens from prompts that contribute least to output quality. Uses perplexity-based token importance scoring.

**Performance**: 2-20x compression with <5% quality loss depending on content type.

### 5.2 — Structural Deduplication

**Concept**: For agent systems, much of the system prompt is repeated across turns. Instead of sending the full prompt each time:

1. Send full prompt on first turn (or after compaction)
2. On subsequent turns, send a hash reference to the static prefix
3. Only send dynamic sections (memory, recent history) in full

**How this works in practice**: Anthropic's `cache_control` is exactly this pattern. For other providers, maintain a server-side prompt template with parameterized slots.

### 5.3 — Progressive Context Loading

**Concept**: Don't load all context upfront. Start with minimal context, expand only if the model requests more.

**Levels**:

1. **Level 0**: Core identity + current task only (~2K tokens)
2. **Level 1**: Add relevant memories (~4K tokens)
3. **Level 2**: Add full tool descriptions (~8K tokens)
4. **Level 3**: Add full history window (~16K tokens)

Most simple queries can be answered at Level 0-1, saving 50-75% of input tokens.

**Relevance to Ironclad**: Combine with ML routing — if the router classifies a query as "simple," use Level 0-1 context. If "complex," use full context.

---

## 6. Self-Sustaining Agent Economics

### 6.1 — x402 Payment Protocol

**Current state**: The x402 protocol (HTTP 402 with signed USDC payments) is already a proven mechanism for purchasing credits.

**Limitation**: Credits can only be spent on a single provider's services. The agent cannot buy inference from other providers or pay for arbitrary services.

### 6.2 — Yield on Idle Treasury

**The problem**: AI agents hold USDC as operational float earning 0% APY. McKinsey projects agentic commerce at $3-5 trillion by 2030. Idle balances could reach $10-30 billion — $500M to $2.4B in unrealized annual yield.

**Solution**: Automated yield strategies for idle USDC:

- **Aave on Base**: Deposit USDC into Aave lending pool. Current yield: 4-6% APY. Instant withdrawal.
- **Compound on Base**: Similar to Aave. 3-5% APY.
- **Circle Earn**: Institutional yield on USDC. 4.5% APY. Less composable.

**Safety**: Only deposit excess above operational float. Example:

- Operational float: $50 (enough for ~1 week of inference)
- Any USDC above $50: deposit into Aave
- When balance drops below $30: withdraw from Aave to replenish

**Relevance to Ironclad**: This is a differentiator. Typical agent systems let USDC sit idle. Ironclad can earn 4-8% APY on its treasury, making it partially self-funding.

### 6.3 — Service Revenue

**Concept**: The agent can offer services (code review, content generation, research) and receive USDC payments directly. Combined with yield generation, this creates a self-sustaining economic loop:

```text
Earn USDC (services) → Deposit excess (Aave) → Earn yield →
Withdraw for compute → Buy inference → Deliver services → Repeat
```

### 6.4 — Multi-Provider Cost Arbitrage

**Concept**: Different providers have different pricing at different times. An agent that monitors real-time pricing can route queries to the cheapest provider that meets quality requirements.

**Example**: If Moonshot offers Kimi K2.5 at $0.50/M tokens and Google offers Gemini Flash at $0.75/M tokens for equivalent quality, route to Moonshot. If Moonshot's circuit breaker trips, route to Google.

**Relevance to Ironclad**: The ML router should include provider pricing as a feature alongside query complexity.

---

## 7. Novel Approaches

### 7.1 — Compile-Time Agent Safety (Rust-Specific)

**Concept**: Use Rust's type system to enforce agent safety at compile time rather than runtime.

**Examples**:

- **State machine as type states**: The agent can only call `sleep()` from the `Running` state. Calling it from `Dead` is a compile error.
- **Policy as phantom types**: A tool call wrapper that carries its policy evaluation result in the type system. You can't execute a tool without first evaluating policy — enforced by the compiler.
- **Financial limits as const generics**: Treasury policy limits embedded as const generics, making it impossible to construct a payment above the cap.

**Relevance to Ironclad**: This is the core advantage of Rust. Safety guarantees that typical agent systems achieve through runtime checks (policy engine, path protection) become compile-time guarantees.

### 7.2 — WASM Plugin System

**Concept**: Instead of a fixed tool set, allow tools to be loaded as WebAssembly modules at runtime. Each tool runs in a sandboxed WASM environment with memory limits and capability restrictions.

**Benefits**:

- Tools can be written in any language that compiles to WASM
- Sandboxed execution — a buggy tool can't crash the runtime
- Memory-limited — prevents OOM from runaway tools
- Hot-reloadable — add new tools without restart

**Relevance to Ironclad**: Use `wasmtime` (Rust-native WASM runtime) for the plugin system. Core tools compiled natively, user-defined tools as WASM plugins.

### 7.3 — Speculative Execution for Agent Loops

**Concept**: While waiting for an LLM response, speculatively prepare for likely next actions:

- Pre-fetch tool results the model is likely to request
- Pre-compute context for likely follow-up questions
- Pre-warm inference for fallback models

**Implementation**:

1. After sending an inference request, analyze the conversation to predict likely tool calls
2. Execute those tool calls in parallel (read-only ones only)
3. When the LLM response arrives, if it requests a pre-fetched tool, return immediately
4. Expected latency reduction: 30-50% for predictable tool sequences

### 7.4 — Tiered Inference Pipeline

**Concept**: Instead of routing to a single model, use a pipeline:

```text
Query → Local Small Model (qwen3:8b, ~100ms) → Confidence Check
    If confident (>0.9): return immediately
    If uncertain: escalate to cloud model (GPT-5.3, ~2s)
```

**Performance**: 70% of queries answered by the local model in <200ms. Only 30% escalate to cloud, saving 70% of cloud API costs.

**Relevance to Ironclad**: Combined with semantic caching, this creates a 3-layer response pipeline:

1. Cache hit → 5ms
2. Local model confident → 200ms
3. Cloud model → 2000ms

---

## 8. Context Observability & Quality Optimization

### 8.1 — Context Observability Approaches

**Problem**: LLM-powered agents assemble context from multiple sources (system prompt, memories, conversation history, tool results) but operators have no visibility into what's being sent, how much it costs, or whether it's effective.

**Commercial solutions**:

| Platform | Approach | Strengths | Weaknesses |
| --- | --- | --- | --- |
| **LangSmith** (LangChain) | Trace-based observability for LangChain graphs. Records every chain step, prompt, and completion. | Deep integration with LangGraph. | Python-only. Requires LangChain dependency. SaaS-only production tier. |
| **Arize Phoenix** | Open-source LLM observability. Traces, evaluations, prompt experimentation. | Self-hostable. Language-agnostic via OTLP. | Focused on evaluation rather than real-time optimization. No cost attribution at the context-component level. |
| **Helicone** | Proxy-based logging. Sits between the app and LLM provider to capture all requests/responses. | Zero-code integration (HTTP proxy). | Cannot inspect context assembly — only sees the final prompt. No per-component attribution. |
| **Weights & Biases Weave** | Experiment tracking adapted for LLM workflows. | Strong visualization. Experiment comparison. | Heavy infrastructure. Not designed for production real-time monitoring. |
| **Braintrust** | Eval framework with logging. Prompt playground and scoring. | Good eval-first workflow. | Limited cost optimization. No memory/RAG effectiveness analysis. |

**Ironclad's approach** (Context Observatory, 0.5.0): Embedded analytics that decompose context at the component level — system prompt, memories, history — and attribute costs to each. Turn-level outcome grading feeds quality metrics. No external dependencies. Runs inside the single binary alongside the agent. Advantages over external tools: (1) sees context before assembly, not just the final prompt, (2) can attribute costs to specific memory tiers and RAG retrievals, (3) outcome grades directly tied to the turns that produced them, (4) zero infrastructure — no proxy, no SaaS, no separate database.

### 8.2 — Prompt Efficiency Research

**Token-level efficiency**: Most LLM applications waste 30-60% of input tokens on boilerplate, redundant context, and low-information memories. The key insight is that not all context tokens contribute equally to output quality.

**Relevant research**:

- **Lost in the Middle** (Liu et al., 2024): LLMs attend most to the beginning and end of long contexts, largely ignoring middle content. Implication: position matters more than volume. Ironclad's progressive context loading (L0-L3) addresses this by placing the most relevant content first.
- **Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks** (Lewis et al., 2020): Foundational RAG paper showing that retrieval quality dominates generation quality. Fewer, more relevant documents outperform many marginally-relevant ones. Ironclad's hybrid search (FTS5 + vector cosine) with configurable `hybrid_weight` implements this.
- **Anthropic's prompt caching** (`cache_control` headers): Demonstrates that static context (system prompt, tool descriptions) should be cached across turns. Ironclad's tier adaptation system applies `cache_control` for T3/T4 providers and uses structural deduplication for others.

**Efficiency metrics implemented**: Output density (useful tokens / total output), budget utilization (tokens used / allocated), system prompt weight (% of input consumed by static context), wasted budget cost (monetary value of unused context slots).

### 8.3 — Quality-Cost Optimization Literature

**The quality-cost tradeoff**: Higher-capability models produce better outputs but cost more. The optimization challenge is finding the minimum-cost configuration that meets a quality threshold for each query type.

**Key findings**:

- **FrugalGPT** (Chen et al., 2023): Cascading LLM APIs with learned stopping criteria. Try cheap models first; escalate only when confidence is low. Achieves 98% quality retention at 50% cost reduction. Relevant to Ironclad's heuristic router and tiered inference pipeline.
- **Routing to the Expert** (Ong et al., 2024): Shows that routing accuracy matters more than model capability for overall system quality. A perfect router with a mix of weak/strong models outperforms always using the strong model. Validates Ironclad's investment in routing over model upgrades.
- **Quality Diversity in LLM Outputs** (2025): Different models excel at different task types. Code generation, creative writing, and factual QA have different optimal model selections. Ironclad's Context Observatory enables this analysis by tracking quality metrics per model per complexity level.

**Ironclad's approach**: The Context Observatory provides the data infrastructure to pursue quality-cost optimization. Turn feedback + efficiency metrics + cost attribution give operators the signals needed to tune routing thresholds, adjust memory budgets, and identify which context components drive quality vs. waste tokens.

---

## 9. Actual Technology Choices (Ironclad)

What Ironclad uses in practice:

| Choice | Implementation |
| -------- | ----------------- |
| **Routing** | Heuristic classifier (message length, tool calls, depth). Configured modes are `"primary"`, `"metascore"`, and `"heuristic"` (heuristic currently aliases metascore behavior). No ONNX. |
| **Cache** | In-memory HashMap (`SemanticCache` in ironclad-llm). L1 exact hash, L3 tool TTL, L2 n-gram similarity. No SQLite cache, no Redis. |
| **Database** | SQLite via **rusqlite** (not diesel). Single DB, WAL mode. |
| **Server** | **Axum** (not Actix). |
| **Ethereum** | **alloy-rs** for chain and contracts. |
| **Policy rules** | 6 rules: AuthorityRule, CommandSafetyRule, FinancialRule, PathProtectionRule, RateLimitRule, ValidationRule. |
| **Money type** | Treasury uses **Money(i64 cents)** internally; API surface is f64 (dollars). |
| **FTS** | `memory_fts` (FTS5) with triggers on `episodic_memory`; working_memory inserts explicitly. |
| **Yield** | Aave V3 on Base Sepolia (yield_engine.rs). |
| **A2A crypto** | x25519-dalek ECDH, AES-256-GCM, HKDF. |
