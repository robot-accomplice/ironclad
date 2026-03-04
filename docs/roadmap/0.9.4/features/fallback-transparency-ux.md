# Fallback Transparency UX

## Objective
Emit one consolidated model-shift notification with final selected model, root reason, and latency/failure summary.

## Scope
- Define implementation constraints and decision boundaries.
- Define telemetry and observability expectations.
- Define required automated tests for merge.

## Acceptance Criteria
- At most one user-facing switch notice per request.
- Notice includes final model and concise reason category.
- Internal logs include full hop trace and timings.

## Out Of Scope
- Runtime feature implementation.
- Refactors unrelated to this feature.
- Release process changes outside 0.9.4 behavior corrections.

## Test Gate (Required For Implementation PR)
- Unit tests for new logic.
- Integration tests for API/channel behavior.
- Soak scenario showing expected behavior and no regressions.
