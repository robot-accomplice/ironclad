# C4 Level 3: Component Diagram -- ironclad-llm

*LLM client layer handling all inference requests: connection pooling, format translation, ML-based model routing, semantic caching, circuit breaking, deduplication, and tier-based prompt adaptation.*

---

## Component Diagram

```mermaid
flowchart TB
    subgraph IroncladLlm ["ironclad-llm"]
        CLIENT["client.rs<br/>HTTP Client Pool"]
        FORMAT["format.rs<br/>API Format Translation"]
        PROVIDER["provider.rs<br/>Provider Definitions"]
        CIRCUIT["circuit.rs<br/>Circuit Breaker"]
        DEDUP["dedup.rs<br/>In-Flight Dedup"]
        TIER["tier.rs<br/>Tier Classification +<br/>Prompt Adaptation"]
        ROUTER["router.rs<br/>ML Model Router"]
        CACHE["cache.rs<br/>Semantic Cache"]
    end

    subgraph ClientDetail ["client.rs"]
        POOL["Persistent reqwest::Client<br/>HTTP/2 where supported<br/>Connection reuse across requests"]
        FORWARD["forward_request()<br/>Send to provider, stream response"]
        RESP_PROC["process_response()<br/>Breaker update, format<br/>back-translation, cost tracking"]
    end

    subgraph FormatDetail ["format.rs"]
        FMT_ENUM["ApiFormat enum<br/>(4 variants, From trait impls)"]
        DETECT_REQ["detect_request_format()"]
        DETECT_RESP["detect_response_format()"]
        XLATE_REQ["translate_request()<br/>12+ translator pairs"]
        XLATE_RESP["translate_response()<br/>7+ translator pairs"]
    end

    subgraph RouterDetail ["router.rs"]
        RULE_MODE["Rule mode:<br/>static fallback chain<br/>(models.primary, models.fallbacks)"]
        ML_MODE["ML mode:<br/>extract features -> ONNX<br/>classifier (~11us) -><br/>complexity score 0.0-1.0"]
        FEATURES["Feature extraction:<br/>message length, tool_call count,<br/>conversation depth, keyword signals"]
        CONFIDENCE["Confidence check:<br/>if T1 response quality poor,<br/>escalate to next tier"]
        FALLBACK["Fallback chain:<br/>on 429/5xx/timeout,<br/>advance to next model"]
    end

    subgraph CacheDetail ["cache.rs"]
        L1["Level 1: Exact hash lookup<br/>(SHA-256 of system + conv + msg)"]
        L2["Level 2: Semantic embedding<br/>(ONNX all-MiniLM-L6-v2, ~5ms)<br/>cosine > cache.semantic_threshold"]
        L3["Level 3: Deterministic tool<br/>result TTL cache"]
        STORE["Store: prompt_hash + embedding<br/>+ response + expires_at"]
        EVICT["Eviction: expire by TTL,<br/>LRU by hit_count when<br/>count > cache.max_entries"]
    end

    subgraph CircuitDetail ["circuit.rs"]
        BREAKER_STATE["Per-provider state:<br/>Closed, Open, HalfOpen"]
        RATE_TRIP["Rate trip:<br/>threshold hits in window_seconds"]
        CREDIT_TRIP["Credit trip:<br/>immediate on 401/402/403"]
        BACKOFF["Exponential backoff:<br/>cooldown -> 2x -> max_cooldown"]
        RECOVER["Recovery:<br/>HalfOpen allows 1 request,<br/>success -> Closed"]
    end

    subgraph DedupDetail ["dedup.rs"]
        FINGERPRINT["fingerprint():<br/>SHA-256 of provider + model +<br/>msg_count + system[:200] +<br/>user[:500]"]
        TRACK["check_and_track():<br/>if fingerprint in-flight,<br/>reject (configurable: warn/block)"]
        RELEASE["release():<br/>remove fingerprint after response"]
        TTL_EVICT["TTL eviction: 120s"]
    end

    subgraph TierDetail ["tier.rs"]
        CLASSIFY["classify(model_name) -> ModelTier"]
        ADAPT_T1["adapt_t1(): condensed prompt,<br/>strip non-essential sections"]
        ADAPT_T2["adapt_t2(): add preamble,<br/>reorder sections for context"]
        ADAPT_T3T4["adapt_t3t4(): passthrough,<br/>inject cache_control headers"]
    end

    ROUTER --> CIRCUIT
    ROUTER --> CLIENT
    CLIENT --> DEDUP
    CLIENT --> FORWARD
    FORWARD --> RESP_PROC
    CACHE --> L1 --> L2 --> L3
```

## Request Pipeline (in order)

1. **Cache check** (`cache.rs`) -- 3-level lookup, return on hit
2. **ML routing** (`router.rs`) -- classify complexity, select model + provider
3. **Circuit breaker** (`circuit.rs`) -- check provider availability
4. **Dedup** (`dedup.rs`) -- reject duplicate in-flight requests
5. **Format translation** (`format.rs`) -- translate request to provider's API format
6. **Tier adaptation** (`tier.rs`) -- adapt prompt for model tier
7. **Forward** (`client.rs`) -- send via persistent connection pool
8. **Response processing** (`client.rs`) -- back-translate format, update breaker, record cost
9. **Cache store** (`cache.rs`) -- store response for future hits

## Dependencies

**External crates**: `reqwest` (HTTP client), `ort` (ONNX runtime for ML router + embeddings), `sha2` (hashing)

**Internal crates**: `ironclad-core` (types, config, errors)

**Depended on by**: `ironclad-agent`, `ironclad-server`
