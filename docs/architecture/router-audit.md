# Router Dataflow and Audit

This document maps current model-router behavior, defines intended routing sequences, and audits implementation paths against intended behavior.

## Current Router Dataflow (As Implemented)

```mermaid
flowchart TD
  subgraph Entry["Inference Entry Paths"]
    E1["agent_message()"]
    E2["agent_message_stream()"]
    E3["process_channel_message()"]
    E4["interview_turn()"]
  end

  subgraph Select["Model Selection"]
    S1["extract_features() + classify_complexity()"]
    S2["select_for_complexity()\nOR\nselect_cheapest_qualified()"]
    S3["model override check"]
    S4["local_first threshold check"]
    S5["breaker/capacity filtering"]
  end

  subgraph Exec["Execution"]
    X1["infer_with_fallback()\n(candidate loop)"]
    X2["single provider call\n(stream/interview paths)"]
  end

  E1 --> S1
  E3 --> S1
  S1 --> S2
  S2 --> S3
  S3 --> S4
  S4 --> S5
  S5 --> X1

  E2 --> S1
  E2 --> X2

  E4 --> X2
```

## Intended Sequence Diagrams

### 1) Complexity-Aware Routing (Non-Cost-Aware)

```mermaid
sequenceDiagram
  participant Req as Request
  participant Rt as ModelRouter
  participant Br as CircuitBreakerRegistry
  participant Cap as CapacityTracker

  Req->>Rt: select_for_complexity(score, registry, cap, breakers)
  Rt->>Rt: override set?
  alt override exists
    Rt-->>Req: override model
  else no override
    Rt->>Br: blocked(primary provider)?
    Rt->>Cap: near-capacity(primary provider)?
    alt local_first && score < threshold && primary usable
      Rt-->>Req: primary
    else
      Rt-->>Req: fallback[0] or next unblocked/capable fallback
    end
  end
```

### 2) Cost-Aware Routing

```mermaid
sequenceDiagram
  participant Req as Request
  participant Rt as ModelRouter
  participant Reg as ProviderRegistry
  participant Br as CircuitBreakerRegistry
  participant Cap as CapacityTracker

  Req->>Rt: select_cheapest_qualified(...)
  Rt->>Rt: build candidate set (primary + fallbacks)
  Rt->>Br: remove blocked providers
  Rt->>Cap: remove near-capacity providers
  alt complexity >= threshold
    Rt->>Reg: keep cloud subset
    Rt-->>Req: cheapest cloud candidate
  else low complexity
    Rt-->>Req: cheapest remaining candidate
  end
```

### 3) Bounded Fallback Execution

```mermaid
sequenceDiagram
  participant S as infer_with_fallback()
  participant Cfg as Config
  participant Br as Breakers
  participant P as Provider

  S->>Cfg: candidates = [initial_model] + fallbacks
  loop each candidate in order
    S->>Br: is_blocked(provider)?
    alt blocked
      S-->>S: skip candidate
    else not blocked
      S->>P: call provider
      alt success
        S->>Br: record_success(provider)
        S-->>S: return response
      else error
        S->>Br: record_failure/record_credit_error(provider)
        S-->>S: continue
      end
    end
  end
  S-->>S: return final error
```

## Audit: Code vs Intended Behavior

### Pass

- Router supports three selection modes: `primary`, `round-robin`, and complexity-aware default.
- Complexity-aware path applies `local_first`, breaker filtering, and capacity filtering.
- Cost-aware path applies breaker and capacity pruning before cost choice.
- Runtime model override (`set_override`) cleanly short-circuits both selection modes.
- Non-stream chat/channel paths run bounded candidate loop via `infer_with_fallback`.

### Mismatches / Risks

1. **Route-family inconsistency**
   - `agent_message()` and channel processing use full fallback loop.
   - `agent_message_stream()` and `interview_turn()` use single-provider calls.
   - This creates behavior drift between API surfaces.

2. **Config-vs-router drift risk**
   - Runtime config mutations can update config structures while active `ModelRouter` internals (`primary`, `fallbacks`, `override`) remain independently stateful.
   - Requires explicit synchronization guarantees per mutation path.

3. **Override observability gap**
   - Override can be set via chat command; status visibility is command/UI dependent.
   - Without explicit audit events, operator may not realize override is active.

4. **Unused `model_overrides` config map**
   - `models.model_overrides` exists in config schema/docs but router selection paths do not currently consume it.

## Files Audited

- `crates/ironclad-llm/src/router.rs`
- `crates/ironclad-server/src/api/routes/agent.rs`
- `crates/ironclad-server/src/api/routes/interview.rs`
- `crates/ironclad-core/src/config.rs`

## Router Test Objectives

1. Verify local-first selection for low complexity with healthy primary.
2. Verify blocked provider skip logic picks next eligible fallback.
3. Verify cost-aware routing selects cheapest eligible candidate.
4. Verify override short-circuits routing and clear restores normal behavior.

## Path Coverage Matrix (Integration Test Set)

- `E1 -> S1 -> S2 -> S3 -> S4 -> S5 -> X1`  
  - `server_api::fallback_chain_is_bounded_to_configured_candidates`
- `E2 -> S1 -> X2`  
  - `server_api::stream_path_uses_bounded_fallback_surface`
- `E3 -> webhook -> process_channel_message`  
  - `server_api::telegram_webhook_public_entrypoint_accepts_and_returns_ok`
- `E3 -> webhook slash payload -> command handling branch`  
  - `server_api::telegram_webhook_public_entrypoint_accepts_slash_command_payload`
- `E4 -> X2`  
  - `server_api::interview_path_uses_shared_fallback_surface`

Router behavior integration set:

- `router_integration::router_local_first_prefers_primary_for_low_complexity`
- `router_integration::router_skips_blocked_first_choice`
- `router_integration::router_cost_aware_chooses_cheapest_eligible`
- `router_integration::router_override_short_circuits_then_clear_restores`
