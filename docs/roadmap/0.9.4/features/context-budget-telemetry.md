# Context Budget Telemetry

## Objective
Expose prompt-budget composition by segment (personality, memory, history, tools) each turn.

## Scope
- Define implementation constraints and decision boundaries.
- Define telemetry and observability expectations.
- Define required automated tests for merge.

## Acceptance Criteria
- Turn records include per-segment token estimates.\n- Dashboard/API can display budget composition for diagnostics.\n- Telemetry overhead remains within defined limits.

## Out Of Scope
- Runtime feature implementation.
- Refactors unrelated to this feature.
- Release process changes outside 0.9.4 behavior corrections.

## Test Gate (Required For Implementation PR)
- Unit tests for new logic.
- Integration tests for API/channel behavior.
- Soak scenario showing expected behavior and no regressions.
