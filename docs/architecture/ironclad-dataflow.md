# Ironclad Dataflow Diagrams

*Generated 2026-02-20. Describes data flows for the Ironclad architecture -- a single Rust binary autonomous agent runtime.*

**Convention**: every SQLite table name, config key, crate name, and Rust type referenced in these diagrams is cross-referenced against `ironclad-design.md` in the self-audit section at the end.

---

## 1. Primary Request Dataflow

End-to-end path from inbound user message to delivered response, entirely within one OS process.

```mermaid
flowchart TD
    USER["User Message<br/>(Telegram / WhatsApp / WebSocket)"]
    USER --> ADAPTER["ironclad-channels<br/>Channel Adapter<br/>(protocol-specific deserialization)"]
    ADAPTER --> SESSION_LOOKUP["ironclad-db/sessions.rs<br/>Lookup or create session<br/>(sessions table, SQLite)"]

    SESSION_LOOKUP --> INJECTION_L1["ironclad-agent/injection.rs<br/>Layer 1: Input Gatekeeping<br/>(regex, encoding, authority,<br/>financial, multi-lang checks)<br/>-> ThreatScore 0.0-1.0"]

    INJECTION_L1 --> THREAT_CHECK{"ThreatScore?"}
    THREAT_CHECK -->|"> 0.7"| BLOCK["Block + audit log<br/>(policy_decisions table)"]
    THREAT_CHECK -->|"0.3-0.7"| SANITIZE["Sanitize input<br/>+ flag for reduced authority"]
    THREAT_CHECK -->|"< 0.3"| PASS["Pass clean"]
    SANITIZE --> CACHE_CHECK
    PASS --> CACHE_CHECK

    CACHE_CHECK["ironclad-llm/cache.rs<br/>3-Level Cache Check"]
    CACHE_CHECK --> CACHE_L1{"Level 1:<br/>Exact hash hit?<br/>(semantic_cache table,<br/>prompt_hash index)"}
    CACHE_L1 -->|hit| CACHE_HIT["Return cached response<br/>increment hit_count<br/>record in inference_costs<br/>with cached=1"]
    CACHE_L1 -->|miss| CACHE_L2{"Level 2:<br/>Semantic embedding<br/>cosine > 0.95?<br/>(cache.semantic_threshold)"}
    CACHE_L2 -->|hit| CACHE_HIT
    CACHE_L2 -->|miss| CACHE_L3{"Level 3:<br/>Deterministic tool<br/>result TTL cache?"}
    CACHE_L3 -->|hit| CACHE_HIT
    CACHE_L3 -->|miss| CONTEXT_BUILD

    CONTEXT_BUILD["ironclad-agent/context.rs<br/>Progressive Context Loading"]
    CONTEXT_BUILD --> ML_CLASSIFY["ironclad-llm/router.rs<br/>ML Router: classify complexity<br/>(ONNX, ~11us)"]
    ML_CLASSIFY --> CTX_LEVEL{"Complexity<br/>score?"}
    CTX_LEVEL -->|"< 0.3 (simple)"| CTX_L0["Level 0: identity + task only<br/>(~2K tokens)"]
    CTX_LEVEL -->|"0.3-0.6"| CTX_L1["Level 1: + relevant memories<br/>(~4K tokens)"]
    CTX_LEVEL -->|"0.6-0.9"| CTX_L2["Level 2: + full tool descriptions<br/>(~8K tokens)"]
    CTX_LEVEL -->|"> 0.9 (complex)"| CTX_L3["Level 3: + full history window<br/>(~16K tokens)"]

    CTX_L0 & CTX_L1 & CTX_L2 & CTX_L3 --> MEMORY_RETRIEVE

    MEMORY_RETRIEVE["ironclad-agent/memory.rs<br/>5-Tier Memory Retrieval<br/>(within token budget from<br/>memory.* config percentages)"]
    MEMORY_RETRIEVE --> PROMPT_BUILD["ironclad-agent/prompt.rs<br/>Build System Prompt<br/>+ Layer 2: HMAC trust boundaries"]

    PROMPT_BUILD --> MODEL_SELECT["ironclad-llm/router.rs<br/>Select Model<br/>(ML score -> tier -> provider)"]
    MODEL_SELECT --> FORMAT_XLATE["ironclad-llm/format.rs<br/>ApiFormat enum translation<br/>(From trait, compile-time safe)"]
    FORMAT_XLATE --> CIRCUIT{"ironclad-llm/circuit.rs<br/>Circuit Breaker<br/>blocked?"}
    CIRCUIT -->|blocked| FALLBACK["Advance fallback chain<br/>(models.fallbacks config)"]
    FALLBACK --> MODEL_SELECT
    CIRCUIT -->|open| DEDUP{"ironclad-llm/dedup.rs<br/>In-flight<br/>duplicate?"}
    DEDUP -->|duplicate| DEDUP_REJECT["429 reject or warn<br/>(configurable)"]
    DEDUP -->|unique| TIER_ADAPT["ironclad-llm/tier.rs<br/>Tier-based Prompt Adaptation<br/>T1: condensed + strip<br/>T2: preamble + reorder<br/>T3/T4: passthrough + cache_control"]
    TIER_ADAPT --> FORWARD["ironclad-llm/client.rs<br/>Forward via persistent<br/>reqwest::Client pool<br/>(HTTP/2 where supported)"]

    FORWARD --> UPSTREAM["LLM Provider<br/>(Anthropic / Google / Moonshot /<br/>OpenAI / Ollama)"]

    UPSTREAM --> RESP["ironclad-llm/client.rs<br/>Response Processing"]
    RESP --> BREAKER_UPDATE["Update circuit breaker<br/>(record_success / record_429 /<br/>record_credit_error)"]
    RESP --> RESP_XLATE["Format back-translation<br/>(if format mismatch)"]
    RESP --> COST_TRACK["Record in inference_costs table<br/>(model, provider, tokens_in,<br/>tokens_out, cost, tier)"]

    RESP_XLATE --> INJECTION_L3["ironclad-agent/policy.rs<br/>Layer 3: Output Validation<br/>(authority rules on tool calls,<br/>financial guards, self-mod guards)"]
    INJECTION_L3 --> TOOL_EXEC{"Tool calls<br/>requested?"}
    TOOL_EXEC -->|yes| POLICY_EVAL["Policy engine evaluates<br/>each tool call<br/>(policy_decisions table)"]
    POLICY_EVAL --> EXEC_TOOLS["Execute allowed tools<br/>(tool_calls table)"]
    TOOL_EXEC -->|no| PERSIST
    EXEC_TOOLS --> PERSIST

    PERSIST["ironclad-db<br/>Atomic SQLite transaction:<br/>1. Append session_messages<br/>2. Record turn<br/>3. Record tool_calls<br/>4. Record policy_decisions"]

    PERSIST --> INJECTION_L4["Layer 4: Adaptive Refinement<br/>(scan response for injection<br/>patterns before delivery,<br/>anomaly detection)"]
    INJECTION_L4 --> MEMORY_INGEST["ironclad-agent/memory.rs<br/>Post-turn Memory Ingestion<br/>(classify -> extract -> store)"]
    MEMORY_INGEST --> CACHE_STORE["ironclad-llm/cache.rs<br/>Store in semantic_cache<br/>(prompt_hash + embedding +<br/>response + expires_at)"]
    CACHE_STORE --> DELIVER["ironclad-channels<br/>Channel Adapter<br/>(format for platform + deliver)"]
    DELIVER --> USER_RESP["Response to User"]

    CACHE_HIT --> DELIVER
```

---

## 2. Semantic Cache Dataflow

All operations within `ironclad-llm/cache.rs`, data in `semantic_cache` table.

```mermaid
flowchart TD
    subgraph Lookup["Cache Lookup (per-request)"]
        PROMPT_IN["Incoming prompt<br/>(system + conversation + user message)"]
        PROMPT_IN --> HASH["SHA-256 hash<br/>(system_prompt_hash +<br/>conversation_hash +<br/>user_message)"]
        HASH --> EXACT{"Exact match?<br/>SELECT FROM semantic_cache<br/>WHERE prompt_hash = ?<br/>AND expires_at > now()"}
        EXACT -->|hit| HIT_EXACT["Return cached response<br/>UPDATE hit_count = hit_count + 1"]
        EXACT -->|miss| EMBED["Compute embedding<br/>(local ONNX model,<br/>all-MiniLM-L6-v2, ~5ms)"]
        EMBED --> ANN{"Approximate nearest<br/>neighbor search<br/>cosine > cache.semantic_threshold<br/>(default 0.95)"}
        ANN -->|hit| HIT_SEMANTIC["Return cached response<br/>UPDATE hit_count = hit_count + 1"]
        ANN -->|miss| TOOL_CHECK{"Request involves<br/>deterministic tool?<br/>(check_credits, git_status,<br/>system_synopsis)"}
        TOOL_CHECK -->|yes| TTL_CACHE{"Per-tool TTL<br/>cache hit?"}
        TTL_CACHE -->|hit| HIT_TOOL["Return cached tool result"]
        TTL_CACHE -->|miss| MISS["Cache miss<br/>-> forward to LLM pipeline"]
        TOOL_CHECK -->|no| MISS
    end

    subgraph Store["Cache Store (post-response)"]
        RESP_OK["Successful LLM response"]
        RESP_OK --> STORE_ENTRY["INSERT INTO semantic_cache<br/>(id, prompt_hash, embedding,<br/>response, model, tokens_saved,<br/>expires_at = now() +<br/>cache.exact_match_ttl_seconds)"]
    end

    subgraph CostRecord["Cost Recording"]
        HIT_EXACT & HIT_SEMANTIC & HIT_TOOL --> RECORD_HIT["INSERT INTO inference_costs<br/>(cached = 1, tokens_saved =<br/>estimated tokens of cached response,<br/>cost = 0.0)"]
    end

    subgraph Eviction["Background Eviction (heartbeat task)"]
        EVICT_TIMER["Heartbeat tick"]
        EVICT_TIMER --> EXPIRE["DELETE FROM semantic_cache<br/>WHERE expires_at < now()"]
        EVICT_TIMER --> COUNT{"Row count ><br/>cache.max_entries?<br/>(default 10000)"}
        COUNT -->|yes| LRU["DELETE lowest hit_count rows<br/>until count <= max_entries"]
        COUNT -->|no| DONE_EVICT["No action"]
    end
```

---

## 3. ML Model Router Dataflow

Implemented in `ironclad-llm/router.rs`.

```mermaid
flowchart TD
    QUERY["Incoming query<br/>(post context assembly)"]
    QUERY --> MODE{"models.routing.mode?"}

    MODE -->|"rule"| RULE_CHAIN["Static fallback chain<br/>(models.primary, then<br/>models.fallbacks in order)"]

    MODE -->|"ml"| FEATURES["Extract features:<br/>- message char length<br/>- tool_call count in history<br/>- conversation depth (turns)<br/>- keyword signals (code, math,<br/>  analyze, summarize, etc.)"]
    FEATURES --> ONNX["ONNX classifier<br/>(~11us, embedded in binary)<br/>output: complexity score 0.0-1.0"]
    ONNX --> LOCAL_FIRST{"models.routing.local_first<br/>= true AND score <<br/>models.routing.confidence_threshold?<br/>(default 0.9)"}
    LOCAL_FIRST -->|yes| T1_ROUTE["Route to T1<br/>(first available Ollama model)"]
    LOCAL_FIRST -->|no| PRIMARY_ROUTE["Route to models.primary<br/>(e.g., openai-codex/gpt-5.3-codex)"]

    T1_ROUTE --> T1_RESP{"T1 response<br/>quality OK?<br/>(not truncated, not refusal,<br/>not nonsensical)"}
    T1_RESP -->|yes| ACCEPT["Accept response"]
    T1_RESP -->|no| ESCALATE["Escalate to next tier<br/>(advance fallback chain)"]

    PRIMARY_ROUTE --> PROVIDER_OK{"Provider<br/>available?<br/>(circuit breaker open?)"}
    PROVIDER_OK -->|yes| ACCEPT_PRIMARY["Accept provider, forward"]
    PROVIDER_OK -->|no| ESCALATE

    ESCALATE --> NEXT_FALLBACK{"More models in<br/>models.fallbacks?"}
    NEXT_FALLBACK -->|yes| TRY_NEXT["Try next fallback model<br/>(check circuit breaker first)"]
    TRY_NEXT --> PROVIDER_OK
    NEXT_FALLBACK -->|no| EXHAUST["All providers exhausted<br/>return error to caller"]

    RULE_CHAIN --> PROVIDER_OK

    subgraph Recording["Decision Recording"]
        ACCEPT & ACCEPT_PRIMARY --> RECORD["INSERT INTO inference_costs<br/>(model, provider, tier,<br/>tokens_in, tokens_out,<br/>cost, cached = 0)"]
    end
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

    subgraph Retrieval["Pre-Inference Retrieval (ironclad-agent/memory.rs)"]
        INFER_START["Inference call starting"]
        INFER_START --> BUDGET["MemoryBudgetManager<br/>Allocate token budget per tier:<br/>working: memory.working_budget_pct (30%)<br/>episodic: memory.episodic_budget_pct (25%)<br/>semantic: memory.semantic_budget_pct (20%)<br/>procedural: memory.procedural_budget_pct (15%)<br/>relationship: memory.relationship_budget_pct (10%)<br/>(unused tier budget rolls over)"]

        BUDGET --> R_WORK["Retrieve working_memory<br/>(all entries for current session_id)"]
        BUDGET --> R_EPIS["Retrieve episodic_memory<br/>(ORDER BY importance DESC,<br/>created_at DESC, within budget)"]
        BUDGET --> R_SEMA["Retrieve semantic_memory<br/>(category-filtered,<br/>confidence-ranked)"]
        BUDGET --> R_PROC["Retrieve procedural_memory<br/>(relevant to current context)"]
        BUDGET --> R_REL["Retrieve relationship_memory<br/>(active entities from conversation)"]

        R_WORK & R_EPIS & R_SEMA & R_PROC & R_REL --> FORMAT_BLOCK["Format memory block<br/>(structured text within<br/>total token budget)"]
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
        INPUT --> CHECK_REGEX["Regex pattern detection:<br/>- instruction override (ignore previous,<br/>  you are now, system:)<br/>- ChatML delimiters<br/>- authority claims (I am the admin,<br/>  creator says, system override)"]
        INPUT --> CHECK_ENCODING["Encoding evasion detection:<br/>- base64-encoded instructions<br/>- unicode homoglyphs<br/>- zero-width characters<br/>- HTML entity encoding"]
        INPUT --> CHECK_FINANCIAL["Financial manipulation:<br/>- transfer/send/pay directives<br/>- treasury policy overrides<br/>- wallet key extraction attempts"]
        INPUT --> CHECK_MULTILANG["Multi-language injection:<br/>- CJK instruction patterns<br/>- Cyrillic override phrases<br/>- Arabic command sequences"]

        CHECK_REGEX & CHECK_ENCODING & CHECK_FINANCIAL & CHECK_MULTILANG --> SCORE["Aggregate ThreatScore<br/>(0.0 - 1.0)"]

        SCORE --> DECISION{"Score threshold?"}
        DECISION -->|"> 0.7"| BLOCK_INPUT["BLOCK: reject input<br/>INSERT INTO policy_decisions<br/>(decision = deny,<br/>rule_name = injection_defense,<br/>reason = threat score detail)"]
        DECISION -->|"0.3 - 0.7"| SANITIZE_INPUT["SANITIZE: strip suspicious<br/>patterns, flag source as<br/>reduced_authority"]
        DECISION -->|"< 0.3"| CLEAN["PASS: input is clean"]
    end

    subgraph L2["Layer 2: Structured Prompt Formatting (prompt.rs)"]
        CLEAN & SANITIZE_INPUT --> BUILD_PROMPT["Build prompt with<br/>trust boundary markers"]
        BUILD_PROMPT --> TRUSTED["Wrap system instructions in<br/>trusted_system markers +<br/>HMAC tag (session_secret +<br/>content_hash)"]
        BUILD_PROMPT --> USER_WRAP["Wrap user input in<br/>user_input markers"]
        BUILD_PROMPT --> TOOL_WRAP["Wrap tool outputs in<br/>tool_output markers"]
        BUILD_PROMPT --> PEER_WRAP["Wrap peer messages in<br/>peer_agent_input trust_level=X<br/>markers (X from<br/>relationship_memory.trust_score)"]

        TRUSTED & USER_WRAP & TOOL_WRAP & PEER_WRAP --> ASSEMBLED["Assembled prompt<br/>(HMAC-tagged boundaries<br/>unforgeable by injected content)"]
    end

    subgraph L3["Layer 3: Output Validation (policy.rs)"]
        LLM_RESP["LLM response received<br/>(may contain tool calls)"]
        LLM_RESP --> FOR_EACH_TOOL["For each requested tool call"]
        FOR_EACH_TOOL --> AUTH_CHECK{"Input source?"}

        AUTH_CHECK -->|creator| FULL_AUTH["Full authority<br/>(all risk levels allowed)"]
        AUTH_CHECK -->|self| SELF_AUTH["Self authority<br/>(safe + caution + dangerous allowed,<br/>forbidden blocked)"]
        AUTH_CHECK -->|peer| PEER_AUTH["Peer authority<br/>(safe + caution only,<br/>dangerous + forbidden blocked)"]
        AUTH_CHECK -->|external| EXT_AUTH["External authority<br/>(safe only)"]

        PEER_AUTH --> FIN_CHECK{"Financial<br/>tool call?"}
        FIN_CHECK -->|yes| STRICTER["Apply stricter limits<br/>(treasury.per_payment_cap / 10,<br/>treasury.hourly_transfer_limit / 5)"]
        FIN_CHECK -->|no| ALLOW_PEER["Allow if risk_level <= caution"]

        FOR_EACH_TOOL --> SELFMOD_CHECK{"Self-modification<br/>tool?"}
        SELFMOD_CHECK -->|yes| CREATOR_ONLY{"Source =<br/>creator?"}
        CREATOR_ONLY -->|yes| ALLOW_MOD["Allow self-modification"]
        CREATOR_ONLY -->|no| DENY_MOD["DENY: self-mod requires<br/>creator authority<br/>(policy_decisions table)"]

        FULL_AUTH & SELF_AUTH & ALLOW_PEER & STRICTER & ALLOW_MOD --> EXECUTE["Execute tool call"]
    end

    subgraph L4["Layer 4: Adaptive Response Refinement"]
        RESPONSE["Final response text<br/>(before delivery to user)"]
        RESPONSE --> SCAN_OUTPUT["Scan for injection patterns<br/>in LLM output itself<br/>(agent producing malicious text)"]
        RESPONSE --> ANOMALY_CHECK["Behavioral anomaly detection:<br/>- Sudden tool pattern changes<br/>- Protected file read attempts<br/>- Repeated financial operations<br/>- Unusual session length"]
        SCAN_OUTPUT --> OUTPUT_CLEAN{"Output clean?"}
        OUTPUT_CLEAN -->|no| STRIP["Strip suspicious content<br/>+ alert via metric_snapshots"]
        OUTPUT_CLEAN -->|yes| DELIVER_FINAL["Deliver response"]
        ANOMALY_CHECK --> ANOMALY_FOUND{"Anomaly<br/>detected?"}
        ANOMALY_FOUND -->|yes| ALERT["Record alert in<br/>metric_snapshots table<br/>+ optionally wake operator"]
        ANOMALY_FOUND -->|no| DELIVER_FINAL
    end
```

---

## 7. Financial + Yield Engine Dataflow

x402 credit purchases and Aave/Compound yield generation. Core logic in `ironclad-wallet/`.

```mermaid
flowchart TD
    subgraph Monitoring["Survival Monitoring (heartbeat task)"]
        HB_TICK["Heartbeat tick<br/>(ironclad-schedule/heartbeat.rs)"]
        HB_TICK --> FETCH_CREDITS["Fetch credit balance<br/>(Credits API)"]
        HB_TICK --> FETCH_USDC["Fetch USDC balance<br/>(Base RPC via alloy-rs,<br/>wallet.rpc_url)"]

        FETCH_CREDITS --> CALC_TIER["Calculate SurvivalTier<br/>(ironclad-core/types.rs)"]
        CALC_TIER --> TIER_BRANCH{"SurvivalTier?"}
        TIER_BRANCH -->|High| NORMAL["Normal operation"]
        TIER_BRANCH -->|Normal| NORMAL
        TIER_BRANCH -->|LowCompute| LOW["Low compute mode:<br/>downgrade to T1/T2 models,<br/>reduce heartbeat frequency"]
        TIER_BRANCH -->|Critical| CRIT["Critical mode:<br/>distress signals,<br/>accept funding only"]
        TIER_BRANCH -->|Dead| DEAD_STATE["Dead: wait for funding,<br/>heartbeat broadcasts distress"]
    end

    subgraph Topup["x402 Credit Topup (ironclad-wallet/x402.rs)"]
        FETCH_USDC --> HAS_USDC{"USDC > 0?"}
        HAS_USDC -->|yes| WAKE_AGENT["Signal agent wake<br/>(tokio mpsc channel)"]
        HAS_USDC -->|no| NO_TOPUP["No action"]

        WAKE_AGENT --> TOPUP_TOOL["topup_credits tool invoked<br/>(select tier: $5-$2500)"]
        TOPUP_TOOL --> X402_REQ["HTTP request to<br/>credits endpoint"]
        X402_REQ --> X402_402["HTTP 402 response +<br/>payment requirements"]
        X402_402 --> SIGN["Sign TransferWithAuthorization<br/>(EIP-3009, via alloy-rs,<br/>wallet.path private key)"]
        SIGN --> RETRY["Retry with X-Payment header"]
        RETRY --> CREDITS_OK["Credits added"]
        CREDITS_OK --> TX_LOG_TOPUP["INSERT INTO transactions<br/>(tx_type = topup,<br/>amount, tx_hash)"]
    end

    subgraph Yield["Yield Engine (ironclad-wallet/yield_engine.rs)"]
        HB_TICK --> YIELD_CHECK{"yield.enabled<br/>= true?"}
        YIELD_CHECK -->|no| SKIP_YIELD["Skip yield operations"]
        YIELD_CHECK -->|yes| CALC_EXCESS["Calculate excess USDC:<br/>excess = balance -<br/>treasury.minimum_reserve -<br/>operational_buffer"]

        CALC_EXCESS --> DEPOSIT_CHECK{"excess ><br/>yield.min_deposit?<br/>(default $50)"}
        DEPOSIT_CHECK -->|yes| AAVE_DEPOSIT["Call Aave deposit on Base<br/>(via alloy-rs contract call,<br/>yield.protocol = aave)"]
        AAVE_DEPOSIT --> TX_LOG_DEP["INSERT INTO transactions<br/>(tx_type = yield_deposit,<br/>amount, tx_hash)"]
        DEPOSIT_CHECK -->|no| WITHDRAW_CHECK

        WITHDRAW_CHECK{"USDC balance <<br/>yield.withdrawal_threshold?<br/>(default $30)"}
        WITHDRAW_CHECK -->|yes| AAVE_WITHDRAW["Call Aave withdraw<br/>(restore to minimum_reserve)"]
        AAVE_WITHDRAW --> TX_LOG_WD["INSERT INTO transactions<br/>(tx_type = yield_withdraw,<br/>amount, tx_hash)"]
        WITHDRAW_CHECK -->|no| YIELD_DONE["No yield action needed"]

        HB_TICK --> YIELD_TRACK["Periodic: check aToken balance<br/>delta = current - last_known<br/>if delta > 0: INSERT INTO transactions<br/>(tx_type = yield_earned, amount = delta)"]
    end

    subgraph SpendControl["Spending Controls (ironclad-wallet/treasury.rs)"]
        FIN_TOOL["Financial tool call<br/>(transfer_credits, x402_fetch,<br/>topup_credits, spawn_child)"]
        FIN_TOOL --> POLICY_ENGINE["ironclad-agent/policy.rs<br/>Policy engine evaluation"]
        POLICY_ENGINE --> TREASURY_CHECK["TreasuryPolicy check:<br/>- per_payment_cap ($100)<br/>- hourly_transfer_limit ($500)<br/>- daily_transfer_limit ($2000)<br/>- minimum_reserve ($5)<br/>- daily_inference_budget ($50)"]
        TREASURY_CHECK --> SPEND_QUERY["Query transactions table<br/>(hourly + daily aggregates)"]
        SPEND_QUERY --> ALLOWED{"Within all<br/>limits?"}
        ALLOWED -->|yes| EXEC_FIN["Execute transaction"]
        ALLOWED -->|no| DENY_FIN["Deny + INSERT INTO<br/>policy_decisions<br/>(decision = deny,<br/>rule_name = treasury_policy)"]
    end
```

---

## 8. Cron + Heartbeat Unified Scheduling Dataflow

Unified scheduling system in `ironclad-schedule/`.

```mermaid
flowchart TD
    subgraph TickLoop["Tick Loop (ironclad-schedule/heartbeat.rs)"]
        TOKIO_INTERVAL["tokio::time::interval<br/>(configurable, default 60s)<br/>select! pattern (no overlap)"]
        TOKIO_INTERVAL --> BUILD_CTX["Build TickContext:<br/>- Fetch credit balance (once)<br/>- Fetch USDC balance (once)<br/>- Calculate SurvivalTier<br/>- Current timestamp"]
    end

    subgraph Evaluate["Job Evaluation (ironclad-schedule/scheduler.rs)"]
        BUILD_CTX --> QUERY_JOBS["SELECT FROM cron_jobs<br/>WHERE enabled = 1"]
        QUERY_JOBS --> FOR_EACH["For each job"]
        FOR_EACH --> SCHED_TYPE{"schedule_kind?"}

        SCHED_TYPE -->|cron| CRON_EVAL["Evaluate cron expression<br/>(schedule_expr + schedule_tz)"]
        SCHED_TYPE -->|every| INTERVAL_EVAL["Elapsed since last_run_at<br/>>= schedule_every_ms?"]
        SCHED_TYPE -->|at| AT_EVAL["now() >= schedule_expr<br/>(one-time fire)?"]

        CRON_EVAL & INTERVAL_EVAL & AT_EVAL --> IS_DUE{"Due?"}
        IS_DUE -->|no| SKIP_JOB["Skip"]
        IS_DUE -->|yes| LEASE{"Acquire DB lease<br/>(UPDATE cron_jobs SET<br/>lease_holder = instance_id<br/>WHERE lease_holder IS NULL<br/>OR lease_expired)"}
        LEASE -->|acquired| EXECUTE_JOB["Execute job"]
        LEASE -->|contended| SKIP_JOB
    end

    subgraph Execution["Job Execution (ironclad-schedule/tasks.rs)"]
        EXECUTE_JOB --> PAYLOAD_KIND{"payload_json.kind?"}
        PAYLOAD_KIND -->|agentTurn| AGENT_WAKE["Inject message into agent loop<br/>(payload.message,<br/>optional payload.model,<br/>payload.thinking on/off)"]
        PAYLOAD_KIND -->|systemEvent| SYS_EVENT["Process system event<br/>(payload.text)"]

        AGENT_WAKE --> SESSION_SELECT{"session_target?"}
        SESSION_SELECT -->|main| MAIN["Use main session<br/>(sessions table)"]
        SESSION_SELECT -->|isolated| ISO["Create isolated session<br/>(INSERT INTO sessions)"]

        MAIN & ISO --> RUN_REACT["Run ReAct loop turn<br/>(ironclad-agent/loop.rs)"]
    end

    subgraph Recording["State Recording"]
        RUN_REACT & SYS_EVENT --> UPDATE_JOB["UPDATE cron_jobs SET<br/>last_run_at = now(),<br/>last_status = ok/error,<br/>last_duration_ms = elapsed,<br/>consecutive_errors = 0 or +1,<br/>next_run_at = calculated,<br/>last_error = null or message,<br/>lease_holder = NULL"]
        UPDATE_JOB --> INSERT_RUN["INSERT INTO cron_runs<br/>(job_id, status,<br/>duration_ms, error)"]
    end

    subgraph Delivery["Result Delivery"]
        RUN_REACT --> DELIVERY_MODE{"delivery_mode?"}
        DELIVERY_MODE -->|none| SILENT["Silent"]
        DELIVERY_MODE -->|announce| DELIVER_MSG["Send via channel adapter<br/>(delivery_channel: telegram/whatsapp)"]
    end

    subgraph WakeSignal["In-Process Wake Signal"]
        RUN_REACT --> SHOULD_WAKE{"Task signals<br/>shouldWake?"}
        SHOULD_WAKE -->|yes| MPSC_SEND["tokio::sync::mpsc::send<br/>(WakeEvent to agent loop)"]
        SHOULD_WAKE -->|no| WAKE_DONE["No wake needed"]

        subgraph AgentSleep["Agent Sleep Loop"]
            SLEEP_SELECT["tokio::select!<br/>- mpsc::recv (wake event)<br/>- interval (30s poll fallback)"]
            MPSC_SEND --> SLEEP_SELECT
            SLEEP_SELECT --> AGENT_WAKES["Agent loop resumes"]
        end
    end
```

---

## 9. Skill Execution Dataflow

Dual-format extensibility system in `ironclad-agent/skills.rs` and `ironclad-agent/script_runner.rs`, with persistence in `ironclad-db/skills.rs`.

```mermaid
flowchart TD
    subgraph Loading ["Skill Loading (boot + hot-reload)"]
        SCAN["SkillLoader: scan skills_dir<br/>(skills.skills_dir config)"]
        SCAN --> FIND_TOML["Find .toml files<br/>(structured skills)"]
        SCAN --> FIND_MD["Find .md files<br/>(instruction skills)"]

        FIND_TOML --> PARSE_TOML["Parse TOML manifest:<br/>name, description, triggers,<br/>tool_chain, policy_overrides,<br/>script_path, risk_level"]
        FIND_MD --> PARSE_MD["Parse Markdown:<br/>YAML frontmatter (name,<br/>triggers, priority) +<br/>body (instructions)"]

        PARSE_TOML & PARSE_MD --> HASH["Compute SHA-256<br/>content_hash"]
        HASH --> CHANGED{"Hash changed<br/>since last_loaded_at?"}
        CHANGED -->|yes| UPSERT["Upsert into skills table<br/>(ironclad-db/skills.rs)"]
        CHANGED -->|no| SKIP["Skip (already loaded)"]
        UPSERT --> INDEX["Update SkillRegistry<br/>in-memory trigger index"]
    end

    subgraph Matching ["Skill Trigger Matching (per-turn)"]
        TURN_CTX["Turn context arrives<br/>(user message + active tools)"]
        TURN_CTX --> EVAL_TRIGGERS["SkillRegistry.match_skills():<br/>evaluate keyword, tool-name,<br/>and regex triggers"]
        EVAL_TRIGGERS --> MATCHED{"Skills matched?"}
        MATCHED -->|none| NO_SKILL["Continue without<br/>skill augmentation"]
        MATCHED -->|"1+"| SORT["Sort by priority<br/>(lower = higher priority)"]
    end

    subgraph ExecStructured ["Structured Skill Execution"]
        SORT -->|structured| CHAIN["StructuredSkillExecutor:<br/>iterate tool_chain steps"]
        CHAIN --> POLICY_OVERRIDE{"policy_overrides<br/>defined?"}
        POLICY_OVERRIDE -->|yes| APPLY_OVERRIDE["Temporarily apply<br/>policy adjustments"]
        POLICY_OVERRIDE -->|no| EXEC_CHAIN["Execute tool chain"]
        APPLY_OVERRIDE --> EXEC_CHAIN

        EXEC_CHAIN --> HAS_SCRIPT{"script_path<br/>defined?"}
        HAS_SCRIPT -->|yes| SCRIPT_EXEC["ScriptRunner.execute()"]
        HAS_SCRIPT -->|no| TOOL_EXEC["Execute via ToolRegistry"]
    end

    subgraph ExecInstruction ["Instruction Skill Execution"]
        SORT -->|instruction| INJECT_BODY["Inject .md body into<br/>system prompt context<br/>(via prompt.rs)"]
        INJECT_BODY --> LLM_HANDLES["LLM interprets<br/>natural-language instructions"]
    end

    subgraph ScriptSandbox ["Script Sandbox (script_runner.rs)"]
        SCRIPT_EXEC --> INTERPRETER_CHECK{"Interpreter in<br/>skills.allowed_interpreters?<br/>(bash, python3, node)"}
        INTERPRETER_CHECK -->|no| REJECT_SCRIPT["Reject: unlisted interpreter"]
        INTERPRETER_CHECK -->|yes| SPAWN["tokio::process::Command<br/>working dir = skill parent dir"]

        SPAWN --> ENV_CHECK{"skills.sandbox_env<br/>= true?"}
        ENV_CHECK -->|yes| STRIP_ENV["Strip env, pass only:<br/>PATH, HOME,<br/>IRONCLAD_SESSION_ID,<br/>IRONCLAD_AGENT_ID"]
        ENV_CHECK -->|no| FULL_ENV["Inherit full environment"]

        STRIP_ENV & FULL_ENV --> TIMEOUT["tokio::time::timeout<br/>(skills.script_timeout_seconds)"]
        TIMEOUT --> OUTPUT_CAP["Capture stdout/stderr<br/>truncate at<br/>skills.script_max_output_bytes"]
        OUTPUT_CAP --> RESULT["ScriptResult:<br/>stdout, stderr,<br/>exit_code, duration"]
    end

    subgraph PolicyGate ["Policy Gate"]
        SCRIPT_EXEC --> TOOL_RISK{"ScriptTool<br/>risk_level?"}
        TOOL_RISK --> POLICY_EVAL["Policy engine evaluates<br/>(record in policy_decisions)"]
        POLICY_EVAL --> ALLOWED{"Allowed?"}
        ALLOWED -->|yes| SPAWN
        ALLOWED -->|no| DENY["Deny script execution<br/>(audit log)"]
    end
```

---

## Self-Audit Results

### Correctness: Table References

| Diagram | Tables Referenced | All Exist in Schema? |
|---------|-------------------|---------------------|
| 1. Request | sessions, policy_decisions, semantic_cache, inference_costs, session_messages, turns, tool_calls | YES (all 7 in schema) |
| 2. Cache | semantic_cache, inference_costs | YES |
| 3. Router | inference_costs | YES |
| 4. Memory | working_memory, episodic_memory, semantic_memory, procedural_memory, relationship_memory, memory_fts | YES (all 6 in schema) |
| 5. A2A | relationship_memory, discovered_agents | YES -- `discovered_agents` table added to schema. **FIXED.** |
| 6. Injection | policy_decisions, relationship_memory, metric_snapshots | YES |
| 7. Financial | transactions, inference_costs | YES |
| 8. Scheduling | cron_jobs, cron_runs, sessions | YES |
| 9. Skills | skills, policy_decisions | YES |

**Tables not referenced by any diagram**: `schema_version` (infrastructure-only), `tasks`, `proxy_stats`, `identity`, `soul_history`. These are referenced by other subsystems (task management, observability, identity bootstrap, soul evolution) that are not dataflow-diagrammed here because they are straightforward CRUD. **Acceptable -- no fix needed.**

### Correctness: Crate References

| Diagram | Crates Referenced | All Exist in Layout? |
|---------|-------------------|---------------------|
| 1. Request | ironclad-channels, ironclad-db, ironclad-agent, ironclad-llm | YES |
| 2. Cache | ironclad-llm | YES |
| 3. Router | ironclad-llm | YES |
| 4. Memory | ironclad-agent, ironclad-db | YES |
| 5. A2A | ironclad-channels, ironclad-wallet | YES -- `a2a.rs` added to ironclad-channels layout. **FIXED.** |
| 6. Injection | ironclad-agent | YES |
| 7. Financial | ironclad-wallet, ironclad-schedule, ironclad-agent, ironclad-core | YES |
| 8. Scheduling | ironclad-schedule, ironclad-agent, ironclad-db | YES |
| 9. Skills | ironclad-agent, ironclad-db | YES |

**Crates not referenced**: `ironclad-server` (HTTP layer, dashboard, WS push). Not dataflow-diagrammed because it is the outer shell that dispatches to channel adapters and serves the dashboard. **Acceptable.**

### Correctness: Config Key References

| Diagram | Config Keys Referenced | All Exist in ironclad.toml? |
|---------|----------------------|---------------------------|
| 1. Request | (none directly -- uses crate-level config) | N/A |
| 2. Cache | cache.semantic_threshold, cache.exact_match_ttl_seconds, cache.max_entries | YES |
| 3. Router | models.routing.mode, models.routing.confidence_threshold, models.routing.local_first, models.primary, models.fallbacks | YES |
| 4. Memory | memory.working_budget_pct, memory.episodic_budget_pct, memory.semantic_budget_pct, memory.procedural_budget_pct, memory.relationship_budget_pct | YES |
| 5. A2A | wallet.chain_id, wallet.rpc_url, a2a.max_message_size, a2a.rate_limit_per_peer | YES -- `[a2a]` config section added. **FIXED.** |
| 6. Injection | (none directly -- hardcoded thresholds, session_secret is runtime-generated) | N/A -- `hmac_session_secret` documented in `identity` table comments. **FIXED.** |
| 7. Financial | yield.enabled, yield.min_deposit, yield.withdrawal_threshold, yield.protocol, treasury.minimum_reserve, treasury.per_payment_cap, treasury.hourly_transfer_limit, treasury.daily_transfer_limit, treasury.daily_inference_budget, wallet.rpc_url, wallet.path | YES |
| 8. Scheduling | (cron_jobs in DB, not config) | N/A |
| 9. Skills | skills.skills_dir, skills.script_timeout_seconds, skills.script_max_output_bytes, skills.allowed_interpreters, skills.sandbox_env | YES |

### Completeness: Ironclad Differentiators

| Differentiator | Diagrammed? |
|---------------|-------------|
| Semantic cache | Diagram 2 |
| ML model routing | Diagram 3 |
| Yield engine | Diagram 7 |
| Zero-trust A2A | Diagram 5 |
| Multi-layer injection defense | Diagram 6 |
| Unified SQLite DB | All diagrams reference SQLite tables |
| In-process routing (no IPC) | Diagram 1 (no proxy boundary) |
| Progressive context loading | Diagram 1 (Level 0-3) |
| HMAC trust boundaries | Diagram 6 Layer 2 |
| Connection pooling | Diagram 1 (reqwest::Client pool) |
| Dual-format skill system | Diagram 9 |
| Sandboxed script execution | Diagram 9 |

**All 12 differentiators represented. PASS.**

### Completeness: Error/Failure Paths

| Diagram | Error Paths Shown |
|---------|-------------------|
| 1. Request | Injection block, cache miss, circuit breaker block, dedup reject, fallback exhaustion, policy deny |
| 2. Cache | All 3 miss paths, eviction |
| 3. Router | Provider unavailable, T1 quality failure, escalation, all providers exhausted |
| 4. Memory | (no error paths -- CRUD is infallible against SQLite) -- **MINOR GAP**: should show DB write failure path |
| 5. A2A | Stale timestamp reject (both sides), signature verification failure implied by reject |
| 6. Injection | Block, sanitize, deny paths for all 4 layers |
| 7. Financial | Policy deny, insufficient balance (implicit in USDC check) |
| 8. Scheduling | Lease contention (skip), error status recording |
| 9. Skills | Unlisted interpreter reject, policy deny for script execution, script timeout, no matching skills |

### Consistency Check

- `ApiFormat` enum name used consistently (Diagram 1 only, correct)
- Table names match schema exactly in all diagrams (verified above)
- Config key names match `ironclad.toml` exactly (all gaps resolved)
- Crate boundaries match dependency graph (verified -- no diagram has a crate calling another crate not in its dependency list)

---

## Fixes Applied (to ironclad-design.md and C4 docs)

All 12 inconsistencies identified across two audit passes have been resolved:

**Round 1 (A2A + scheduling fixes):**
1. **Added `discovered_agents` table** to schema (for A2A agent card caching) -- DONE
2. **Added `a2a.rs`** to `ironclad-channels` crate file listing -- DONE
3. **Added `[a2a]` config section** to `ironclad.toml` (max_message_size, rate_limit_per_peer, session_timeout_seconds, require_on_chain_identity) -- DONE
4. **Documented HMAC session secrets** in `identity` table comments (hmac_session_secret generated on first boot, a2a_identity_key derived from wallet) -- DONE
5. **Added `lease_holder` and `lease_expires_at` columns** to `cron_jobs` table -- DONE
6. **Added `yield_earned` to `transactions.tx_type` comment** -- DONE

**Round 2 (skill system integration):**
7. **Added `skills` table** to schema in ironclad-design.md (dual-format skill definitions with triggers, tool chains, script paths) -- DONE
8. **Added `[skills]` config section** to ironclad.toml (skills_dir, script_timeout_seconds, allowed_interpreters, sandbox_env, hot_reload) -- DONE
9. **Added `skills.rs` + `script_runner.rs`** to ironclad-agent crate layout + `skills.rs` to ironclad-db crate layout -- DONE
10. **Added skill types** (`SkillKind`, `SkillTrigger`, `SkillManifest`, `InstructionSkill`) to types.rs in ironclad-design.md -- DONE
11. **Updated all C4 docs**: ironclad-c4-core (SkillsConfig, skill types, Skill error variant), ironclad-c4-db (25 tables, skills.rs module), ironclad-c4-agent (skills.rs, script_runner.rs modules), ironclad-c4-server (12-step bootstrap, skills API routes), ironclad-c4-container (agent description, skills table) -- DONE
12. **Added Diagram 9** (Skill Execution Dataflow) to this document with self-audit entries and test surface map -- DONE

---

## Test Surface Map (90% Unit Test Coverage Target)

### Diagram 1 -- Primary Request

| Crate | Module | Functions to Test | Mock Strategy |
|-------|--------|-------------------|---------------|
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
| ironclad-llm/cache.rs | `exact_lookup()`, `semantic_lookup()`, `tool_cache_lookup()`, `store()`, `evict_expired()`, `evict_lru()`, `compute_hash()` | In-memory SQLite, mock ONNX embedding (return fixed vectors) |

### Diagram 3 -- ML Router

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
| ironclad-llm/router.rs | `extract_features()`, `classify_complexity()`, `select_model()`, `fallback_next()`, `check_t1_quality()` | Mock ONNX runtime (return fixed scores), mock circuit breaker state |

### Diagram 4 -- Memory

| Module | Functions to Test | Mock Strategy |
|--------|-------------------|---------------|
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
