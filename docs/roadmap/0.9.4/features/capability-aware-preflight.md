# Capability-Aware Preflight

## Objective
Classify models by callable capability (chat/tool/embedding-only) before soak and routing experiments.

## Scope
- Define implementation constraints and decision boundaries.
- Define telemetry and observability expectations.
- Define required automated tests for merge.

## Acceptance Criteria
- Embedding-only models are excluded from chat soak runs.
- Preflight output distinguishes unreachable vs unsupported capability.
- Matrix reports include capability class column.

## Out Of Scope
- Runtime feature implementation.
- Refactors unrelated to this feature.
- Release process changes outside 0.9.4 behavior corrections.

## Test Gate (Required For Implementation PR)
- Unit tests for new logic.
- Integration tests for API/channel behavior.
- Soak scenario showing expected behavior and no regressions.
