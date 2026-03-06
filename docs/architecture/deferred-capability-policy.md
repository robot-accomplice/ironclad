# Deferred Capability Policy

## Purpose

Prevent scope drift and vestigial integrations during active release execution.

This policy governs any capability that is intentionally postponed (deferred) so teams do not partially wire, partially test, or partially ship features that are out of scope for the current release.

## What Counts as a Deferred Capability

A capability is deferred when any of the following is true:

- It is not in the active release checklist scope.
- It lacks a named owner for implementation and operations.
- It lacks release-grade tests for correctness, failure handling, and rollback.
- It cannot be reverted cleanly if runtime behavior regresses.

## Mandatory Reintroduction Criteria

A deferred capability can only be reintroduced when all criteria are met:

1. Clear business/runtime objective tied to release scope.
2. Named owner accountable for delivery and runtime behavior.
3. Explicit test plan (happy path, failure path, idempotency/recovery).
4. Rollback plan with operational trigger conditions.

## Guardrails

- No partial or vestigial integration in active release scope.
- No hidden feature flags as a substitute for acceptance criteria.
- No runtime-visible artifacts (DB rows, config knobs, UI controls, CLI commands) for deferred items unless intentionally marked as deferred and non-operational.
- Any active capability must map to at least one built-in skill (or have an explicit documented exception).

## Operationalization

- Every release checklist must link to this policy.
- Deferred items must be explicitly tracked as deferred in roadmap/release docs.
- Mechanic checks may enforce deferred-state hygiene (cleanup/reconcile) where applicable.

## Current Deferred Record

- **Conway integration**: deferred and intentionally out of active implementation scope.
