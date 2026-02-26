# v0.8.0 Stabilization Design: 95% Coverage, Architecture Audit, Bug Discovery & Iterative Fix

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

## Overview

This document defines the design for lifting the v0.8.0 feature freeze. The freeze is **hard** — no
feature work anywhere until all 11 crates reach 95% test coverage and every discovered bug is fixed
and verified.

Four interconnected workstreams execute in a **tiered convergence** strategy with **inside-out
validation**:

1. **Architecture Audit + Wiring Validation** — verify every diagram and every crate-to-crate
   contract against v0.8.0 code
2. **Bug Discovery Program** — automated-first, 8-layer discovery feeding a single bug ledger
3. **Path to 95% Coverage** — unit, integration, and regression coverage ramped inside-out by
   dependency tier
4. **Iterative Fix & Retest** — fix inside-out, retest outside-in, batch by tier

### Strategy: Tiered Convergence

Architecture audit is global (diagrams span crates). Per-crate bug discovery and coverage work
parallelize within dependency tiers. This prevents "fix crate A, break crate B" churn while being
faster than fully sequential execution.

### Principle: Inside-Out Validation

Validation proceeds from the innermost dependency (ironclad-core) outward. Each layer's wiring is
validated before the next layer is tested. If `db` isn't correctly implementing the contracts `agent`
expects, no amount of server-level testing will find the real bug.

### Dependency Tiers

```
Tier 0: ironclad-core          (no internal deps — foundation)
Tier 1: ironclad-db             (depends on core)
Tier 2: ironclad-llm            (depends on core, db)
Tier 3: ironclad-agent          (depends on core, db, llm)
Tier 4: ironclad-wallet         (depends on core, db)
        ironclad-channels       (depends on core, db, agent)
        ironclad-schedule       (depends on core, db)
        ironclad-plugin-sdk     (depends on core)
        ironclad-browser        (depends on core)
Tier 5: ironclad-server         (depends on everything)
Tier 6: ironclad-tests          (integration validation of full stack)
```

---

## Phase 1: Architecture Audit + Inside-Out Wiring Validation

### 1A: Full Diagram Audit

**Scope**: Every Mermaid diagram in `docs/architecture/` (50+ diagrams across 17 files) checked
against current v0.8.0 code. Diagrams were last updated at v0.5.0-v0.6.0 — the codebase has 80+
changes since then including new subsystems (durable delivery queue, abuse protection, skills
catalog).

**Method** (automated-first):
- Parse each Mermaid diagram programmatically to extract declared components, relationships, and
  data flows
- Cross-reference against actual `pub` exports, `use` imports, trait implementations, and function
  call graphs in the code
- Flag every delta: missing components, phantom components (in diagram but not in code), missing
  relationships, reversed data flows, renamed types

**Deliverable**: `docs/audit/architecture-drift-report.md`

Every discrepancy is categorized as:

| Category | Description |
|----------|-------------|
| **Structural drift** | Component exists in diagram but not code (or vice versa) |
| **Relationship drift** | Connection shown but not implemented (or exists but undocumented) |
| **Behavioral drift** | Flow described doesn't match actual execution path |
| **Naming drift** | Types/functions renamed but diagram not updated |

### 1B: Inside-Out Wiring Validation

For each dependency edge in the crate graph, validate that the contract between producer and
consumer is correct.

**Per-edge validation checklist**:

1. **Trait contracts**: Does the consumer use traits from the producer? Are all required methods
   implemented? Do the type signatures match expectations?
2. **Error propagation**: Does the consumer correctly handle all error variants the producer can
   return?
3. **State assumptions**: Does the consumer assume initialization order, database schema, or config
   fields that the producer guarantees?
4. **Concurrency contracts**: If the producer hands out `Arc<Mutex<T>>` or channels, does the
   consumer respect the locking protocol?
5. **Lifecycle coupling**: Does the consumer depend on the producer's startup/shutdown order?

**Deliverable**: `docs/audit/wiring-validation-report.md` — per-edge assessment with pass/fail/
concern status and specific code references.

### 1C: Remediation

All drift and wiring issues are fixed before proceeding:
- Diagrams updated to match code (or code fixed if the diagram represents the *intended* design)
- Broken wiring patched with regression tests
- Each fix committed with conventional commit (`fix:` or `docs:`) and mapped to the regression
  matrix

---

## Phase 2: Bug Discovery Program (Automated-First)

All discovery techniques run in parallel, feeding into a single bug ledger. Discovery builds the
complete defect inventory before fix work begins.

### 2A: Discovery Techniques (8 layers)

#### Layer 1 — Static Analysis (zero-runtime)

- **Clippy hardening**: Run with `clippy::pedantic`, `clippy::nursery`, and `clippy::cargo` lint
  groups selectively to catch subtle issues (unreachable patterns, missing `Send` bounds,
  unnecessary allocations)
- **`cargo audit --deny warnings`**: Catch yanked crates and unmaintained dependencies
- **`cargo deny`**: License and duplicate-dependency checking
- **`cargo semver-checks`**: Verify v0.8.0 public API hasn't broken semver guarantees vs v0.7.1
- **Custom lint pass**: Grep-based checks for known anti-patterns (`unwrap` in non-test code,
  `todo!()` in non-test paths, unbounded `Vec::push` in loops, missing `Drop` implementations on
  resource-holding types)

#### Layer 2 — Coverage Gap Analysis

- Run `cargo llvm-cov --show-missing-lines` per-crate
- Cross-reference uncovered lines against function complexity (high-complexity + zero-coverage =
  high-risk)
- Prioritize uncovered `match` arms, error handlers, and `unsafe` blocks
- Output: ranked list of uncovered code regions by risk score

#### Layer 3 — Expanded Fuzz Testing

Expand from 2 fuzz targets to cover all parsing and deserialization boundaries:

| Target | Crate | Input |
|--------|-------|-------|
| Config parsing | ironclad-core | Arbitrary TOML |
| DB query construction | ironclad-db | Arbitrary session/memory inputs |
| LLM response parsing | ironclad-llm | Malformed API responses |
| Channel message deserialization | ironclad-channels | Arbitrary webhook payloads |
| Plugin manifest parsing | ironclad-plugin-sdk | Arbitrary TOML manifests |
| Cron expression parsing | ironclad-schedule | Arbitrary cron strings |
| Injection check (existing) | ironclad-agent | Arbitrary strings |
| Output scan (existing) | ironclad-agent | Arbitrary strings |

Each target runs for minimum 5 minutes in CI (configurable longer locally). Crashes and hangs are
automatically filed to the bug ledger.

#### Layer 4 — Mutation Testing

- Integrate `cargo-mutants` to systematically mutate code and check if tests catch the mutation
- Run per-crate starting from Tier 0 (core) outward
- Surviving mutants indicate: (a) missing test coverage, or (b) dead code
- Each surviving mutant in non-trivial code is a bug ledger entry (category: "weak test" or
  "dead code")

#### Layer 5 — Property-Based Test Expansion

Expand from 2 proptest usages to cover:

- All serialization roundtrips (config, session state, memory entries, cron specs)
- Idempotency invariants (delivery queue dedup, session create-or-find)
- Monotonicity invariants (sequence IDs, timestamps never go backward)
- Commutativity where expected (e.g., memory merge operations)

#### Layer 6 — Integration Fault Injection

- Simulate failures at crate boundaries: database errors, LLM timeouts, channel delivery failures,
  wallet RPC errors
- Verify each crate degrades gracefully rather than panicking or corrupting state
- Uses the wiring validation from Phase 1B to target the highest-risk edges

#### Layer 7 — CLI/Web UI Behavioral Parity Testing (including path testing)

- Inventory every operation available through *both* the CLI (24 commands) and the web
  dashboard/API (85 routes)
- For each shared operation, execute via CLI and via API, then compare:
  - **Outcomes**: Response shape and content (same fields, same values)
  - **Side effects**: Same database state mutations, same event emissions
  - **Error behavior**: Same status codes, same error messages for equivalent failures
  - **Auth behavior**: Same permission checks, same token handling

**Path testing** (not just outcomes):
- Instrument crate boundary calls with a lightweight trace collector (behind `#[cfg(test)]`, zero
  production cost)
- Each shared operation records its execution trace — which functions were called, in what order,
  on which crates
- Execute the operation via CLI, capture trace; execute the same operation via API, capture trace
- Assert traces match structurally (same crate transitions, same DB writes, same event emissions
  in the same order)
- Divergent paths are bug ledger entries even if the final result is identical — category:
  "path divergence"

This catches cases like: CLI writes session then sends notification, but API sends notification
then writes session (race window for crash between the two steps).

#### Layer 8 — Cross-Platform Behavioral Delta Testing

**Target platforms**: macOS (aarch64-darwin, x86_64-darwin), Linux (x86_64, aarch64),
Windows (x86_64)

**General focus areas**:
- **Filesystem**: Path separators, temp directory behavior, file locking, symlink handling,
  case sensitivity
- **Process management**: Browser CDP process spawning, signal handling, child process cleanup
- **Network**: Loopback binding, socket reuse, IPv6 availability
- **SQLite**: WAL mode behavior differences, file locking across platforms
- **Cron/scheduling**: Timezone handling, system clock resolution
- **Terminal**: ANSI escape code support, stdin/stdout encoding

**Windows-specific sub-layers** (known historical and current issues):

**8a — In-Place Upgrade Testing (executable replacement)**:
- On Windows, a running `.exe` cannot be overwritten — upgrade must use rename-and-swap or staged
  replacement
- Test matrix:
  - Upgrade while server is running (should gracefully restart or queue upgrade)
  - Upgrade while CLI command is in-flight (should complete current operation, then upgrade)
  - Upgrade with locked files (WAL journal, PID file) — verify no corruption
  - Rollback on failed upgrade (partial write, permission denied)
- On macOS/Linux: verify the same upgrade codepath works with simpler overwrite semantics
- Every platform must produce identical post-upgrade state given identical pre-upgrade state

**8b — Daemon/Service Lifecycle Testing**:
- Windows service behavior (`sc.exe` / `windows-service` crate) vs Unix daemon
  (`fork`/`systemd` socket activation):
  - Start, stop, restart, status query — verify identical state transitions
  - Signal handling: `SIGTERM`/`SIGINT` (Unix) vs `SERVICE_CONTROL_STOP`/
    `SERVICE_CONTROL_SHUTDOWN` (Windows)
  - Crash recovery: process dies unexpectedly — verify clean restart, no orphaned resources
    (lock files, sockets, child processes)
  - Heartbeat behavior: does the scheduler's lease-based heartbeat survive service pause/resume
    on Windows?
  - Long-running operation interruption: what happens to an in-flight LLM call or delivery queue
    flush when the service is stopped?
- Tests run in CI on actual platform targets (not emulated)

**Method**: Run full regression battery + UAT smoke suite in CI across all 5 build targets (extend
the existing release pipeline to run tests, not just build). Any test that passes on one platform
but fails on another is a bug ledger entry.

### 2B: Bug Ledger

All discoveries feed into: `docs/audit/bug-ledger.md`

| Field | Description |
|-------|-------------|
| **ID** | `BUG-NNN` sequential identifier |
| **Source** | Discovery technique (static, fuzz, mutation, coverage-gap, wiring, parity, platform, manual) |
| **Crate** | Affected crate(s) |
| **Tier** | Dependency tier (0-6) |
| **Severity** | Critical / High / Medium / Low |
| **Category** | See categories below |
| **Description** | What's wrong |
| **Location** | `file:line` reference |
| **Reproduction** | Steps or test case that triggers it |
| **Status** | Open / In Progress / Fixed / Verified |

**Bug categories**:

| Category | Description |
|----------|-------------|
| crash | Panic or abort in production path |
| data corruption | Silent data loss or mutation |
| logic error | Incorrect behavior under normal operation |
| missing validation | Input accepted that should be rejected |
| weak test | Test exists but doesn't catch mutations |
| dead code | Unreachable code with no purpose |
| doc drift | Architecture diagram doesn't match code |
| wiring break | Contract between crates is violated |
| parity violation | CLI and web UI produce different results for same operation |
| path divergence | CLI and web UI reach same result via different intermediate steps |
| platform delta | Behavior differs across macOS/Linux/Windows |
| upgrade failure | In-place executable replacement breaks on any platform |
| service lifecycle | Daemon/service start/stop/crash behavior diverges across platforms |
| security | Security bypass, injection, or credential exposure |

### 2C: Severity Criteria

| Severity | Criteria |
|----------|----------|
| **Critical** | Data corruption, security bypass, panic in production path, financial miscalculation |
| **High** | Incorrect behavior under normal operation, silent failure, state leak across sessions |
| **Medium** | Edge-case failures, degraded performance, misleading error messages |
| **Low** | Dead code, cosmetic issues, weak tests with no production impact |

### 2D: Discovery Exit Criteria

Bug discovery is "complete" when:

1. All 8 automated layers have run to completion across all 11 crates
2. Coverage gap analysis shows no uncovered Critical or High severity code regions remain
   unexamined
3. Fuzz targets have each run for minimum 5 minutes with no new crashes in the final minute
4. Mutation testing survival rate is below 15% per crate (85%+ of mutants caught)
5. CLI/web parity matrix shows zero behavioral or path divergences for all shared operations
6. Full regression battery passes on all 5 platform targets with zero platform-specific failures

---

## Phase 3: Path to 95% Unit, Integration, and Regression Coverage

Currently at 80% floor with 1,569 tests. Target: 95% across unit, integration, and regression.

### 3A: Coverage Measurement

| Level | Measures | Tooling | Target |
|-------|----------|---------|--------|
| **Unit** | Per-function line coverage within each crate | `cargo llvm-cov` per-crate | 95% per crate |
| **Integration** | Cross-crate interaction coverage | `cargo llvm-cov` on integration suite | 95% of crate boundary functions exercised |
| **Regression** | Previously-fixed bugs have tests | Regression matrix | 100% of bug ledger entries have regression tests |

### 3B: Coverage Ramp — Inside-Out by Tier

Tests are written in dependency order. When writing a Tier 3 test (agent), you can trust the
Tier 0-2 code it depends on.

| Crate | Current tests | Target tests | Focus areas |
|-------|--------------|-------------|-------------|
| **ironclad-core** (Tier 0) | 129 | ~180 | Config parsing (every field/default/validation), error variants, personality loading, keystore crypto + failure modes |
| **ironclad-db** (Tier 1) | 229 | ~320 | All 32 tables CRUD + constraints, FTS5 (empty corpus, Unicode, special chars), embedding/HNSW edge cases, WAL concurrent read/write, migration up/down |
| **ironclad-llm** (Tier 2) | 225 | ~315 | Semantic cache (hit/miss/eviction/corruption), circuit breaker state transitions, heuristic router decision paths, format translation (4 formats + malformed), OAuth refresh/expiration/revocation |
| **ironclad-agent** (Tier 3) | 323 | ~450 | ReAct loop transitions + max-iteration + cancellation, all 10 tool categories (success/failure/timeout/denied), policy engine rules, 4-layer injection defense (each layer + composed), memory 5-tier + consolidation + eviction |
| **ironclad-wallet** (Tier 4) | 75 | ~105 | Treasury policy enforcement, yield calculations, x402 edge cases |
| **ironclad-channels** (Tier 4) | 158 | ~220 | Every adapter, delivery queue retry/dead-letter, CLI/web parity |
| **ironclad-schedule** (Tier 4) | 54 | ~76 | Cron edge cases, lease contention, timezone transitions, DST |
| **ironclad-plugin-sdk** (Tier 4) | 34 | ~48 | Manifest validation, hot-reload, sandbox escape attempts |
| **ironclad-browser** (Tier 4) | 33 | ~46 | CDP session lifecycle, process cleanup, evaluate limits |
| **ironclad-server** (Tier 5) | 370 | ~520 | All 85 routes (success/auth-fail/validation-fail/not-found), CLI/web parity, SSE UTF-8/backpressure/disconnect, config hot-reload |
| **ironclad-tests** (Tier 6) | 67 | ~130 | Full-stack user journeys, fault injection at crate boundaries, platform-specific gated tests |

### 3C: Test Quality Standards

Every new test must meet:

1. **Deterministic** — no flaky tests; time-dependent tests use controlled clocks
2. **Isolated** — no shared mutable state between tests; fresh DB per test
3. **Named descriptively** — `<unit>_<scenario>_<expected_outcome>`
4. **Categorized** — tagged as unit, integration, or regression in the regression matrix
5. **Fast** — unit tests < 100ms, integration tests < 2s (slower tests get `#[ignore]` with
   dedicated justfile target)

### 3D: Coverage Gating

- `.coverage-baseline` ratchets upward as each tier completes
- CI blocks any PR that drops coverage below current baseline (existing 0.5% tolerance)
- Target milestones: 80% -> 85% -> 90% -> 95%, each tier completion bumps baseline
- Per-crate coverage tracked separately — no crate hides behind workspace average

---

## Phase 4: Iterative Bug Fix and Retest

The engine that processes the bug ledger down to zero open entries. Key principle: **fix
inside-out, retest outside-in**.

### 4A: Triage and Prioritization

After discovery completes, the full bug ledger is triaged:

1. **Sort by tier** (Tier 0 first) — core bugs block everything downstream
2. **Within tier, sort by severity** — Critical > High > Medium > Low
3. **Group related bugs** — multiple symptoms of same root cause get a single fix
4. **Identify cross-tier bugs** — wiring issue between Tier 2 and 3 is fixed at the lower tier
   and retested at both

Output: `docs/audit/fix-queue.md` — ordered work queue.

### 4B: Fix Cycle (per bug)

```
1. REPRODUCE
   Write a failing test that demonstrates the bug.
   This test becomes the permanent regression test.
        |
        v
2. FIX
   Minimal change to make the failing test pass.
   No scope creep — fix only what's broken.
        |
        v
3. VERIFY LOCALLY
   Run the new regression test.
   Run the full tier's test suite.
   Run the downstream tier's integration tests.
        |
        v
4. COMMIT
   fix: <description> (BUG-NNN)
   Update regression-matrix.md.
   Update bug-ledger.md status -> Fixed.
        |
        v
5. CI GATE
   Full CI pipeline must pass.
   Coverage must not drop (ratchet holds).
   Cross-platform targets must pass.
        |
        v
6. VERIFY
   Bug ledger status -> Verified.
   Run just test-regression to confirm
   no other regression tests broke.
```

### 4C: Retest Strategy — Outside-In

Fixes go inside-out (core first), but retesting goes outside-in. After each batch of fixes at a
tier:

1. Run the full integration suite (`ironclad-tests`) — catches regressions at system level
2. Run the regression battery (`just test-regression`) — catches known-bug recurrences
3. Run CLI/web parity tests (Layer 7) — catches behavioral drift introduced by the fix
4. Run cross-platform CI (Layer 8) — catches platform-specific regressions
5. Re-run coverage — verify the fix + new test pushed coverage upward

If any retest fails, that's a new bug ledger entry (category: "regression from fix") triaged at
the same or higher severity as the original bug.

### 4D: Batch Rhythm

| Batch | Scope | Retest gate |
|-------|-------|-------------|
| Batch 0 | All Tier 0 bugs (core) | Full regression battery + downstream integration |
| Batch 1 | All Tier 1 bugs (db) | Same + Tier 2-3 integration |
| Batch 2 | All Tier 2 bugs (llm) | Same + agent integration |
| Batch 3 | All Tier 3 bugs (agent) | Same + domain crate integration |
| Batch 4 | All Tier 4 bugs (wallet, channels, schedule, plugin-sdk, browser) | Full integration + CLI/web parity + cross-platform |
| Batch 5 | All Tier 5 bugs (server) | Full stack retest |
| Batch 6 | All Tier 6 bugs (integration tests) | `just test-v080-go-live` |

### 4E: Feature Freeze Exit Criteria

The freeze lifts when **all** of the following are true:

1. Bug ledger has zero Open or In Progress entries
2. Every bug ledger entry is in Verified status with a passing regression test
3. Per-crate unit coverage >= 95%
4. Integration coverage >= 95% of crate boundary functions exercised
5. Regression matrix has 100% coverage of bug ledger entries
6. All 50+ architecture diagrams match current code (zero drift)
7. Wiring validation report shows all edges pass
8. CLI/web parity matrix shows zero behavioral or path divergences
9. Cross-platform CI passes on all 5 targets with zero platform-specific failures
10. Mutation testing survival rate < 15% per crate
11. `just test-v080-go-live` passes clean
12. `.coverage-baseline` updated to 95.00%

---

## Deliverable Artifacts

| Artifact | Path | Purpose |
|----------|------|---------|
| Architecture drift report | `docs/audit/architecture-drift-report.md` | Line-item inventory of diagram vs code discrepancies |
| Wiring validation report | `docs/audit/wiring-validation-report.md` | Per-edge contract assessment |
| Bug ledger | `docs/audit/bug-ledger.md` | Master defect inventory |
| Fix queue | `docs/audit/fix-queue.md` | Prioritized work queue |
| Regression matrix updates | `docs/testing/regression-matrix.md` | Maps every fix to its regression test |
| Coverage baseline | `.coverage-baseline` | Ratcheting coverage floor (80 -> 95) |
