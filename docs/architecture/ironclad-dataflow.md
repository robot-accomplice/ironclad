# Ironclad Dataflow Diagrams

*Data flows for the Ironclad architecture -- a single Rust binary autonomous agent runtime.*

**Convention**: every SQLite table name, config key, crate name, and Rust type referenced in these diagrams is cross-referenced against `ironclad-design.md` in the cross-reference section at the end.

---

## 0. Runtime Config Reload Dataflow

`ironclad.toml` is now a runtime-reloadable source of truth. Update flows persist to disk first (with backup) and then apply to in-memory runtime state.

```mermaid
flowchart TD
    subgraph requestLayer [RequestLayer]
        op[OperatorCLIorAPI]
        patch[ConfigPatchOrFileEdit]
    end

    subgraph validationLayer [ValidationLayer]
        parse[ParseTomlOrJsonPatch]
        validate[SchemaAndSemanticValidate]
    end

    subgraph persistenceLayer [PersistenceLayer]
        backup[CreateTimestampedBackup]
        atomicWrite[AtomicWriteIroncladToml]
    end

    subgraph runtimeLayer [RuntimeApplyLayer]
        apply[ApplyToRuntimeState]
        sync[SyncRouterAndA2A]
        deferred[EmitDeferredApplyHints]
    end

    op --> patch
    patch --> parse
    parse --> validate
    validate --> backup
    backup --> atomicWrite
    atomicWrite --> apply
    apply --> sync
    sync --> deferred
```

## 1. Primary Request Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

End-to-end path from inbound user message to delivered response, entirely within one OS process.

0.6.0 targeted additions reflected in the runtime:

- Capacity-aware model selection now records per-provider throughput and feeds headroom + preemptive breaker pressure.
- Session lookup/create is scope-aware (`agent`, `peer`, `group`) in web and channel paths.
- Session and context UI rendering supports sanitized markdown in dashboard views.
- Local model onboarding now supports Apertus with SGLang-first host recommendation and resource-aware model filtering.
- Outbound channel retries are persisted in `delivery_queue` and recovered on restart before retry drain loops resume.
- Session rotation now evaluates `session.reset_schedule` cron expressions directly (including timezone-prefixed schedules) instead of top-of-hour polling.

```mermaid
flowchart TD
    subgraph installSetup [InstallAndSetup]
        optIn[UserOptsInForApertus]
        probe[ProbeSystemResources]
        detect[DetectLocalHosts]
        bootstrap[BootstrapLocalHost]
        choose[ChooseEligibleModel]
    end

    subgraph hostAdapters [HostAdapters]
        sglang[SGLangRecommended]
        vllm[vLLMFallback]
        dockerRunner[DockerModelRunnerFallback]
        ollama[OllamaFallback]
    end

    optIn --> probe
    probe --> detect
    detect --> bootstrap
    detect --> choose
    bootstrap --> choose
    choose --> sglang
    choose --> vllm
    choose --> dockerRunner
    choose --> ollama
```

```mermaid
flowchart TD
    subgraph Intake["① Intake & Screening"]
        USER["User Message<br/>(Telegram / WhatsApp / WebSocket)"]
        USER --> ADAPTER["ironclad-channels · Channel Adapter"]
        ADAPTER --> SESSION_LOOKUP["ironclad-db/sessions.rs<br/>Lookup or create session"]
        SESSION_LOOKUP --> INJECTION_L1["ironclad-agent/injection.rs<br/>L1: Input Gatekeeping<br/>(regex, encoding, authority,<br/>financial, multi-lang)<br/>→ ThreatScore 0.0–1.0"]
        INJECTION_L1 --> THREAT_CHECK{"ThreatScore?"}
        THREAT_CHECK -->|"> 0.7"| BLOCK["Block + audit log<br/>(policy_decisions table)"]
        THREAT_CHECK -->|"0.3–0.7"| SANITIZE["Sanitize + flag reduced authority"]
        THREAT_CHECK -->|"< 0.3"| PASS["Pass clean"]
    end

    SANITIZE & PASS --> CACHE_CHECK

    subgraph CachePhase["② Cache Lookup (in-memory HashMap)"]
        CACHE_CHECK["ironclad-llm/cache.rs · 3-Level"]
        CACHE_CHECK --> CACHE_L1{"L1: Exact hash?"}
        CACHE_L1 -->|hit| CACHE_HIT["Cache hit"]
        CACHE_L1 -->|miss| CACHE_L3{"L3: Tool TTL?"}
        CACHE_L3 -->|hit| CACHE_HIT
        CACHE_L3 -->|miss| CACHE_L2{"L2: Semantic n-gram cosine?"}
        CACHE_L2 -->|hit| CACHE_HIT
        CACHE_L2 -->|miss| CACHE_MISS["Cache miss"]
    end

    CACHE_HIT --> DELIVER_CACHED["ironclad-channels · Deliver cached response to User"]
    CACHE_MISS --> CONTEXT_BUILD

    subgraph ContextPhase["③ Context & Prompt Assembly"]
        CONTEXT_BUILD["ironclad-agent/context.rs"]
        CONTEXT_BUILD --> HEURISTIC["ironclad-llm/router.rs<br/>Heuristic classify_complexity"]
        HEURISTIC --> EMBED_QUERY["ironclad-llm/embedding.rs<br/>EmbeddingClient · embed_single(query)<br/>(external provider or n-gram fallback)"]
        EMBED_QUERY --> MEMORY_RETRIEVE["ironclad-agent/retrieval.rs<br/>MemoryRetriever · 5-tier retrieval<br/>(hybrid_search: FTS5 + vector cosine)"]
        MEMORY_RETRIEVE --> LOAD_HISTORY["ironclad-db/sessions.rs<br/>list_messages(session_id, 50)"]
        LOAD_HISTORY --> BUILD_CTX["ironclad-agent/context.rs<br/>build_context(system + memories + history)"]
        BUILD_CTX --> PROMPT_BUILD["ironclad-agent/prompt.rs<br/>Build system prompt + HMAC L2 boundaries"]
    end

    PROMPT_BUILD --> MODEL_SELECT

    subgraph LlmPipeline["④ LLM Inference Pipeline"]
        MODEL_SELECT["ironclad-llm/router.rs<br/>select_for_complexity"]
        MODEL_SELECT --> CIRCUIT{"Circuit Breaker<br/>blocked?"}
        CIRCUIT -->|"blocked → advance fallback"| MODEL_SELECT
        CIRCUIT -->|open| DEDUP{"In-flight<br/>duplicate?"}
        DEDUP -->|duplicate| DEDUP_REJECT["429 reject"]
        DEDUP -->|unique| FORMAT_XLATE["ironclad-llm/format.rs<br/>ApiFormat translation"]
        FORMAT_XLATE --> TIER_ADAPT["ironclad-llm/tier.rs<br/>T1: condense · T2: reorder<br/>T3/T4: passthrough + cache_control"]
        TIER_ADAPT --> FORWARD["ironclad-llm/client.rs<br/>HTTP/2 forward (reqwest pool)"]
        FORWARD --> UPSTREAM["LLM Provider<br/>(Anthropic / OpenAI / Ollama / Google Gemini /<br/>OpenRouter / DeepSeek / Groq / Moonshot /<br/>SGLang / vLLM / Docker Model Runner / llama-cpp)"]
    end

    UPSTREAM --> RESP

    subgraph ResponsePhase["⑤ Response Processing"]
        RESP["Response received"]
        RESP --> BREAKER_UPDATE["Update circuit breaker"]
        RESP --> COST_TRACK["Record inference_costs<br/>(model, provider, tokens, cost, tier)"]
        RESP --> RESP_XLATE["Format back-translation"]
        RESP_XLATE --> TOOL_EXEC{"Tool calls<br/>requested?"}
        TOOL_EXEC -->|yes| POLICY_EVAL["Policy engine (6 rules)"]
        POLICY_EVAL --> EXEC_TOOLS["Execute allowed tools"]
        TOOL_EXEC -->|no| PERSIST
        EXEC_TOOLS --> PERSIST
    end

    subgraph DeliverPhase["⑥ Persist & Deliver"]
        PERSIST["ironclad-db · Atomic SQLite transaction:<br/>session_messages, turn, tool_calls,<br/>policy_decisions"]
        PERSIST --> INJECTION_L4["L4 scan_output:<br/>NFKC, decode, homoglyph, regex"]
        INJECTION_L4 --> MEMORY_INGEST["ingest_turn → classify → store<br/>(background tokio::spawn)"]
        MEMORY_INGEST --> EMBED_RESP["ironclad-llm/embedding.rs<br/>Generate embedding for response<br/>→ store_embedding(BLOB)"]
        EMBED_RESP --> CACHE_STORE["Cache store (HashMap)<br/>+ periodic SQLite flush"]
        CACHE_STORE --> DELIVER["ironclad-channels · Deliver"]
        DELIVER --> USER_RESP["Response to User"]
    end
```

---

## 2. Semantic Cache Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Runtime cache in `ironclad-llm/cache.rs` (in-memory HashMap) with **SQLite persistence** via `ironclad-db/cache.rs`. On startup the server loads persisted entries from the `semantic_cache` table; a background task flushes in-memory entries to SQLite every 5 minutes and evicts expired rows.

```mermaid
flowchart TD
    subgraph Lookup["Cache Lookup (per-request)"]
        PROMPT_IN["Incoming prompt"]
        PROMPT_IN --> HASH["SemanticCache::compute_hash<br/>(system | messages | user_msg)"]
        HASH --> EXACT{"lookup_exact(prompt_hash)<br/>HashMap get, check expires_at"}
        EXACT -->|hit| HIT_EXACT["Return CachedResponse<br/>hits++"]
        EXACT -->|miss| L3{"lookup_tool_ttl(prompt_hash)"}
        L3 -->|hit| HIT_TOOL["Return cached (shorter TTL if tools)"]
        L3 -->|miss| EMBED["lookup_semantic(prompt)<br/>real embedding (if provider configured)<br/>or char n-gram fallback, cosine"]
        EMBED --> ANN{"cosine >= similarity_threshold (0.85)"}
        ANN -->|hit| HIT_SEMANTIC["Return best match"]
        ANN -->|miss| MISS["Cache miss -> LLM pipeline"]
    end

    subgraph Store["Cache Store (post-response)"]
        RESP_OK["Successful LLM response"]
        RESP_OK --> STORE_ENTRY["store() or store_with_embedding()<br/>entries.insert(prompt_hash, entry)<br/>evict_lfu() if len >= max_entries"]
    end

    subgraph Persistence["SQLite Persistence (ironclad-db/cache.rs)"]
        BOOT_LOAD["Server startup:<br/>load_cache_entries() → import_entries()"]
        FLUSH["Background task (5 min interval):<br/>export_entries() → save_cache_entry()<br/>evict_expired_cache()"]
    end

    subgraph Eviction["Eviction"]
        EVICT_EXPIRE["evict_expired(): retain where expires_at > now()"]
        EVICT_LFU["evict_lfu(): remove min hits when at capacity"]
        DB_EVICT["evict_expired_cache():<br/>DELETE FROM semantic_cache<br/>WHERE expires_at < now()"]
    end
```

---

## 3. Metascore Model Router Dataflow
<!-- last_updated: 2026-03-01, version: 0.9.1 -->

Routing hot path in `ironclad-server/api/routes/agent/routing.rs::select_routed_model_with_audit()`. Feature extraction and complexity classification in `ironclad-llm/router.rs`. Model profiles and metascore in `ironclad-llm/profile.rs`. Tiered inference (confidence evaluation + cloud escalation) in `ironclad-llm/tiered.rs`.

```mermaid
flowchart TD
    QUERY["Incoming query<br/>(post context assembly)"]
    QUERY --> MODE{"models.routing.mode?"}

    MODE -->|"primary"| DIRECT["Use primary model<br/>(skip scoring)"]

    MODE -->|"metascore / heuristic(alias)"| FEATURES["extract_features():<br/>message len, tool_call count, depth"]
    FEATURES --> CLASSIFY["classify_complexity()<br/>weighted sum → score 0.0–1.0"]

    subgraph MetascoreRouting["Metascore Model Selection (v0.9.1)"]
        CLASSIFY --> PROFILES["build_model_profiles():<br/>primary + fallbacks ×<br/>(provider, quality, capacity, breakers)"]
        PROFILES --> SCORE["metascore(model, complexity, cost_aware):<br/>efficacy × cost × availability × locality<br/>× confidence penalty"]
        SCORE --> SELECT["select_by_metascore():<br/>filter blocked → rank → best candidate"]
    end

    subgraph TieredInference["Tiered Inference (v0.9.1)"]
        DIRECT & SELECT --> INFER["infer_with_fallback()"]
        INFER --> CONFIDENCE{"ConfidenceEvaluator:<br/>token prob + length +<br/>uncertainty signals"}
        CONFIDENCE -->|"above floor"| RECORD
        CONFIDENCE -->|"below floor<br/>(local model)"| ESCALATE["EscalationTracker::record()<br/>→ cloud model fallback"]
        ESCALATE --> INFER
    end

    RECORD["Record inference_costs +<br/>QualityTracker::record() +<br/>ModelSelectionAudit with<br/>metascore breakdown"]
```

**Metascore dimensions** (weights: normal / cost-aware):

| Dimension | Normal | Cost-Aware | Source |
|-----------|--------|------------|--------|
| Efficacy | 0.45 | 0.35 | `QualityTracker` estimated quality (EMA) |
| Cost | 0.15 | 0.30 | Sigmoid-normalized inverse of per-token cost |
| Availability | 0.30 | 0.25 | Circuit breaker health × capacity headroom |
| Locality | 0.10 | 0.10 | Local bonus for simple tasks, cloud for complex |

Cold-start confidence penalty: linear ramp 0.6→1.0 over first 10 observations.

---

## 4. Memory Lifecycle Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

5-tier memory system unified in a single SQLite DB. Ingestion in `ironclad-agent/memory.rs`, storage in `ironclad-db/memory.rs`. The hippocampus schema map (`ironclad-db/schema_map.rs`) provides runtime introspection of memory table structures for dynamic query construction and context-aware retrieval.

```mermaid
flowchart TD
    subgraph Ingestion["Post-Turn Ingestion (ironclad-agent/memory.rs)"]
        TURN_DONE["Turn completed<br/>(thinking + tool results available)"]
        TURN_DONE --> CLASSIFY["Classify turn type<br/>(reasoning, tool_use, creative,<br/>financial, social, maintenance)"]

        CLASSIFY --> EX_WORKING["Update working_memory<br/>- Current goals<br/>- Active observations<br/>- In-progress plans<br/>- Reflections"]
        CLASSIFY --> EX_EPISODIC["Extract episodic events<br/>- Significant tool calls<br/>- Decision points<br/>- Outcomes and results"]
        CLASSIFY --> EX_SEMANTIC["Extract semantic facts<br/>- Learned information<br/>- Environmental changes<br/>- Financial state updates"]
        CLASSIFY --> EX_PROCEDURAL["Track procedural outcomes<br/>- Procedure name<br/>- success_count++ or failure_count++<br/>- Step adjustments"]
        CLASSIFY --> EX_RELATIONSHIP["Update relationships<br/>- trust_score adjustments<br/>- interaction_count++<br/>- last_interaction timestamp"]
    end

    subgraph Store["Atomic SQLite Transaction (ironclad-db/memory.rs)"]
        EX_WORKING --> T_WORKING["INSERT/UPDATE working_memory<br/>(session_id, entry_type,<br/>content, importance)"]
        EX_EPISODIC --> T_EPISODIC["INSERT episodic_memory<br/>(classification, content,<br/>importance)"]
        EX_SEMANTIC --> T_SEMANTIC["INSERT OR REPLACE semantic_memory<br/>(category, key, value,<br/>confidence)"]
        EX_PROCEDURAL --> T_PROCEDURAL["INSERT OR REPLACE procedural_memory<br/>(name, steps, success_count,<br/>failure_count)"]
        EX_RELATIONSHIP --> T_RELATIONSHIP["INSERT OR REPLACE relationship_memory<br/>(entity_id, entity_name,<br/>trust_score, interaction_summary,<br/>interaction_count, last_interaction)"]

        T_EPISODIC --> FTS_SYNC["Sync memory_fts<br/>(FTS5 virtual table)"]
    end

    subgraph EmbeddingGen["Embedding Generation (post-ingestion)"]
        EX_EPISODIC --> GEN_EMBED["ironclad-llm/embedding.rs<br/>EmbeddingClient · embed_single(content)<br/>(external provider or n-gram fallback)"]
        GEN_EMBED --> CHUNK_CHECK{"Content > 512 tokens?"}
        CHUNK_CHECK -->|yes| CHUNK["ironclad-agent/retrieval.rs<br/>chunk_text() with overlap<br/>(default 512 tok, 64 overlap)"]
        CHUNK -->|each chunk| STORE_EMBED["ironclad-db/embeddings.rs<br/>store_embedding()<br/>(BLOB format, ~4x smaller than JSON)"]
        CHUNK_CHECK -->|no| STORE_EMBED
    end

    subgraph Retrieval["Pre-Inference Retrieval (ironclad-agent/retrieval.rs)"]
        INFER_START["Inference call starting"]
        INFER_START --> SCHEMA_MAP["ironclad-db/schema_map.rs<br/>Hippocampus schema introspection<br/>(table structure, column types,<br/>index availability)"]
        SCHEMA_MAP --> EMBED_QUERY_R["ironclad-llm/embedding.rs<br/>Generate query embedding"]
        EMBED_QUERY_R --> BUDGET["MemoryRetriever<br/>MemoryBudgetManager allocates<br/>token budget per tier:<br/>working: 30% · episodic: 25%<br/>semantic: 20% · procedural: 15%<br/>relationship: 10%<br/>(unused budget rolls over)"]

        BUDGET --> R_WORK["Retrieve working_memory<br/>(all entries for current session_id)"]
        BUDGET --> R_HYBRID["hybrid_search(query, embedding)<br/>FTS5 keyword + vector cosine<br/>(configurable hybrid_weight)"]
        BUDGET --> R_PROC["Retrieve procedural_memory<br/>(tool success/failure rates)"]
        BUDGET --> R_REL["Retrieve relationship_memory<br/>(active entities from conversation)"]

        R_HYBRID --> ANN_CHECK{"ANN index built?<br/>(memory.ann_index = true<br/>& entries > threshold)"}
        ANN_CHECK -->|yes| ANN_SEARCH["ironclad-db/ann.rs<br/>HNSW O(log n) search<br/>(instant-distance crate)"]
        ANN_CHECK -->|no| BRUTE["Brute-force cosine scan<br/>(O(n) over embeddings table)"]

        R_WORK & ANN_SEARCH & BRUTE & R_PROC & R_REL --> FORMAT_BLOCK["Format memory block<br/>(structured text within<br/>total token budget)"]
        FORMAT_BLOCK --> INJECT["Inject into context<br/>(after system prompt,<br/>before conversation history)"]
    end

    subgraph Pruning["Background Pruning (heartbeat task)"]
        PRUNE_TICK["Heartbeat tick"]
        PRUNE_TICK --> PRUNE_WORKING["DELETE expired working_memory<br/>(sessions that are closed)"]
        PRUNE_TICK --> PRUNE_EPISODIC["DELETE lowest importance<br/>episodic_memory when count<br/>exceeds threshold"]
        PRUNE_TICK --> PRUNE_FTS["Rebuild memory_fts<br/>after bulk deletes"]
    end
```

---

## 4.1 Behavior Guard + Deterministic Shortcut Dataflow
<!-- last_updated: 2026-03-06, version: 0.9.5-prep -->

High-frequency operator prompts with deterministic intent (for example, filesystem counts and direct capability checks) now prefer the execution-shortcut path. Guardrails sanitize internal protocol metadata and force user-facing fallbacks when model output degrades.

```mermaid
flowchart TD
    U["User Prompt"] --> I["Intent Classifier (intents.rs)"]
    I --> B{"Bypass cache?"}
    B -->|yes| S{"Deterministic shortcut match?"}
    S -->|yes| T["Execute tool/runtime shortcut<br/>(core::try_execution_shortcut)"]
    T --> R["Verified result + tool evidence"]
    S -->|no| M["LLM inference path"]
    B -->|no| M
    M --> G1["Execution truth guard"]
    G1 --> G2["Personality + jargon guards"]
    G2 --> G3["Internal protocol guard<br/>(strip delegation/tool metadata)"]
    G3 --> F{"Content empty/degraded?"}
    F -->|yes| D["Deterministic quality fallback"]
    F -->|no| R
    D --> R
    R --> O["Channel formatter + delivery"]
```

---

## 5. Zero-Trust Agent-to-Agent Communication Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

New subsystem. Identity via `ironclad-wallet/wallet.rs`, protocol in `ironclad-channels/a2a.rs`, trust data in `relationship_memory` table.

```mermaid
flowchart TD
    subgraph Discovery["Agent Discovery"]
        NEED_PEER["Need to contact peer agent"]
        NEED_PEER --> CACHE_CARD{"Agent card in<br/>discovered_agents table?<br/>(TTL-based cache)"}
        CACHE_CARD -->|hit| USE_CARD["Use cached agent card"]
        CACHE_CARD -->|miss| ERC8004["Query ERC-8004 registry<br/>on Base (via alloy-rs)<br/>wallet.chain_id = 8453<br/>wallet.rpc_url"]
        ERC8004 --> PARSE_CARD["Parse JSON-LD Agent Card<br/>(capabilities, endpoints, services)"]
        PARSE_CARD --> STORE_CARD["Cache in discovered_agents table"]
        STORE_CARD --> USE_CARD
    end

    subgraph MutualAuth["Mutual Authentication (challenge-response)"]
        USE_CARD --> HELLO["Agent A: POST /a2a/hello<br/>body: DID_A + nonce_A (32 bytes)<br/>+ timestamp_A<br/>+ signature_A(nonce_A || timestamp_A)"]

        HELLO --> B_VERIFY["Agent B: Verify signature_A<br/>against A's on-chain public key<br/>(ERC-8004 registry lookup)"]
        B_VERIFY --> B_FRESH{"timestamp_A<br/>within 60s?"}
        B_FRESH -->|no| B_REJECT["Reject: stale timestamp"]
        B_FRESH -->|yes| B_RESPOND["Agent B responds:<br/>DID_B + nonce_B<br/>+ signature_B(nonce_A || nonce_B || timestamp_B)"]

        B_RESPOND --> A_VERIFY["Agent A: Verify signature_B<br/>against B's on-chain public key"]
        A_VERIFY --> A_FRESH{"timestamp_B<br/>within 60s?"}
        A_FRESH -->|no| A_REJECT["Reject: stale timestamp"]
        A_FRESH -->|yes| DERIVE_KEY["Both sides: derive session key<br/>via ECDH (ephemeral keypairs)<br/>-> AES-256-GCM for forward secrecy"]
    end

    subgraph Messaging["Encrypted Message Exchange"]
        DERIVE_KEY --> SESSION_READY["Authenticated session established"]
        SESSION_READY --> SEND_MSG["Send message:<br/>AES-256-GCM encrypted payload<br/>+ per-message nonce<br/>+ HMAC authentication tag"]
        SEND_MSG --> RECV_VALIDATE["Receiver validates:<br/>1. Decrypt with session key<br/>2. Verify HMAC tag<br/>3. Check message size < a2a.max_message_size (64KB)<br/>4. Rate limit check < a2a.rate_limit_per_peer (10/min)"]
        RECV_VALIDATE --> INJECTION_SCREEN["Pass content through<br/>injection defense pipeline<br/>(Layer 1 gatekeeping)<br/>with source = peer_agent"]
        INJECTION_SCREEN --> TRUST_WRAP["Wrap in trust boundary:<br/>peer_agent_input trust_level=X<br/>(from relationship_memory.trust_score)"]
    end

    subgraph TrustUpdate["Trust Score Management"]
        TRUST_WRAP --> PROCESS["Process peer message<br/>(with reduced authority)"]
        PROCESS --> OUTCOME{"Interaction<br/>outcome?"}
        OUTCOME -->|positive| TRUST_UP["UPDATE relationship_memory<br/>SET trust_score = min(1.0, trust_score + 0.05),<br/>interaction_count = interaction_count + 1,<br/>last_interaction = now()"]
        OUTCOME -->|negative| TRUST_DOWN["UPDATE relationship_memory<br/>SET trust_score = max(0.0, trust_score - 0.1),<br/>interaction_count = interaction_count + 1,<br/>last_interaction = now()"]
        OUTCOME -->|neutral| TRUST_SAME["UPDATE relationship_memory<br/>SET interaction_count = interaction_count + 1,<br/>last_interaction = now()"]
    end

    subgraph Opacity["Opacity Principle"]
        OPACITY_NOTE["Agents NEVER expose to peers:<br/>- Internal memory contents<br/>- Tool state or outputs<br/>- Prompt content or system prompt<br/>- Wallet private key<br/>- Session history<br/>Only structured A2A task messages"]
    end
```

---

## 6. Multi-Layer Prompt Injection Defense Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

4-layer defense system spanning `ironclad-agent/injection.rs`, `ironclad-agent/prompt.rs`, and `ironclad-agent/policy.rs`.

```mermaid
flowchart TD
    subgraph L1["Layer 1: Input Gatekeeping (injection.rs)"]
        INPUT["Raw input text<br/>(user, peer, or tool output)"]
        INPUT --> CHECKS["Parallel checks:<br/>· Regex (instruction override, ChatML, authority)<br/>· Encoding evasion (base64, homoglyphs, zero-width)<br/>· Financial manipulation (transfer, policy override)<br/>· Multi-language (CJK, Cyrillic, Arabic)"]
        CHECKS --> SCORE["Aggregate ThreatScore 0.0–1.0"]
        SCORE --> DECISION{"Threshold?"}
        DECISION -->|"> 0.7"| BLOCK_INPUT["BLOCK · log to policy_decisions"]
        DECISION -->|"0.3–0.7"| SANITIZE_INPUT["SANITIZE · reduced_authority"]
        DECISION -->|"< 0.3"| CLEAN["PASS"]
    end

    CLEAN & SANITIZE_INPUT --> BUILD_PROMPT

    subgraph L2["Layer 2: Structured Prompt Formatting (prompt.rs)"]
        BUILD_PROMPT["Build prompt with trust boundaries"]
        BUILD_PROMPT --> WRAP["Wrap each section:<br/>· trusted_system + HMAC tag<br/>· user_input<br/>· tool_output<br/>· peer_agent_input trust_level=X"]
        WRAP --> ASSEMBLED["Assembled HMAC-tagged prompt<br/>(unforgeable by injected content)"]
    end

    subgraph L3["Layer 3: Output Validation (policy.rs)"]
        LLM_RESP["LLM response with tool calls"]

        subgraph AuthGate["Authority Gate (per tool call)"]
            AUTH_CHECK{"Input source?"}
            AUTH_CHECK -->|creator| FULL_AUTH["All risk levels"]
            AUTH_CHECK -->|self| SELF_AUTH["Safe + Caution + Dangerous"]
            AUTH_CHECK -->|peer| PEER_AUTH["Safe + Caution only"]
            AUTH_CHECK -->|external| EXT_AUTH["Safe only"]
        end

        subgraph PeerFinancial["Peer Financial Guard"]
            PEER_AUTH --> FIN_CHECK{"Financial call?"}
            FIN_CHECK -->|yes| STRICTER["Stricter limits<br/>(cap/10, hourly/5)"]
            FIN_CHECK -->|no| ALLOW_PEER["Allow"]
        end

        subgraph SelfModGuard["Self-Modification Guard"]
            SELFMOD_CHECK{"Self-mod tool?"}
            SELFMOD_CHECK -->|no| SKIP_MOD["N/A"]
            SELFMOD_CHECK -->|yes| CREATOR_ONLY{"Source = creator?"}
            CREATOR_ONLY -->|yes| ALLOW_MOD["Allow"]
            CREATOR_ONLY -->|no| DENY_MOD["DENY"]
        end

        LLM_RESP --> AUTH_CHECK
        LLM_RESP --> SELFMOD_CHECK
        FULL_AUTH & SELF_AUTH & ALLOW_PEER & STRICTER & ALLOW_MOD --> EXECUTE["Execute tool call"]
    end

    subgraph L4["Layer 4: Adaptive Refinement"]
        RESPONSE["Final response text"]
        RESPONSE --> SCAN_OUTPUT["Scan output for<br/>injection patterns"]
        SCAN_OUTPUT --> OUTPUT_CLEAN{"Clean?"}
        OUTPUT_CLEAN -->|no| STRIP["Strip + alert"]
        OUTPUT_CLEAN -->|yes| DELIVER_FINAL["Deliver"]
        RESPONSE --> ANOMALY_CHECK["Behavioral anomaly check:<br/>tool pattern changes,<br/>protected file access,<br/>repeated financial ops"]
        ANOMALY_CHECK --> ANOMALY_FOUND{"Anomaly?"}
        ANOMALY_FOUND -->|yes| ALERT["Alert via metric_snapshots"]
        ANOMALY_FOUND -->|no| DELIVER_FINAL
    end
```

---

## 7. Financial + Yield Engine Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

x402 credit purchases and Aave/Compound yield generation. Core logic in `ironclad-wallet/`.

```mermaid
flowchart TD
    subgraph Monitoring["① Survival Monitoring (heartbeat task)"]
        HB_TICK["Heartbeat tick<br/>(ironclad-schedule/heartbeat.rs)"]
        HB_TICK --> FETCH_CREDITS["Fetch credit balance"]
        HB_TICK --> FETCH_USDC["Fetch USDC balance<br/>(Base RPC via alloy-rs)"]
        FETCH_CREDITS --> CALC_TIER["Calculate SurvivalTier"]
        CALC_TIER --> TIER_BRANCH{"SurvivalTier?"}
        TIER_BRANCH -->|"High / Normal"| NORMAL["Normal operation"]
        TIER_BRANCH -->|LowCompute| LOW["Downgrade to T1/T2 models"]
        TIER_BRANCH -->|Critical| CRIT["Distress signals, accept funding only"]
        TIER_BRANCH -->|Dead| DEAD_STATE["Wait for funding"]
    end

    FETCH_USDC --> HAS_USDC
    CALC_TIER --> YIELD_CHECK

    subgraph Topup["② x402 Credit Topup (ironclad-wallet/x402.rs)"]
        HAS_USDC{"USDC > 0?"}
        HAS_USDC -->|no| NO_TOPUP["No action"]
        HAS_USDC -->|yes| WAKE_AGENT["Signal agent wake (mpsc)"]
        WAKE_AGENT --> TOPUP_TOOL["topup_credits tool<br/>(select tier: $5–$2500)"]
        TOPUP_TOOL --> X402_REQ["POST to credits endpoint"]
        X402_REQ --> X402_402["HTTP 402 + payment requirements"]
        X402_402 --> SIGN["Sign TransferWithAuthorization<br/>(EIP-3009, alloy-rs)"]
        SIGN --> RETRY["Retry with X-Payment header"]
        RETRY --> CREDITS_OK["Credits added"]
        CREDITS_OK --> TX_LOG_TOPUP["INSERT transactions<br/>(topup, amount, tx_hash)"]
    end

    subgraph Yield["③ Yield Engine (ironclad-wallet/yield_engine.rs)"]
        YIELD_CHECK{"yield.enabled?"}
        YIELD_CHECK -->|no| SKIP_YIELD["Skip"]
        YIELD_CHECK -->|yes| CALC_EXCESS["excess = balance −<br/>minimum_reserve − buffer"]
        CALC_EXCESS --> DEPOSIT_CHECK{"excess > min_deposit?"}
        DEPOSIT_CHECK -->|yes| AAVE_DEPOSIT["Aave deposit on Base"]
        AAVE_DEPOSIT --> TX_LOG_DEP["INSERT transactions<br/>(yield_deposit)"]
        DEPOSIT_CHECK -->|no| WITHDRAW_CHECK{"balance <<br/>withdrawal_threshold?"}
        WITHDRAW_CHECK -->|yes| AAVE_WITHDRAW["Aave withdraw<br/>(restore to min_reserve)"]
        AAVE_WITHDRAW --> TX_LOG_WD["INSERT transactions<br/>(yield_withdraw)"]
        WITHDRAW_CHECK -->|no| YIELD_DONE["No yield action"]
    end

    subgraph YieldTracking["④ Yield Earnings (periodic)"]
        HB_TICK2["Heartbeat tick"] --> YIELD_TRACK["Check aToken balance delta<br/>if delta > 0: INSERT transactions<br/>(yield_earned, delta)"]
    end

    subgraph SpendControl["⑤ Spending Controls (ironclad-wallet/treasury.rs)"]
        FIN_TOOL["Financial tool call<br/>(transfer_credits, x402_fetch,<br/>topup_credits, spawn_child)"]
        FIN_TOOL --> POLICY_ENGINE["Policy engine evaluation"]
        POLICY_ENGINE --> TREASURY_CHECK["TreasuryPolicy checks:<br/>per_payment_cap · hourly_limit<br/>daily_limit · min_reserve<br/>daily_inference_budget"]
        TREASURY_CHECK --> SPEND_QUERY["Query transactions<br/>(hourly + daily aggregates)"]
        SPEND_QUERY --> ALLOWED{"Within limits?"}
        ALLOWED -->|yes| EXEC_FIN["Execute transaction"]
        ALLOWED -->|no| DENY_FIN["Deny + log to<br/>policy_decisions"]
    end
```

---

## 8. Cron + Heartbeat Unified Scheduling Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Unified scheduling system in `ironclad-schedule/`.

```mermaid
flowchart TD
    subgraph TickLoop["① Tick Loop (heartbeat.rs)"]
        TOKIO_INTERVAL["tokio::time::interval<br/>(default 60s, select! no overlap)"]
        TOKIO_INTERVAL --> BUILD_CTX["Build TickContext:<br/>credit balance, USDC balance,<br/>SurvivalTier, timestamp"]
    end

    BUILD_CTX --> QUERY_JOBS

    subgraph Evaluate["② Job Evaluation (scheduler.rs)"]
        QUERY_JOBS["SELECT cron_jobs WHERE enabled = 1"]
        QUERY_JOBS --> FOR_EACH["For each job"]
        FOR_EACH --> SCHED_TYPE{"schedule_kind?"}
        SCHED_TYPE -->|cron| CRON_EVAL["Evaluate cron expression"]
        SCHED_TYPE -->|every| INTERVAL_EVAL["Elapsed ≥ schedule_every_ms?"]
        SCHED_TYPE -->|at| AT_EVAL["now() ≥ schedule_expr?"]
        CRON_EVAL & INTERVAL_EVAL & AT_EVAL --> IS_DUE{"Due?"}
        IS_DUE -->|no| SKIP_JOB["Skip"]
        IS_DUE -->|yes| LEASE{"Acquire DB lease?"}
        LEASE -->|contended| SKIP_JOB
        LEASE -->|acquired| EXECUTE_JOB["Execute job"]
    end

    EXECUTE_JOB --> PAYLOAD_KIND

    subgraph Execution["③ Job Execution (tasks.rs)"]
        PAYLOAD_KIND{"payload_json.kind?"}
        PAYLOAD_KIND -->|agentTurn| AGENT_NOOP["DEPRECATED: agent_turn_legacy<br/>(noop with warning log)"]
        PAYLOAD_KIND -->|systemEvent| SYS_EVENT["Process system event"]
    end

    subgraph PostExecution["④ Post-Execution"]
        direction LR
        subgraph Recording["State Recording"]
            UPDATE_JOB["UPDATE cron_jobs<br/>(last_run_at, status, duration,<br/>next_run_at, lease = NULL)"]
            UPDATE_JOB --> INSERT_RUN["INSERT cron_runs"]
        end
        subgraph Delivery["Result Delivery"]
            DELIVERY_MODE{"delivery_mode?"}
            DELIVERY_MODE -->|none| SILENT["Silent"]
            DELIVERY_MODE -->|announce| DELIVER_MSG["Send via channel adapter"]
        end
    end

    AGENT_NOOP & SYS_EVENT --> UPDATE_JOB
    SYS_EVENT --> DELIVERY_MODE

    subgraph WakeSignal["⑤ In-Process Wake Signal"]
        SHOULD_WAKE{"Task signals shouldWake?"}
        SHOULD_WAKE -->|no| WAKE_DONE["No wake"]
        SHOULD_WAKE -->|yes| MPSC_SEND["mpsc::send(WakeEvent)"]
        MPSC_SEND --> SLEEP_SELECT["Agent sleep loop:<br/>tokio::select! on<br/>mpsc::recv | 30s poll"]
        SLEEP_SELECT --> AGENT_WAKES["Agent loop resumes"]
    end

    SYS_EVENT --> SHOULD_WAKE
```

---

## 9. Skill Execution Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Dual-format extensibility system in `ironclad-agent/skills.rs` and `ironclad-agent/script_runner.rs`, with persistence in `ironclad-db/skills.rs`.

```mermaid
flowchart TD
    subgraph Loading["① Skill Loading (boot + hot-reload)"]
        SCAN["SkillLoader: scan skills_dir"]
        SCAN --> FIND_TOML["Find .toml (structured)"]
        SCAN --> FIND_MD["Find .md (instruction)"]
        FIND_TOML --> PARSE_TOML["Parse TOML manifest"]
        FIND_MD --> PARSE_MD["Parse YAML frontmatter + body"]
        PARSE_TOML & PARSE_MD --> HASH["SHA-256 content_hash"]
        HASH --> CHANGED{"Hash changed?"}
        CHANGED -->|yes| UPSERT["Upsert skills table"]
        CHANGED -->|no| SKIP["Skip (unchanged)"]
        UPSERT --> INDEX["Update SkillRegistry trigger index"]
    end

    subgraph Matching["② Trigger Matching (per-turn)"]
        TURN_CTX["Turn context<br/>(user message + active tools)"]
        TURN_CTX --> EVAL_TRIGGERS["SkillRegistry.match_skills():<br/>keyword, tool-name, regex"]
        EVAL_TRIGGERS --> MATCHED{"Matched?"}
        MATCHED -->|none| NO_SKILL["Continue without skills"]
        MATCHED -->|"1+"| SORT["Sort by priority"]
    end

    SORT -->|instruction| INJECT_BODY
    SORT -->|structured| CHAIN

    subgraph ExecInstruction["③a Instruction Skill"]
        INJECT_BODY["Inject .md body into<br/>system prompt (prompt.rs)"]
        INJECT_BODY --> LLM_HANDLES["LLM interprets instructions"]
    end

    subgraph ExecStructured["③b Structured Skill Execution"]
        CHAIN["StructuredSkillExecutor:<br/>iterate tool_chain steps"]
        CHAIN --> POLICY_OVERRIDE{"policy_overrides?"}
        POLICY_OVERRIDE -->|yes| APPLY_OVERRIDE["Apply overrides"]
        POLICY_OVERRIDE -->|no| EXEC_CHAIN["Execute tool chain"]
        APPLY_OVERRIDE --> EXEC_CHAIN
        EXEC_CHAIN --> HAS_SCRIPT{"script_path?"}
        HAS_SCRIPT -->|no| TOOL_EXEC["Execute via ToolRegistry"]
        HAS_SCRIPT -->|yes| POLICY_CHECK["Policy engine evaluates<br/>ScriptTool risk_level"]
    end

    subgraph ScriptPipeline["④ Script Execution Pipeline"]
        POLICY_CHECK --> ALLOWED{"Allowed?"}
        ALLOWED -->|no| DENY["Deny (audit log)"]
        ALLOWED -->|yes| INTERPRETER_CHECK{"Interpreter in<br/>whitelist?"}
        INTERPRETER_CHECK -->|no| REJECT_SCRIPT["Reject: unlisted"]
        INTERPRETER_CHECK -->|yes| SPAWN["Spawn process<br/>(skill parent dir)"]
        SPAWN --> ENV_CHECK{"sandbox_env?"}
        ENV_CHECK -->|yes| STRIP_ENV["Strip env → PATH, HOME,<br/>SESSION_ID, AGENT_ID only"]
        ENV_CHECK -->|no| FULL_ENV["Inherit full env"]
        STRIP_ENV & FULL_ENV --> TIMEOUT["Timeout enforcement"]
        TIMEOUT --> OUTPUT_CAP["Capture + truncate output"]
        OUTPUT_CAP --> RESULT["ScriptResult:<br/>stdout, stderr, exit_code"]
    end
```

---

## 10. Approval Workflow Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Tool gating with human-in-the-loop approval for dangerous operations. `ApprovalManager` pauses the agent loop until an admin resolves the request via the dashboard WebSocket or REST API.

```mermaid
flowchart TD
    subgraph GateCheck["① Tool Gate Check"]
        AGENT_CALL["AgentLoop<br/>Tool call requested"]
        AGENT_CALL --> GATE{"ApprovalManager<br/>requires_approval(tool, risk)?"}
        GATE -->|no| DIRECT_EXEC["Execute immediately"]
        GATE -->|yes| CREATE_REQ["ApprovalManager<br/>create_request(tool, args, context)"]
    end

    subgraph Notification["② Approval Notification"]
        CREATE_REQ --> PERSIST_REQ["INSERT approval_requests<br/>(tool, args, status=pending,<br/>requested_at)"]
        PERSIST_REQ --> EVENT["EventBus<br/>publish(ApprovalRequested)"]
        EVENT --> WS_PUSH["DashboardWS<br/>push to connected clients"]
        EVENT --> PAUSE["AgentLoop<br/>pause execution<br/>(tokio::sync::oneshot)"]
    end

    subgraph Resolution["③ Approve / Deny"]
        WS_PUSH --> ADMIN_UI["Dashboard UI<br/>approval card rendered"]
        ADMIN_UI --> ADMIN_ACT{"Admin action?"}
        ADMIN_ACT -->|approve| APPROVE_API["AdminAPI<br/>POST /api/approvals/:id/approve"]
        ADMIN_ACT -->|deny| DENY_API["AdminAPI<br/>POST /api/approvals/:id/deny"]
        APPROVE_API --> UPDATE_APPROVED["UPDATE approval_requests<br/>SET status=approved,<br/>resolved_by, resolved_at"]
        DENY_API --> UPDATE_DENIED["UPDATE approval_requests<br/>SET status=denied,<br/>resolved_by, resolved_at, reason"]
    end

    subgraph Resume["④ Agent Resume"]
        UPDATE_APPROVED --> SIGNAL_OK["EventBus<br/>publish(ApprovalResolved)"]
        UPDATE_DENIED --> SIGNAL_DENY["EventBus<br/>publish(ApprovalResolved)"]
        SIGNAL_OK --> RESUME_EXEC["AgentLoop resumes<br/>→ execute tool"]
        SIGNAL_DENY --> RESUME_SKIP["AgentLoop resumes<br/>→ skip tool, report denial"]
    end
```

---

## 11. Browser Tool Execution Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

CDP-based browser automation via `BrowserTool`. The `BrowserManager` maintains a pool of Chrome sessions with idle eviction.

```mermaid
flowchart TD
    subgraph Dispatch["① Tool Dispatch"]
        AGENT["AgentLoop<br/>tool_call: browser_action"]
        AGENT --> REGISTRY["ToolRegistry<br/>lookup('browser_action')"]
        REGISTRY --> BROWSER_TOOL["BrowserTool<br/>validate args (url, action, selector)"]
    end

    subgraph SessionMgmt["② CDP Session Management"]
        BROWSER_TOOL --> MGR_CHECK{"BrowserManager<br/>active session?"}
        MGR_CHECK -->|yes| REUSE["Reuse existing CdpSession"]
        MGR_CHECK -->|no| LAUNCH["BrowserManager<br/>launch Chrome<br/>(--headless, --remote-debugging-port)"]
        LAUNCH --> CDP_CONNECT["CdpSession<br/>WebSocket connect to CDP endpoint"]
        CDP_CONNECT --> REUSE
    end

    subgraph Execution["③ Action Execution"]
        REUSE --> ACTION_TYPE{"action type?"}
        ACTION_TYPE -->|navigate| NAV["CdpSession<br/>Page.navigate(url)<br/>wait for load event"]
        ACTION_TYPE -->|click| CLICK["CdpSession<br/>DOM.querySelector(selector)<br/>Input.dispatchMouseEvent"]
        ACTION_TYPE -->|extract| EXTRACT["CdpSession<br/>Runtime.evaluate(js)<br/>extract page content"]
        ACTION_TYPE -->|screenshot| SCREENSHOT["CdpSession<br/>Page.captureScreenshot<br/>→ base64 PNG"]
    end

    subgraph Result["④ Result Processing"]
        NAV & CLICK & EXTRACT & SCREENSHOT --> RESULT["BrowserTool<br/>format result<br/>(content, screenshot, timing)"]
        RESULT --> TIMEOUT_CHECK{"Exceeded<br/>timeout?"}
        TIMEOUT_CHECK -->|yes| CLEANUP["BrowserManager<br/>kill session, report timeout"]
        TIMEOUT_CHECK -->|no| RETURN["Return to AgentLoop"]
    end

    subgraph Lifecycle["⑤ Session Lifecycle"]
        IDLE_CHECK["BrowserManager<br/>idle timeout monitor"]
        IDLE_CHECK -->|"idle > 5 min"| TEARDOWN["Close CdpSession<br/>kill Chrome process"]
        IDLE_CHECK -->|"max sessions"| EVICT_LRU["Evict LRU session"]
    end
```

---

## 12. Context Assembly & Snapshot Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Complexity-driven context assembly with progressive trimming. `build_context()` allocates token budgets per tier using `MemoryBudgetManager` and persists snapshots via direct `context_snapshots` table inserts.

```mermaid
flowchart TD
    subgraph Classify["① Complexity Classification"]
        INPUT["Incoming turn<br/>(message + tool context)"]
        INPUT --> CLASSIFIER["classify_complexity()<br/>extract_features():<br/>msg length, tool count,<br/>conversation depth, topic shifts"]
        CLASSIFIER --> SCORE["Complexity score 0.0–1.0"]
    end

    subgraph Budget["② Token Budget Allocation"]
        SCORE --> BUDGET_MGR["MemoryBudgetManager<br/>allocate(complexity, model_limit)"]
        BUDGET_MGR --> TIERS["Per-tier allocation:<br/>system: 15% · memories: 30%<br/>history: 40% · tools: 15%<br/>(adjusted by complexity)"]
    end

    subgraph Retrieve["③ Tiered Retrieval"]
        TIERS --> MEM_RETRIEVE["MemoryRetriever<br/>retrieve_within_budget()"]
        MEM_RETRIEVE --> T_WORKING["Working memory<br/>(session-scoped, all entries)"]
        MEM_RETRIEVE --> T_EPISODIC["Episodic memory<br/>(hybrid search, ranked)"]
        MEM_RETRIEVE --> T_SEMANTIC["Semantic memory<br/>(key-value facts)"]
        MEM_RETRIEVE --> T_PROCEDURAL["Procedural memory<br/>(tool success rates)"]
        MEM_RETRIEVE --> T_RELATIONSHIP["Relationship memory<br/>(entity context)"]
    end

    subgraph Assemble["④ Context Assembly"]
        T_WORKING & T_EPISODIC & T_SEMANTIC & T_PROCEDURAL & T_RELATIONSHIP --> BUILDER["build_context()<br/>assemble(system, memories,<br/>history, tool_defs)"]
        BUILDER --> TRIM{"Within<br/>model limit?"}
        TRIM -->|no| PROGRESSIVE["Progressive trim:<br/>reduce history → reduce memories<br/>→ condense system prompt"]
        PROGRESSIVE --> BUILDER
        TRIM -->|yes| ASSEMBLED["Assembled context<br/>(L0: system, L1: memories,<br/>L2: history, L3: tools)"]
    end

    subgraph Snapshot["⑤ Snapshot Persistence"]
        ASSEMBLED --> SNAP["Direct INSERT context_snapshots<br/>(turn_id, tier_sizes,<br/>total_tokens, complexity,<br/>trimmed_flag)"]
        SNAP --> METRICS["Context efficiency metrics<br/>(utilization %, waste,<br/>trim frequency)"]
    end
```

---

## 13. Response Transform Pipeline Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

> **Note (v0.8.0)**: The 4-stage pipeline described below (`ReasoningExtractor`, `FormatNormalizer`, `ContentGuard`, PII scan) exists only in the dead-code file `ironclad-llm/src/transform.rs`, which is NOT declared as `pub mod` in `lib.rs` and is unreachable from other crates. Actual response processing in v0.8.0 is performed inline in `agent.rs`. This diagram is retained for reference but does **not** describe current runtime behavior.

Post-LLM response processing chain (unimplemented -- see note above): reasoning extraction, format normalization, and content guarding before the response reaches the agent loop.

```mermaid
flowchart TD
    subgraph Receive["① Raw Response"]
        RAW["LlmResponse<br/>(raw provider response)"]
        RAW --> PIPELINE["Pipeline<br/>init transform chain"]
    end

    subgraph Extract["② Reasoning Extraction"]
        PIPELINE --> REASONING["ReasoningExtractor"]
        REASONING --> HAS_THINKING{"Contains<br/>thinking blocks?"}
        HAS_THINKING -->|yes| SPLIT["Split thinking from content<br/>store reasoning separately"]
        HAS_THINKING -->|no| PASS_THROUGH["Pass through unchanged"]
        SPLIT --> STORE_REASON["Persist reasoning<br/>(turns.thinking_content)"]
    end

    subgraph Normalize["③ Format Normalization"]
        STORE_REASON & PASS_THROUGH --> NORMALIZER["FormatNormalizer"]
        NORMALIZER --> STRIP_PROVIDER["Strip provider-specific<br/>metadata and wrappers"]
        STRIP_PROVIDER --> UNIFY_TOOL["Unify tool_call format<br/>(provider → internal ToolCall)"]
        UNIFY_TOOL --> FIX_ENCODING["Fix encoding issues<br/>(NFKC normalize, trim)"]
    end

    subgraph Guard["④ Content Guard"]
        FIX_ENCODING --> GUARD["ContentGuard"]
        GUARD --> PII_CHECK["PII leak scan<br/>(API keys, tokens, secrets)"]
        PII_CHECK --> INJECT_CHECK["Injection echo check<br/>(LLM parroting attack strings)"]
        INJECT_CHECK --> SIZE_CHECK{"Response<br/>size OK?"}
        SIZE_CHECK -->|"exceeds max"| TRUNCATE["Truncate with indicator"]
        SIZE_CHECK -->|ok| CLEAN["Clean output"]
        TRUNCATE --> CLEAN
    end

    CLEAN --> OUTPUT["Cleaned response<br/>→ AgentLoop"]
```

---

## 14. Streaming LLM Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Token-by-token streaming from LLM provider through SSE to the dashboard. The `StreamAccumulator` buffers deltas while the `EventBus` publishes chunks to WebSocket subscribers in real time. Note: v0.8.0 streaming performs in-flight deduplication (`dedup.rs`) before provider dispatch (same as non-stream inference).

```mermaid
flowchart TD
    subgraph Init["① Stream Initialization"]
        REQ["Request<br/>(stream: true)"]
        REQ --> DEDUP_STREAM["In-flight dedup<br/>(dedup.rs: check_and_track)"]
        DEDUP_STREAM --> CLIENT["LlmClient<br/>forward_stream(provider, payload)"]
        CLIENT --> HTTP2["HTTP/2 POST<br/>(Accept: text/event-stream)"]
        HTTP2 --> PROVIDER["LLM Provider<br/>begin SSE response"]
    end

    subgraph Chunks["② SSE Chunk Processing"]
        PROVIDER --> SSE["SseStream<br/>parse event-stream"]
        SSE --> CHUNK_LOOP["For each SSE chunk"]
        CHUNK_LOOP --> PARSE["Parse chunk delta<br/>(content, tool_call, usage)"]
        PARSE --> ACCUM["Accumulator<br/>append delta to buffer<br/>track token count"]
    end

    subgraph Publish["③ Real-Time Publish"]
        ACCUM --> BUS["EventBus<br/>publish(StreamChunk)"]
        BUS --> WS_CLIENTS["DashboardWS<br/>broadcast to subscribers"]
        WS_CLIENTS --> RENDER["Dashboard<br/>render token-by-token"]
    end

    subgraph Finalize["④ Stream Finalization"]
        SSE -->|"[DONE]"| FINAL["Accumulator<br/>finalize()"]
        FINAL --> FULL_RESP["Complete response<br/>(full text, tool_calls, usage)"]
        FULL_RESP --> BREAKER_OK["Update circuit breaker<br/>(record_success)"]
        FULL_RESP --> COST["Record inference_costs"]
        FULL_RESP --> CACHE_STORE["Cache store<br/>(prompt_hash → response)"]
        FULL_RESP --> RETURN["Return to AgentLoop"]
    end

    subgraph Error["⑤ Stream Error Handling"]
        SSE -->|error/timeout| STREAM_ERR["Stream error"]
        STREAM_ERR --> PARTIAL{"Partial content<br/>accumulated?"}
        PARTIAL -->|yes| USE_PARTIAL["Return partial response<br/>with truncation flag"]
        PARTIAL -->|no| FALLBACK["Trigger fallback<br/>provider chain"]
    end
```

---

## 15. Addressability Filter Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Determines whether an inbound message is addressed to the agent. The `FilterChain` applies OR logic across mention, reply, and conversation filters — if any filter matches, the message is dispatched.

```mermaid
flowchart TD
    subgraph Receive["① Inbound Message"]
        MSG["InboundMsg<br/>(channel, sender, content,<br/>metadata)"]
        MSG --> CHAIN["FilterChain<br/>evaluate(message, config)"]
    end

    subgraph Filters["② Filter Evaluation (OR logic)"]
        CHAIN --> MENTION["MentionFilter<br/>@mention or bot name<br/>in message text"]
        CHAIN --> REPLY["ReplyFilter<br/>reply_to_message_id<br/>matches agent's message"]
        CHAIN --> CONV["ConversationFilter<br/>DM or active conversation<br/>window (last N minutes)"]
    end

    subgraph Decision["③ Dispatch Decision"]
        MENTION --> OR{"Any filter<br/>matched?"}
        REPLY --> OR
        CONV --> OR
        OR -->|yes| DISPATCH["Dispatch to AgentLoop<br/>set addressability_source:<br/>mention | reply | conversation"]
        OR -->|no| DROP["Drop message<br/>(not addressed to agent)"]
    end
```

---

## 16. Context Observatory Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Background observability pipeline that records turn-level metrics, analyzes context efficiency, assigns grades, and emits tuning recommendations.

```mermaid
flowchart TD
    subgraph Record["① Turn Recording"]
        TURN["Turn completed"]
        TURN --> RECORDER["TurnRecorder<br/>capture turn metrics:<br/>tokens_in, tokens_out,<br/>tool_calls, duration_ms"]
        RECORDER --> PERSIST_TURN["INSERT turn_observations<br/>(turn_id, metrics, timestamp)"]
    end

    subgraph Snapshot["② Context Snapshot"]
        TURN --> CAPTURE["SnapshotCapture<br/>snapshot context state:<br/>tier sizes, utilization,<br/>memory entries used"]
        CAPTURE --> PERSIST_SNAP["INSERT context_snapshots<br/>(turn_id, snapshot_data)"]
    end

    subgraph Analyze["③ Heuristic Analysis"]
        PERSIST_TURN & PERSIST_SNAP --> ANALYZER["Analyzer<br/>(background task)"]
        ANALYZER --> TOKEN_RATIO["Token efficiency:<br/>output_tokens / input_tokens"]
        ANALYZER --> CACHE_RATE["Cache hit rate:<br/>hits / total lookups"]
        ANALYZER --> TRIM_FREQ["Trim frequency:<br/>progressive trims / turns"]
        ANALYZER --> TOOL_EFF["Tool efficiency:<br/>successful_calls / total_calls"]
    end

    subgraph Efficiency["④ Efficiency Metrics"]
        TOKEN_RATIO & CACHE_RATE & TRIM_FREQ & TOOL_EFF --> ENGINE["compute_efficiency()<br/>(ironclad-db/efficiency.rs)<br/>compute composite score"]
        ENGINE --> TREND["Trend analysis:<br/>sliding window (last 50 turns)"]
    end

    subgraph Grade["⑤ Grading"]
        TREND --> GRADING["Heuristic grading<br/>assign grade A–F"]
        GRADING --> STORE_GRADE["Store efficiency results<br/>(metric_snapshots table)"]
    end

    subgraph Recommend["⑥ Recommendations"]
        STORE_GRADE --> REC_ENGINE["Recommendation logic"]
        REC_ENGINE --> REC_BUDGET["Budget: adjust tier allocations"]
        REC_ENGINE --> REC_CACHE["Cache: tune similarity threshold"]
        REC_ENGINE --> REC_MODEL["Model: suggest routing changes"]
        REC_BUDGET & REC_CACHE & REC_MODEL --> EMIT["Emit recommendations<br/>(EventBus → Dashboard)"]
    end
```

---

## 17. Plugin SDK Execution Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Plugin discovery, manifest parsing, tool registration, and sandboxed execution. Plugins extend the agent's tool surface at runtime.

```mermaid
flowchart TD
    subgraph Discover["① Plugin Discovery"]
        SCAN["Discovery<br/>scan plugins_dir"]
        SCAN --> FIND["Find plugin.toml manifests"]
        FIND --> HASH["SHA-256 content hash"]
        HASH --> CHANGED{"Hash changed<br/>since last load?"}
        CHANGED -->|no| SKIP["Skip (unchanged)"]
        CHANGED -->|yes| PARSE
    end

    subgraph Parse["② Manifest Parsing"]
        PARSE["ManifestParser<br/>parse plugin.toml"]
        PARSE --> VALIDATE["Validate manifest:<br/>name, version, entrypoint,<br/>permissions, tool_defs"]
        VALIDATE --> VALID{"Valid?"}
        VALID -->|no| REJECT["Reject plugin<br/>(log parse error)"]
        VALID -->|yes| REGISTER
    end

    subgraph Register["③ Tool Registration"]
        REGISTER["ToolDef registration"]
        REGISTER --> FOR_EACH["For each tool_def<br/>in manifest"]
        FOR_EACH --> BUILD_DEF["Build ToolDefinition:<br/>name, description,<br/>parameters schema,<br/>risk_level"]
        BUILD_DEF --> INSERT_REGISTRY["Register in ToolRegistry<br/>(prefixed: plugin.name.tool)"]
        INSERT_REGISTRY --> UPSERT_DB["Upsert plugins table<br/>(name, version, hash,<br/>tool_count, status=active)"]
    end

    subgraph Execute["④ Plugin Execution"]
        TOOL_CALL["AgentLoop: tool call<br/>→ plugin.name.tool"]
        TOOL_CALL --> RUNNER["PluginRunner<br/>resolve entrypoint"]
        RUNNER --> PERM_CHECK{"Permissions<br/>satisfied?"}
        PERM_CHECK -->|no| DENY["Deny execution"]
        PERM_CHECK -->|yes| SPAWN["Spawn process"]
    end

    subgraph Sandbox["⑤ Sandboxed Execution"]
        SPAWN --> SANDBOX_APPLY["Sandbox<br/>apply restrictions"]
        SANDBOX_APPLY --> ENV_STRIP["Strip environment<br/>(allowlist only)"]
        ENV_STRIP --> FS_RESTRICT["Filesystem restriction<br/>(plugin dir + tmp only)"]
        FS_RESTRICT --> NET_POLICY{"Network<br/>allowed?"}
        NET_POLICY -->|no| NO_NET["Block outbound"]
        NET_POLICY -->|yes| ALLOW_NET["Allow (logged)"]
        NO_NET & ALLOW_NET --> RUN["Execute with timeout"]
        RUN --> CAPTURE["Capture stdout/stderr"]
        CAPTURE --> PLUGIN_RESULT["PluginResult<br/>(output, exit_code, duration)"]
    end
```

---

## 18. OAuth & Credential Resolution Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Multi-strategy credential resolution: environment variables, encrypted keystore, and OAuth token refresh with automatic rotation.

```mermaid
flowchart TD
    subgraph Resolve["① Credential Resolution"]
        NEED["API call requires credential<br/>(provider, service)"]
        NEED --> STRATEGY{"Resolution<br/>strategy?"}
    end

    subgraph EnvPath["② Environment Variable"]
        STRATEGY -->|env| ENV["EnvVar<br/>std::env::var(key)"]
        ENV --> ENV_FOUND{"Found?"}
        ENV_FOUND -->|yes| ENV_VAL["Use env value"]
        ENV_FOUND -->|no| FALLBACK_KS["Fall back to Keystore"]
    end

    subgraph KeystorePath["③ Keystore Lookup"]
        STRATEGY -->|keystore| KS_DIRECT["Keystore<br/>lookup(service, key_name)"]
        FALLBACK_KS --> KS_DIRECT
        KS_DIRECT --> KS_DECRYPT["Decrypt value<br/>(AES-256-GCM, master key<br/>derived from wallet)"]
        KS_DECRYPT --> KS_FOUND{"Found &<br/>not expired?"}
        KS_FOUND -->|yes| KS_VAL["Use keystore value"]
        KS_FOUND -->|no| OAUTH_NEEDED["Needs OAuth refresh"]
    end

    subgraph OAuthPath["④ OAuth Token Refresh"]
        STRATEGY -->|oauth| OAUTH_DIRECT["OAuthManager<br/>get_token(service)"]
        OAUTH_NEEDED --> OAUTH_DIRECT
        OAUTH_DIRECT --> TOKEN_CHECK{"Cached token<br/>still valid?"}
        TOKEN_CHECK -->|yes| USE_CACHED["Use cached token"]
        TOKEN_CHECK -->|no| REFRESH["OAuthManager<br/>POST /oauth/token<br/>(refresh_token grant)"]
        REFRESH --> REFRESH_OK{"Refresh<br/>succeeded?"}
        REFRESH_OK -->|yes| STORE_TOKEN["Store new token<br/>in Keystore<br/>(encrypted, with TTL)"]
        STORE_TOKEN --> USE_NEW["Use new token"]
        REFRESH_OK -->|no| AUTH_FAIL["Credential error<br/>→ surface to agent"]
    end

    subgraph Inject["⑤ Credential Injection"]
        ENV_VAL & KS_VAL & USE_CACHED & USE_NEW --> INJECT_CRED["CredentialInjection<br/>inject into request"]
        INJECT_CRED --> HEADER["Set Authorization header<br/>or provider-specific header"]
        HEADER --> REDACT["Redact credential from<br/>logs and tool output"]
    end
```

---

## 19. Channel Adapter Lifecycle Dataflow
<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Full lifecycle of a channel adapter: initialization, message reception (webhook or polling), addressability filtering, agent dispatch, response formatting, and health monitoring with auto-reconnect.

```mermaid
flowchart TD
    subgraph Init["① Adapter Initialization"]
        CONFIG["Config<br/>channels.adapter.enabled"]
        CONFIG --> ADAPTER_INIT["AdapterInit<br/>validate credentials,<br/>configure webhook/polling"]
        ADAPTER_INIT --> MODE{"Receive mode?"}
        MODE -->|webhook| WEBHOOK["Register webhook URL<br/>with platform API"]
        MODE -->|polling| POLL_START["Start long-poll loop<br/>(tokio::time::interval)"]
    end

    subgraph Receive["② Message Reception"]
        WEBHOOK --> INBOUND["Receive inbound message"]
        POLL_START --> POLL["Poll<br/>fetch new messages<br/>since last_update_id"]
        POLL --> INBOUND
        INBOUND --> PARSE["Parse platform payload<br/>→ InboundMessage"]
    end

    subgraph Filter["③ Addressability Check"]
        PARSE --> FILTER["FilterChain<br/>evaluate addressability"]
        FILTER --> ADDRESSED{"Addressed<br/>to agent?"}
        ADDRESSED -->|no| DROP["Drop (not for agent)"]
        ADDRESSED -->|yes| SESSION["Lookup/create session<br/>(ironclad-db)"]
    end

    subgraph Dispatch["④ Agent Dispatch"]
        SESSION --> AGENT_DISPATCH["AgentDispatch<br/>send to AgentLoop<br/>(mpsc channel)"]
        AGENT_DISPATCH --> PROCESS["AgentLoop processes<br/>(ReAct cycle)"]
        PROCESS --> AGENT_RESULT["Agent response"]
    end

    subgraph Respond["⑤ Response Delivery"]
        AGENT_RESULT --> FORMAT["Format for platform<br/>(markdown → platform markup,<br/>split long messages)"]
        FORMAT --> SEND["Response<br/>POST to platform API"]
        SEND --> RATE_LIMIT{"Rate limited?"}
        RATE_LIMIT -->|yes| BACKOFF["Exponential backoff<br/>+ retry"]
        BACKOFF --> SEND
        RATE_LIMIT -->|no| DELIVERED["Message delivered"]
    end

    subgraph Lifecycle["⑥ Health & Reconnect"]
        HEALTH["Periodic health check"]
        HEALTH --> CONNECTED{"Connection<br/>healthy?"}
        CONNECTED -->|yes| CONTINUE["Continue"]
        CONNECTED -->|no| RECONNECT["Reconnect<br/>(exponential backoff)"]
        RECONNECT --> ADAPTER_INIT
    end
```

---

## 20. Context Checkpoint Dataflow
<!-- last_updated: 2026-03-01, version: 0.9.0 -->

Checkpoints capture compiled context state every N turns, enabling instant agent readiness on boot and crash recovery with bounded data loss.

```mermaid
flowchart TD
    subgraph turnLoop [Turn Loop]
        turn[InferenceTurn]
        counter[TurnCounter mod N]
    end

    subgraph saveFlow [Checkpoint Save]
        snapshot[CompileContextSnapshot]
        hash[HashSystemPrompt]
        summarize[SummarizeTopKMemory]
        persist[WriteToContextCheckpoints]
    end

    subgraph loadFlow [Checkpoint Load on Boot]
        boot[SessionStart]
        query[QueryLatestCheckpoint]
        validate[ValidateFormatVersion]
        warm[WarmContextFromCheckpoint]
        background[BackgroundFullRetrieval]
    end

    subgraph clearFlow [Checkpoint Cleanup]
        governor[SessionGovernor]
        expire[SessionExpiry]
        clear[ClearCheckpointsForSession]
    end

    turn --> counter
    counter -->|every N turns| snapshot
    snapshot --> hash
    snapshot --> summarize
    hash --> persist
    summarize --> persist
    persist -->|INSERT context_checkpoints| DB[(SQLite)]

    boot --> query
    query -->|SELECT latest by session| DB
    DB --> validate
    validate -->|version match| warm
    validate -->|stale version| background
    warm --> background

    governor --> expire
    expire --> clear
    clear -->|DELETE by session_id| DB
```

Config: `[context.checkpoint]` — `enabled` (bool), `every_n_turns` (u32, default 10).

---

## 21. Durable Delivery Queue Dataflow
<!-- last_updated: 2026-03-01, version: 0.9.0 -->

Outbound channel messages are persisted before send attempts. On crash recovery, pending deliveries are replayed from the store, preventing message loss.

```mermaid
flowchart TD
    subgraph sendPath [Send Path]
        reply[ChannelReply]
        enqueue[PersistToDeliveryQueue]
        attempt[SendViaAdapter]
        success[MarkDelivered]
        fail[IncrementAttempts]
        retry[ScheduleNextRetry]
        deadletter[MoveToDeadLetter]
    end

    subgraph recoveryPath [Recovery on Boot]
        startup[ServerStartup]
        recover[RecoverFromStore]
        replay[ReplayPendingDeliveries]
    end

    reply --> enqueue
    enqueue -->|INSERT delivery_queue| DB[(SQLite)]
    enqueue --> attempt
    attempt -->|success| success
    attempt -->|failure| fail
    success -->|UPDATE status=delivered| DB
    fail --> retry
    retry -->|UPDATE next_retry_at| DB
    fail -->|max attempts exceeded| deadletter
    deadletter -->|UPDATE status=dead_letter| DB

    startup --> recover
    recover -->|SELECT status=pending| DB
    recover --> replay
    replay --> attempt
```

Schema: `delivery_queue` table with idempotency_key, attempt count, next_retry_at, and terminal failure reason.

---

## 22. Episodic Digest Dataflow
<!-- last_updated: 2026-03-01, version: 0.9.0 -->

When sessions close (TTL expiry, rotation, archive), an LLM-generated digest captures key decisions, unresolved tasks, and learned facts. These digests feed future context assembly with decay-weighted relevance.

```mermaid
flowchart TD
    subgraph triggerLayer [Digest Triggers]
        ttl[SessionTTLExpiry]
        rotate[SessionRotation]
        archive[CompactBeforeArchive]
    end

    subgraph digestLayer [Digest Generation]
        governor[SessionGovernor]
        fetch[FetchRecentMessages]
        generate[DigestOnClose]
        llm[LLMSummarize]
        store[StoreAsEpisodicMemory]
    end

    subgraph retrievalLayer [Digest Retrieval]
        newSession[NewSessionStart]
        retrieve[MemoryRetriever]
        decay[DecayWeightedRelevance]
        inject[InjectIntoContext]
    end

    ttl --> governor
    rotate --> governor
    archive --> governor
    governor --> fetch
    fetch -->|list_messages| DB[(SQLite)]
    fetch --> generate
    generate --> llm
    llm --> store
    store -->|INSERT episodic_memory with digest flag| DB

    newSession --> retrieve
    retrieve -->|hybrid FTS5 + vector search| DB
    retrieve --> decay
    decay --> inject
```

Config: `[digest]` — `enabled` (bool), `max_tokens` (usize, default 512).

---

## 23. Prompt Compression Dataflow
<!-- last_updated: 2026-03-01, version: 0.9.0 -->

When enabled, the prompt compression gate reduces token count in assembled prompts before inference. Targets long conversation histories and verbose tool descriptions.

```mermaid
flowchart TD
    subgraph assembly [Context Assembly]
        system[SystemPrompt]
        memory[MemoryRetrieval]
        history[ConversationHistory]
        tools[ToolDescriptions]
        assemble[AssembleFullPrompt]
    end

    subgraph compression [Compression Gate]
        gate{prompt_compression enabled?}
        measure[MeasureTokenCount]
        compress[PromptCompressor]
        ratio[TargetCompressionRatio]
        pruned[PrunedPrompt]
    end

    subgraph inference [Inference]
        send[SendToLLM]
    end

    system --> assemble
    memory --> assemble
    history --> assemble
    tools --> assemble
    assemble --> gate
    gate -->|disabled| send
    gate -->|enabled| measure
    measure --> compress
    ratio --> compress
    compress --> pruned
    pruned --> send
```

Config: `[cache]` — `prompt_compression` (bool, default false), `compression_target_ratio` (f64, default 0.5).

---

## 24. Introspection Tool Architecture
<!-- last_updated: 2026-03-01, version: 0.9.0 -->

Four read-only introspection tools give the agent self-awareness of its runtime state, memory tiers, channel health, and subagent/task status. Tools access runtime state through `ToolContext` which now carries optional database and channel references.

```mermaid
flowchart TD
    subgraph entryPoints [Entry Points]
        api[REST API]
        channel[Channel Message]
        ws[WebSocket]
    end

    subgraph threading [Context Threading]
        input[InferenceInput.channel_label]
        react[RunInferenceAndReact]
        exec[ExecuteToolCall]
        ctx[ToolContext]
    end

    subgraph tools [Introspection Tools]
        runtime[GetRuntimeContextTool]
        memory[GetMemoryStatsTool]
        health[GetChannelHealthTool]
        subagent[GetSubagentStatusTool]
    end

    subgraph data [Data Sources]
        ctxFields[session_id, agent_id, authority, workspace_root, channel]
        budgets[Memory Tier Budgets: working 30%, episodic 25%, semantic 20%, procedural 15%, relationship 10%]
        dbAgents[sub_agents table]
        dbTasks[tasks table]
    end

    api -->|channel_label: api| input
    channel -->|channel_label: platform| input
    ws -->|channel_label: ws| input
    input --> react
    react --> exec
    exec --> ctx

    ctx --> runtime
    ctx --> memory
    ctx --> health
    ctx --> subagent

    runtime --> ctxFields
    memory --> budgets
    health --> ctxFields
    subagent -->|db.conn()| dbAgents
    subagent -->|db.conn()| dbTasks
```

`ToolContext` fields: `session_id`, `agent_id`, `authority`, `workspace_root`, `channel: Option<String>`, `db: Option<Database>`.

---

## Cross-Reference Tables

### Table References

| Diagram | Tables Referenced |
| --------- | ------------------- |
| 1. Request | sessions, policy_decisions, semantic_cache, inference_costs, session_messages, turns, tool_calls, embeddings |
| 2. Cache | semantic_cache, inference_costs |
| 3. Router | inference_costs |
| 4. Memory | working_memory, episodic_memory, semantic_memory, procedural_memory, relationship_memory, memory_fts, embeddings |
| 5. A2A | relationship_memory, discovered_agents |
| 6. Injection | policy_decisions, relationship_memory, metric_snapshots |
| 7. Financial | transactions, inference_costs |
| 8. Scheduling | cron_jobs, cron_runs, sessions |
| 9. Skills | skills, policy_decisions |
| 10. Approval Workflow | approval_requests, policy_decisions |
| 11. Browser Tool | (no direct DB tables; session state in-memory) |
| 12. Context Assembly | context_snapshots, working_memory, episodic_memory, semantic_memory, procedural_memory, relationship_memory |
| 13. Response Transform | turns (thinking_content column) |
| 14. Streaming LLM | inference_costs, semantic_cache |
| 15. Addressability Filter | (no direct DB tables; filter logic in-memory) |
| 16. Context Observatory | turn_observations, context_snapshots, observatory_grades, metric_snapshots |
| 17. Plugin SDK | plugins, policy_decisions |
| 18. OAuth & Credentials | (keystore encrypted on-disk, tokens in-memory) |
| 19. Channel Adapter | sessions |
| 20. Context Checkpoint | context_checkpoints, sessions |
| 21. Durable Delivery Queue | delivery_queue |
| 22. Episodic Digest | sessions, session_messages, episodic_memory |
| 23. Prompt Compression | (no direct DB tables; compression is in-memory before inference) |
| 24. Introspection Tools | sub_agents, tasks, working_memory, episodic_memory, semantic_memory, procedural_memory, relationship_memory, delivery_queue |

Tables not referenced by any diagram: `schema_version` (infrastructure-only), `proxy_stats`, `identity`, `soul_history` -- these are straightforward CRUD subsystems not requiring dataflow diagrams.

### Crate References

| Diagram | Crates Referenced |
| --------- | ------------------- |
| 1. Request | ironclad-channels, ironclad-db, ironclad-agent, ironclad-llm |
| 2. Cache | ironclad-llm, ironclad-db |
| 3. Router | ironclad-llm |
| 4. Memory | ironclad-agent, ironclad-db, ironclad-llm |
| 5. A2A | ironclad-channels, ironclad-wallet |
| 6. Injection | ironclad-agent |
| 7. Financial | ironclad-wallet, ironclad-schedule, ironclad-agent, ironclad-core |
| 8. Scheduling | ironclad-schedule, ironclad-agent, ironclad-db |
| 9. Skills | ironclad-agent, ironclad-db |
| 10. Approval Workflow | ironclad-agent, ironclad-server, ironclad-db |
| 11. Browser Tool | ironclad-agent |
| 12. Context Assembly | ironclad-agent, ironclad-llm, ironclad-db |
| 13. Response Transform | ironclad-llm, ironclad-agent |
| 14. Streaming LLM | ironclad-llm, ironclad-server |
| 15. Addressability Filter | ironclad-channels |
| 16. Context Observatory | ironclad-agent, ironclad-db |
| 17. Plugin SDK | ironclad-agent, ironclad-db |
| 18. OAuth & Credentials | ironclad-wallet, ironclad-llm |
| 19. Channel Adapter | ironclad-channels, ironclad-agent, ironclad-db |
| 20. Context Checkpoint | ironclad-db, ironclad-agent, ironclad-core |
| 21. Durable Delivery Queue | ironclad-channels, ironclad-db |
| 22. Episodic Digest | ironclad-agent, ironclad-db, ironclad-llm, ironclad-schedule |
| 23. Prompt Compression | ironclad-llm, ironclad-agent |
| 24. Introspection Tools | ironclad-agent, ironclad-db |

`ironclad-server` is not dataflow-diagrammed because it is the outer shell that dispatches to channel adapters and serves the dashboard.

### Config Key References

| Diagram | Config Keys Referenced |
| --------- | ---------------------- |
| 1. Request | (crate-level config, not direct keys) |
| 2. Cache | cache.semantic_threshold, cache.exact_match_ttl_seconds, cache.max_entries |
| 3. Router | models.routing.mode, models.routing.confidence_threshold, models.routing.local_first, models.primary, models.fallbacks |
| 4. Memory | memory.working_budget_pct, memory.episodic_budget_pct, memory.semantic_budget_pct, memory.procedural_budget_pct, memory.relationship_budget_pct, memory.embedding_provider, memory.embedding_model, memory.hybrid_weight, memory.ann_index |
| 5. A2A | wallet.chain_id, wallet.rpc_url, a2a.max_message_size, a2a.rate_limit_per_peer |
| 6. Injection | (hardcoded thresholds; `hmac_session_secret` stored in `identity` table) |
| 7. Financial | yield.enabled, yield.min_deposit, yield.withdrawal_threshold, yield.protocol, treasury.minimum_reserve, treasury.per_payment_cap, treasury.hourly_transfer_limit, treasury.daily_transfer_limit, treasury.daily_inference_budget, wallet.rpc_url, wallet.path |
| 8. Scheduling | (cron_jobs in DB, not config) |
| 9. Skills | skills.skills_dir, skills.script_timeout_seconds, skills.script_max_output_bytes, skills.allowed_interpreters, skills.sandbox_env |
| 10. Approval Workflow | approval.enabled, approval.timeout_seconds, approval.gated_risk_levels |
| 11. Browser Tool | browser.enabled, browser.chrome_path, browser.max_sessions, browser.idle_timeout_seconds |
| 12. Context Assembly | context.system_budget_pct, context.memory_budget_pct, context.history_budget_pct, context.tool_budget_pct |
| 13. Response Transform | (pipeline stages hardcoded; no user-facing config) |
| 14. Streaming LLM | models.stream_by_default |
| 15. Addressability Filter | channels.addressability.mention_names, channels.addressability.conversation_window_minutes |
| 16. Context Observatory | observatory.enabled, observatory.analysis_interval_turns, observatory.window_size |
| 17. Plugin SDK | plugins.plugins_dir, plugins.sandbox_env, plugins.allowed_network |
| 18. OAuth & Credentials | credentials.keystore_path, credentials.oauth_providers |
| 19. Channel Adapter | channels.telegram.enabled, channels.whatsapp.enabled, channels.telegram.polling_interval_ms |

### Differentiator Coverage

| Differentiator | Diagram |
| --------------- | --------- |
| Semantic cache | 2 |
| Persistent semantic cache (SQLite-backed) | 2 |
| Heuristic model routing | 3 |
| Yield engine | 7 |
| Zero-trust A2A | 5 |
| Multi-layer injection defense | 6 |
| Unified SQLite DB | All |
| In-process routing (no IPC) | 1 |
| Progressive context loading | 1 |
| HMAC trust boundaries | 6 (Layer 2) |
| Connection pooling | 1 |
| Dual-format skill system | 9 |
| Sandboxed script execution | 9 |
| Hybrid RAG (FTS5 + vector cosine) | 1, 4 |
| Multi-provider embedding (OpenAI/Ollama/Google + n-gram fallback) | 1, 4 |
| Binary BLOB embedding storage | 4 |
| HNSW ANN index (instant-distance) | 4 |
| Content chunking with overlap | 4 |
| Human-in-the-loop approval gating | 10 |
| CDP browser automation | 11 |
| Complexity-driven context assembly | 12 |
| Response transform pipeline | 13 |
| Token-by-token SSE streaming | 14 |
| Multi-filter addressability (OR logic) | 15 |
| Context observatory (grading + recommendations) | 16 |
| Plugin SDK (sandboxed execution) | 17 |
| Multi-strategy credential resolution | 18 |
| Channel adapter lifecycle (health + reconnect) | 19 |
| Hippocampus schema introspection | 4 |
| ML model routing (ONNX alternative) | 3 |

### Error/Failure Paths

| Diagram | Error Paths Shown |
| --------- | ------------------- |
| 1. Request | Injection block, cache miss, circuit breaker block, dedup reject, fallback exhaustion, policy deny |
| 2. Cache | All 3 miss paths, eviction |
| 3. Router | Provider unavailable, T1 quality failure, escalation, all providers exhausted |
| 4. Memory | No error paths (CRUD against SQLite) |
| 5. A2A | Stale timestamp reject (both sides), signature verification failure |
| 6. Injection | Block, sanitize, deny paths for all 4 layers |
| 7. Financial | Policy deny, insufficient balance |
| 8. Scheduling | Lease contention (skip), error status recording |
| 9. Skills | Unlisted interpreter reject, policy deny, script timeout, no matching skills |
| 10. Approval Workflow | Approval timeout, admin deny |
| 11. Browser Tool | Session timeout, CDP connect failure, max sessions exceeded |
| 12. Context Assembly | Progressive trim (over budget), model limit exceeded |
| 13. Response Transform | PII leak detected, injection echo detected, response truncation |
| 14. Streaming LLM | Stream error/timeout, partial response, fallback chain |
| 15. Addressability Filter | All filters fail (message dropped) |
| 16. Context Observatory | (no error paths; observatory is advisory-only) |
| 17. Plugin SDK | Invalid manifest, permission denied, sandbox timeout |
| 18. OAuth & Credentials | Env var missing, token expired, OAuth refresh failure |
| 19. Channel Adapter | Rate limited (backoff), connection unhealthy (reconnect) |

---

## Test Surface Map (90% Unit Test Coverage Target)

### Diagram 1 -- Primary Request

| Crate | Module | Functions to Test | Mock Strategy |
| ------- | -------- | ------------------- | --------------- |
| ironclad-channels | telegram.rs, whatsapp.rs, web.rs | `parse_inbound()`, `format_outbound()` | Mock HTTP payloads |
| ironclad-db | sessions.rs | `find_or_create()`, `append_message()` | In-memory SQLite |
| ironclad-agent | context.rs | `build_context()`, `progressive_load()` | Fixture sessions |
| ironclad-agent | prompt.rs | `build_system_prompt()`, `inject_hmac_boundaries()` | Known inputs/outputs |
| ironclad-llm | format.rs | All `From<ApiFormat>` impls (12+ pairs) | Pure function tests |
| ironclad-llm | circuit.rs | `is_blocked()`, `record_429()`, `record_success()`, `record_credit_error()`, exponential backoff | Time-based tests with `tokio::time::pause()` |
| ironclad-llm | dedup.rs | `fingerprint()`, `check_and_track()`, `release()`, TTL expiry | Time-based tests |
| ironclad-llm | tier.rs | `classify()`, `adapt_t1()`, `adapt_t2()`, `adapt_t3t4()` | Known model -> tier mappings |
| ironclad-llm | client.rs | `forward_request()`, `process_response()` | `mockall` mock of `reqwest::Client` |
| ironclad-db | metrics.rs | `record_inference_cost()`, `query_hourly()`, `query_daily()` | In-memory SQLite |

### Diagram 2 -- Semantic Cache

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
| ironclad-llm/cache.rs | `lookup_exact()`, `lookup_semantic()`, `lookup_tool_ttl()`, `lookup()`, `store()`, `store_with_embedding()`, `evict_expired()`, `evict_lfu()`, `compute_hash()` | In-memory HashMap, fixed n-gram vectors for tests |

### Diagram 3 -- Heuristic Router

| Module | Functions to Test | Mock Strategy |
| -------- | ------------------- | --------------- |
| ironclad-llm/router.rs | `extract_features()`, `classify_complexity()`, `select_model()`, `select_for_complexity()`, `advance_fallback()`, `reset()` | HeuristicBackend (no mock; pure functions), mock ProviderRegistry |

### Diagram 4 -- Memory

| Module | Functions to Test | Mock Strategy |
| -------- | ------------------- | --------------- |
| ironclad-db/memory.rs | `store_working()`, `store_episodic()`, `store_semantic()`, `store_procedural()`, `store_relationship()`, `retrieve_*()` for all 5 tiers, `prune_*()`, `fts_search()` | In-memory SQLite |
| ironclad-agent/memory.rs | `classify_turn()`, `extract_episodic()`, `extract_semantic()`, `extract_procedural()`, `allocate_budget()`, `format_memory_block()` | Fixture turns, mock DB |

### Diagram 5 -- A2A

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
| ironclad-channels/a2a.rs | `generate_hello()`, `verify_hello()`, `generate_response()`, `verify_response()`, `derive_session_key()`, `encrypt_message()`, `decrypt_message()`, `validate_timestamp()`, `check_rate_limit()`, `check_message_size()` | Mock alloy-rs signer (deterministic keypairs), mock ERC-8004 registry (return known agent cards) |

### Diagram 6 -- Injection Defense

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
| ironclad-agent/injection.rs | `check_regex_patterns()`, `check_encoding_evasion()`, `check_financial_manipulation()`, `check_multilang()`, `compute_threat_score()`, `sanitize()` | Corpus of known injection strings (from academic papers + established injection test suites) |
| ironclad-agent/prompt.rs | `inject_hmac_boundary()`, `verify_hmac_boundary()` | Known session secrets + content hashes |
| ironclad-agent/policy.rs | `evaluate_authority()`, `check_financial_peer()`, `check_self_mod_authority()` | Fixture tool calls with various sources |

### Diagram 7 -- Financial

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
| ironclad-wallet/treasury.rs | `check_per_payment()`, `check_hourly_limit()`, `check_daily_limit()`, `check_minimum_reserve()`, `check_inference_budget()` | In-memory SQLite with fixture transactions |
| ironclad-wallet/yield_engine.rs | `calculate_excess()`, `should_deposit()`, `should_withdraw()`, `record_yield_earned()` | Mock alloy-rs contract calls (return balances), in-memory SQLite |
| ironclad-wallet/x402.rs | `build_payment_header()`, `sign_transfer_with_authorization()` | Mock signer (deterministic) |
| ironclad-wallet/wallet.rs | `load_or_generate()`, `sign_message()`, `public_address()` | Temp directory for wallet files |

### Diagram 8 -- Scheduling

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
| ironclad-schedule/scheduler.rs | `evaluate_cron()`, `evaluate_interval()`, `evaluate_at()`, `acquire_lease()`, `release_lease()`, `calculate_next_run()` | In-memory SQLite, fixed timestamps |
| ironclad-schedule/heartbeat.rs | `build_tick_context()`, `run_tick()` | Mock credit/USDC fetchers, mock agent loop |
| ironclad-schedule/tasks.rs | Each built-in task function | Mock dependencies per task |

### Diagram 9 -- Skills

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
| ironclad-agent/skills.rs | `SkillLoader::scan_dir()`, `SkillLoader::parse_toml()`, `SkillLoader::parse_md()`, `SkillLoader::compute_hash()`, `SkillLoader::hot_reload()`, `SkillRegistry::match_skills()`, `SkillRegistry::index_triggers()`, `StructuredSkillExecutor::execute_chain()`, `InstructionSkillExecutor::inject_body()` | Fixture .toml/.md skill files in temp dir, in-memory SQLite, mock ToolRegistry |
| ironclad-agent/script_runner.rs | `ScriptRunner::execute()`, `check_interpreter_whitelist()`, `build_sandbox_env()`, timeout enforcement, output truncation | Fixture scripts (bash echo, python print, slow script for timeout), temp working dirs |
| ironclad-db/skills.rs | `register_skill()`, `get_skill()`, `list_skills()`, `update_skill()`, `delete_skill()`, `find_by_trigger()`, `check_content_hash()` | In-memory SQLite |

### Integration Tests (not counted toward unit 90%)

| Test | Scope | Mock Strategy |
|------|-------|---------------|
| Full request round-trip | Channel -> Agent -> LLM -> Response | Mock LLM provider (wiremock), in-memory SQLite |
| A2A handshake + message | Two Ironclad instances | Localhost, deterministic keys |
| Cron job fires and completes | Scheduler -> Agent -> DB | In-memory SQLite, mock LLM |
| Injection attack corpus | 55+ known attacks from literature | No mocks (tests the actual defense) |
| Yield deposit/withdraw cycle | Balance changes trigger actions | Mock Aave contract |
| Structured skill execution | Skill trigger -> tool chain -> script | Fixture skill files, mock LLM |
| Instruction skill injection | Skill trigger -> prompt injection -> LLM response | Fixture .md skill, mock LLM |
| Skill hot-reload | File change detected -> re-index | Temp dir, file write + reload trigger |

### Coverage Estimation

With the test surface above, estimated per-crate coverage:

| Crate | Estimated Coverage | Notes |
|-------|-------------------|-------|
| ironclad-core | 95% | Enums, config parsing, error types -- all pure |
| ironclad-db | 95% | All CRUD testable with in-memory SQLite |
| ironclad-llm | 90% | format.rs 100%, circuit/dedup/tier 95%, client.rs ~85% (streaming harder to test) |
| ironclad-agent | 90% | injection.rs 95%, policy.rs 95%, memory.rs 90%, skills.rs 92%, script_runner.rs 90%, loop.rs ~85% (ReAct cycle state machine) |
| ironclad-schedule | 92% | Scheduler logic pure, heartbeat needs mock time |
| ironclad-wallet | 88% | Treasury 95%, wallet/x402 90%, yield_engine ~80% (DeFi contract interactions) |
| ironclad-channels | 85% | telegram/whatsapp parsing 90%, a2a crypto 95%, WebSocket ~75% |
| ironclad-server | 80% | API routes testable, dashboard static serving less so |
| **Weighted Average** | **~90%** | Meets target |
