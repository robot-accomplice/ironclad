# Regression Test Policy

This policy defines how Ironclad prevents repeat defects from reaching release tags.

## Required for Every Bugfix PR

- Add or update at least one regression test that reproduces the bug class.
- Map the test to an ID in `docs/testing/regression-matrix.md`.
- Place the test in the closest layer to the bug:
  - `L1` unit test for deterministic logic regressions
  - `L2` integration test for cross-crate behavior regressions
  - `L3` release smoke test for high-risk operator-critical flows

## Regression Battery Gate

- Canonical command: `just test-regression`
- PR CI: regression battery runs as a required status check.
- Release tags: regression battery is required and blocking.

## v0.8.0 Go-Live Strict Gate

For `v0.8.0`, treat regression prevention as release-critical:

- Canonical release command: `just test-v080-go-live`
- The following are all blocking on release tags:
  - `just test-release-critical` (workspace + integration + regression battery)
  - `just test-soak-fuzz` (bounded soak and fuzz battery)
  - `scripts/run-uat-stack.sh` (CLI + web UAT smoke)
  - `just test-release-doc-gate` (docs/artifact/provenance consistency)
- No release job in this chain may run as informational-only.

## Naming Convention

- Test names should include behavior intent and regression class.
- Preferred pattern:
  - `regression_<subsystem>_<behavior>`
  - or explicit behavior names with a linked matrix ID in code comments.

## Change Management

- When a new regression class appears in production or release candidate:
  1. Add a row to `docs/testing/regression-matrix.md`.
  2. Add at least one deterministic test.
  3. Ensure `just test-regression` includes it if it is release-critical.

## Flake Handling

- Flaky tests must not silently remain in the release battery.
- If a flaky test is quarantined:
  - mark it clearly in the test file and matrix entry,
  - open a follow-up fix task,
  - remove quarantine before the next minor release cut.
- For `v0.8.0`, flaky tests in any blocking gate are a release stopper.
