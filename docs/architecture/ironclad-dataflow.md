# Ironclad Dataflow Diagrams

*Data flows for the Ironclad architecture -- a single Rust binary autonomous agent runtime.*

**Convention**: every SQLite table name, config key, crate name, and Rust type referenced in these diagrams is cross-referenced against `ironclad-design.md` in the cross-reference section at the end.

---

## 1. Primary Request Dataflow

End-to-end path from inbound user message to delivered response, entirely within one OS process.

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
        MODEL_SELECT --> FORMAT_XLATE["ironclad-llm/format.rs<br/>ApiFormat translation"]
        FORMAT_XLATE --> CIRCUIT{"Circuit Breaker<br/>blocked?"}
        CIRCUIT -->|"blocked → advance fallback"| MODEL_SELECT
        CIRCUIT -->|open| DEDUP{"In-flight<br/>duplicate?"}
        DEDUP -->|duplicate| DEDUP_REJECT["429 reject"]
        DEDUP -->|unique| TIER_ADAPT["ironclad-llm/tier.rs<br/>T1: condense · T2: reorder<br/>T3/T4: passthrough + cache_control"]
        TIER_ADAPT --> FORWARD["ironclad-llm/client.rs<br/>HTTP/2 forward (reqwest pool)"]
        FORWARD --> UPSTREAM["LLM Provider<br/>(Anthropic / Google / Moonshot /<br/>OpenAI / Ollama)"]
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

## 3. Heuristic Model Router Dataflow

Implemented in `ironclad-llm/router.rs`. **No ONNX or ML** — heuristic classifier only.

```mermaid
flowchart TD
    QUERY["Incoming query<br/>(post context assembly)"]
    QUERY --> MODE{"models.routing.mode?"}

    MODE -->|"primary"| DIRECT["Use primary model"]

    MODE -->|"heuristic / ml"| FEATURES["extract_features():<br/>message len, tool_call count, depth"]
    FEATURES --> HEURISTIC["HeuristicBackend::classify_complexity<br/>weighted sum → score 0.0–1.0"]

    subgraph ModelSelection["Model Selection"]
        HEURISTIC --> LOCAL_FIRST{"local_first &&<br/>score < threshold?"}
        LOCAL_FIRST -->|yes| T1_ROUTE["Use primary (local)"]
        LOCAL_FIRST -->|no| PRIMARY_ROUTE["select_for_complexity:<br/>high score → fallback[0]<br/>else primary"]
    end

    subgraph ProviderCheck["Provider Availability (with fallback chain)"]
        DIRECT & T1_ROUTE & PRIMARY_ROUTE --> BREAKER{"Circuit breaker<br/>open?"}
        BREAKER -->|open| ACCEPT["Accept provider · forward"]
        BREAKER -->|blocked| FALLBACK{"Fallbacks<br/>remaining?"}
        FALLBACK -->|yes| NEXT["Advance to next<br/>fallback model"]
        NEXT --> BREAKER
        FALLBACK -->|no| EXHAUST["All providers exhausted<br/>→ error to caller"]
    end

    subgraph QualityGate["Response Quality Gate (local models)"]
        ACCEPT --> QUALITY{"Response<br/>quality OK?"}
        QUALITY -->|yes| RECORD
        QUALITY -->|"no (local only)"| ESCALATE["Escalate → re-enter<br/>with next fallback"]
        ESCALATE --> BREAKER
    end

    RECORD["Record inference_costs<br/>(model, provider, tier,<br/>tokens_in/out, cost)"]
```

---

## 4. Memory Lifecycle Dataflow

5-tier memory system unified in a single SQLite DB. Ingestion in `ironclad-agent/memory.rs`, storage in `ironclad-db/memory.rs`.

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
        INFER_START --> EMBED_QUERY_R["ironclad-llm/embedding.rs<br/>Generate query embedding"]
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

## 5. Zero-Trust Agent-to-Agent Communication Dataflow

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
        PAYLOAD_KIND -->|agentTurn| AGENT_WAKE["Inject message into agent loop"]
        PAYLOAD_KIND -->|systemEvent| SYS_EVENT["Process system event"]
        AGENT_WAKE --> SESSION_SELECT{"session_target?"}
        SESSION_SELECT -->|main| MAIN["Use main session"]
        SESSION_SELECT -->|isolated| ISO["Create isolated session"]
        MAIN & ISO --> RUN_REACT["Run ReAct loop turn"]
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

    RUN_REACT & SYS_EVENT --> UPDATE_JOB
    RUN_REACT --> DELIVERY_MODE

    subgraph WakeSignal["⑤ In-Process Wake Signal"]
        SHOULD_WAKE{"Task signals shouldWake?"}
        SHOULD_WAKE -->|no| WAKE_DONE["No wake"]
        SHOULD_WAKE -->|yes| MPSC_SEND["mpsc::send(WakeEvent)"]
        MPSC_SEND --> SLEEP_SELECT["Agent sleep loop:<br/>tokio::select! on<br/>mpsc::recv | 30s poll"]
        SLEEP_SELECT --> AGENT_WAKES["Agent loop resumes"]
    end

    RUN_REACT --> SHOULD_WAKE
```

---

## 9. Skill Execution Dataflow

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

Tables not referenced by any diagram: `schema_version` (infrastructure-only), `tasks`, `proxy_stats`, `identity`, `soul_history` -- these are straightforward CRUD subsystems not requiring dataflow diagrams.

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
