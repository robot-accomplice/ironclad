# C4 Level 3: Component Diagram -- ironclad-llm

*LLM client layer: HTTP client (reqwest), provider translation (UnifiedRequest/UnifiedResponse), **heuristic** complexity classification and model routing, **in-memory** semantic cache (HashMap), circuit breaker, and deduplication. No ONNX or ML models.*

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
        ROUTER["router.rs<br/>Heuristic Model Router"]
        CACHE["cache.rs<br/>In-Memory SemanticCache<br/>(HashMap)"]
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
        HEURISTIC["Heuristic classifier (RouterBackend):<br/>HeuristicBackend weights message length,<br/>tool_call count, conversation depth<br/>-> complexity score 0.0-1.0"]
        MODES["Routing modes: primary, round-robin,<br/>heuristic/ml (default 'heuristic';<br/>'ml' is backward-compat alias)"]
        FEATURES["extract_features(): msg len, tool_calls, depth<br/>classify_complexity(features) -> f64"]
        SELECT["select_for_complexity(): local_first +<br/>confidence_threshold -> primary or fallback"]
        FALLBACK["advance_fallback(), reset()<br/>on 429/5xx/timeout"]
    end

    subgraph CacheDetail ["cache.rs - In-Memory Only"]
        L1["L1: lookup_exact(prompt_hash)<br/>HashMap key = SHA-256(system|msgs|user_msg)"]
        L2["L2: lookup_semantic(prompt)<br/>char n-gram embedding, cosine > threshold"]
        L3["L3: lookup_tool_ttl(prompt_hash)<br/>shorter TTL for tool-involved entries"]
        STORE["store() / store_with_embedding()<br/>evict_lfu() at max_entries"]
        EVICT["evict_expired() by Instant"]
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

1. **Cache check** (`cache.rs`) — in-memory 3-level lookup (exact hash → tool TTL → semantic n-gram), return on hit
2. **Routing** (`router.rs`) — heuristic `classify_complexity(features)`; `select_for_complexity()` with optional `ProviderRegistry` for `is_local`
3. **Circuit breaker** (`circuit.rs`) — per-provider state (Closed/Open/HalfOpen), configurable threshold/window/cooldown
4. **Dedup** (`dedup.rs`) — in-flight duplicate detection
5. **Format translation** (`format.rs`) — `translate_request(UnifiedRequest, ApiFormat)`, `translate_response(Value, ApiFormat)` → `UnifiedResponse`
6. **Tier adaptation** (`tier.rs`) — tier-based prompt adaptation (T1 strip/condense, T2 preamble, T3/T4 passthrough)
7. **Forward** (`client.rs`) — `forward_request` / `forward_with_provider` (reqwest POST, auth + extra headers)
8. **Response** — back-translate, update breaker, record cost
9. **Cache store** (`cache.rs`) — `store` or `store_with_embedding` in HashMap

## Dependencies

**External crates**: `reqwest` (HTTP client), `sha2` (hashing). **No ONNX or ML runtime.**

**Internal crates**: `ironclad-core` (types, config, errors)

**Depended on by**: `ironclad-agent`, `ironclad-server`
