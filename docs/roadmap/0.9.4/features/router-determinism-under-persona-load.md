# Router Determinism Under Persona Load

## Objective
Guarantee pinned model requests remain pinned when fallback is disabled, including persona-seeded prompts.

## Scope
- Define implementation constraints and decision boundaries.
- Define telemetry and observability expectations.
- Define required automated tests for merge.

## Acceptance Criteria
- Pinned+no-fallback requests never drift to a different actual model.
- Routing audit clearly indicates deterministic selection path.
- Failing providers surface explicit error instead of silent model drift.

## Out Of Scope
- Runtime feature implementation.
- Refactors unrelated to this feature.
- Release process changes outside 0.9.4 behavior corrections.

## Test Gate (Required For Implementation PR)
- Unit tests for new logic.
- Integration tests for API/channel behavior.
- Soak scenario showing expected behavior and no regressions.
