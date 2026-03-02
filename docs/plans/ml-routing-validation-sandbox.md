# ML Routing Validation Sandbox Plan

## Goal

Validate whether ML-based routing provides **empirical** improvement over current heuristic/metascore routing before any product shipment.

## Branch

- Working branch: `codex/ml-routing-validation-sandbox`
- ML work stays off release branches until promotion criteria pass.

## Sandbox Design

### Data Source

- Use unidirectional sync from local installed instance telemetry into harness fixtures.
- Sync direction is strictly: local runtime -> validation sandbox.
- No writes from sandbox back to production runtime.

### Harness Modes

1. **Replay mode**
- Re-run historical turns through:
  - baseline heuristic/metascore router
  - candidate ML router
- Produce comparable outputs and decision traces.

2. **Shadow mode**
- In live operation, execute baseline router.
- ML router predicts in parallel and logs counterfactual choice only.
- No user-visible behavior changes.

3. **Canary mode (optional, later)**
- Controlled traffic split after shadow success.
- Immediate rollback to baseline on regression thresholds.

## Evaluation Metrics

Primary:
- quality proxy delta (from outcome signals)
- cost per successful turn
- latency impact
- escalation/fallback frequency

Secondary:
- policy/breaker constraint violation attempts (should remain zero)
- decision stability under similar inputs
- cold-start degradation vs baseline

## Promotion Criteria

ML can be considered for shipment only if all hold over a sustained window:
- statistically significant non-negative quality delta
- no material cost regression beyond agreed threshold
- no latency regression beyond agreed threshold
- zero safety/policy constraint bypasses
- reproducible results across replay + shadow windows

## Operational Guardrails

- Hard kill switch at config/runtime level
- Mandatory decision attribution (`baseline`, `ml-shadow`, `ml-canary`)
- Feature-schema version checks on model artifact load
- Automatic fallback to baseline on load/eval failures

## Deliverables

1. Harness sync adapter for local telemetry ingestion (unidirectional)
2. Replay/shadow evaluation reports
3. Weekly longitudinal performance summaries
4. Promotion/no-promotion recommendation with raw evidence

## Non-Goals

- Shipping ML routing to default product path during validation period
- Replacing existing metascore heuristics before criteria are met
