# v0.8.0 Stabilization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Lift the v0.8.0 feature freeze by reaching 95% test coverage across all 11 crates, auditing all architecture diagrams, discovering and fixing every bug, and validating CLI/web parity and cross-platform behavior.

**Architecture:** Tiered convergence with inside-out validation. Phase 1 (architecture audit) is global. Phases 2-4 (discovery, coverage, fix/retest) proceed by dependency tier: core → db → llm → agent → {wallet, channels, schedule, plugin-sdk, browser} → server → tests.

**Tech Stack:** Rust, cargo-llvm-cov, cargo-mutants, cargo-fuzz (libFuzzer), proptest, cargo-audit, cargo-deny, cargo-semver-checks, just (task runner), GitHub Actions CI.

**Design doc:** `docs/plans/2026-02-26-v080-stabilization-design.md`

---

## Pre-Requisites: Tooling Setup

### Task 0: Install discovery tooling

**Files:**
- Modify: `justfile` (add new targets)
- Modify: `Cargo.toml` (add dev-dependencies)

**Step 1: Install cargo tools**

Run:
```bash
cargo install cargo-mutants cargo-deny cargo-semver-checks
```

Expected: All three install successfully. `cargo-llvm-cov`, `cargo-audit`, and `cargo-fuzz` should already be installed.

**Step 2: Create `deny.toml` config**

Create: `deny.toml`

```toml
[advisories]
vulnerability = "deny"
unmaintained = "warn"
yanked = "deny"

[licenses]
unlicensed = "deny"
allow = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Zlib",
    "BSL-1.0",
    "CC0-1.0",
    "OpenSSL",
]

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

**Step 3: Add justfile targets for new tools**

Append to `justfile`:
```just
# ── Stabilization Discovery ──────────────────────────

# Run cargo-deny checks (licenses, bans, advisories)
deny:
    cargo deny check

# Run semver compatibility check against previous release
semver-check:
    cargo semver-checks check-release --baseline-rev v0.7.1

# Run mutation testing for a specific crate
mutants crate:
    cargo mutants -p ironclad-{{crate}} --timeout 60

# Run mutation testing workspace-wide (slow — use per-crate for iteration)
mutants-all:
    cargo mutants --workspace --timeout 60

# Run expanded fuzz targets (requires nightly)
fuzz-all seconds="300":
    bash scripts/run-expanded-fuzz.sh {{seconds}}

# Per-crate coverage with missing lines shown
coverage-gaps crate:
    cargo llvm-cov -p ironclad-{{crate}} --show-missing-lines

# Run CLI/web parity tests
test-parity:
    cargo test -p ironclad-tests parity::

# Run cross-platform regression battery
test-platform:
    cargo test --workspace -- --include-ignored platform_
```

**Step 4: Commit**

```bash
git add deny.toml justfile
git commit -m "chore: add stabilization discovery tooling (deny, semver-checks, mutants)"
```

---

## Phase 1: Architecture Audit + Inside-Out Wiring Validation

### Task 1: Create audit scaffold and bug ledger template

**Files:**
- Create: `docs/audit/architecture-drift-report.md`
- Create: `docs/audit/wiring-validation-report.md`
- Create: `docs/audit/bug-ledger.md`
- Create: `docs/audit/fix-queue.md`

**Step 1: Create docs/audit directory and templates**

Create `docs/audit/bug-ledger.md`:
```markdown
# Bug Ledger — v0.8.0 Stabilization

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

## Summary

| Severity | Open | In Progress | Fixed | Verified |
|----------|------|-------------|-------|----------|
| Critical | 0    | 0           | 0     | 0        |
| High     | 0    | 0           | 0     | 0        |
| Medium   | 0    | 0           | 0     | 0        |
| Low      | 0    | 0           | 0     | 0        |

## Entries

| ID | Source | Crate | Tier | Severity | Category | Description | Location | Status |
|----|--------|-------|------|----------|----------|-------------|----------|--------|
```

Create `docs/audit/architecture-drift-report.md`:
```markdown
# Architecture Drift Report — v0.8.0

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Diagrams audited against v0.8.0 code. Diagrams were last updated at v0.5.0-v0.6.0.

## Summary

| File | Diagrams | Structural | Relationship | Behavioral | Naming | Status |
|------|----------|-----------|-------------|-----------|--------|--------|

## Detailed Findings

(populated per-file during audit)
```

Create `docs/audit/wiring-validation-report.md`:
```markdown
# Wiring Validation Report — v0.8.0

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Per-edge contract assessment across crate dependency graph.

## Edge Summary

| Producer → Consumer | Traits | Errors | State | Concurrency | Lifecycle | Status |
|---------------------|--------|--------|-------|-------------|-----------|--------|

## Detailed Findings

(populated per-edge during validation)
```

Create `docs/audit/fix-queue.md`:
```markdown
# Fix Queue — v0.8.0 Stabilization

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Prioritized by tier (inside-out), then severity (critical-first).

## Queue

| Priority | Bug ID | Tier | Severity | Crate | Description | Estimated Scope |
|----------|--------|------|----------|-------|-------------|----------------|
```

**Step 2: Commit**

```bash
git add docs/audit/
git commit -m "docs: create audit scaffold (bug ledger, drift report, wiring report, fix queue)"
```

---

### Task 2: Audit C4 System Context diagram

**Files:**
- Read: `docs/architecture/ironclad-c4-system-context.md`
- Cross-reference: `crates/ironclad-server/src/` (external integrations), `crates/ironclad-channels/src/` (channel adapters), `crates/ironclad-wallet/src/` (blockchain), `crates/ironclad-llm/src/` (LLM providers)
- Update: `docs/audit/architecture-drift-report.md`

**Step 1: Extract declared components from diagram**

Read the Mermaid C4Context block. List every `System_Ext`, `Person`, and `System` node with its label.

**Step 2: Cross-reference each external system against code**

For each external system in the diagram:
- Search for its integration in the codebase (e.g., "Ollama" → grep for `ollama` in `crates/ironclad-llm/`)
- Verify the relationship direction matches actual data flow
- Check for external systems in code that are NOT in the diagram (e.g., new v0.8.0 additions)

**Step 3: Record findings in drift report**

Append to `docs/audit/architecture-drift-report.md` under the Summary table and add a `### ironclad-c4-system-context.md` section with line-item findings.

**Step 4: File any bugs found to bug ledger**

For each discrepancy, add a row to `docs/audit/bug-ledger.md` with category `doc drift`.

**Step 5: Commit**

```bash
git add docs/audit/
git commit -m "docs(audit): audit C4 system context diagram against v0.8.0 code"
```

---

### Task 3: Audit C4 Container diagram

**Files:**
- Read: `docs/architecture/ironclad-c4-container.md`
- Cross-reference: All `crates/*/Cargo.toml` for dependency relationships
- Update: `docs/audit/architecture-drift-report.md`

**Step 1: Extract all container nodes and relationships from diagram**

**Step 2: Cross-reference against actual Cargo.toml dependencies**

Run:
```bash
for crate in crates/ironclad-*/Cargo.toml; do echo "=== $crate ==="; grep 'ironclad-' "$crate" | grep -v '#'; done
```

Compare the declared inter-crate dependencies against what the diagram shows.

**Step 3: Record findings, file bugs, commit**

Same pattern as Task 2.

---

### Task 4-13: Audit each C4 Component diagram (one per crate)

Repeat the Task 2 pattern for each of the 10 crate-level C4 diagrams:

| Task | File | Cross-reference crate |
|------|------|-----------------------|
| 4  | `docs/architecture/ironclad-c4-core.md` | `crates/ironclad-core/src/` |
| 5  | `docs/architecture/ironclad-c4-db.md` | `crates/ironclad-db/src/` |
| 6  | `docs/architecture/ironclad-c4-llm.md` | `crates/ironclad-llm/src/` |
| 7  | `docs/architecture/ironclad-c4-agent.md` | `crates/ironclad-agent/src/` |
| 8  | `docs/architecture/ironclad-c4-wallet.md` | `crates/ironclad-wallet/src/` |
| 9  | `docs/architecture/ironclad-c4-channels.md` | `crates/ironclad-channels/src/` |
| 10 | `docs/architecture/ironclad-c4-schedule.md` | `crates/ironclad-schedule/src/` |
| 11 | `docs/architecture/ironclad-c4-server.md` | `crates/ironclad-server/src/` |
| 12 | `docs/architecture/ironclad-c4-browser.md` | `crates/ironclad-browser/src/` |
| 13 | `docs/architecture/ironclad-c4-plugin-sdk.md` | `crates/ironclad-plugin-sdk/src/` |

For each task:
1. Read the diagram, extract all declared components (modules, structs, traits, functions)
2. Run `get_symbols_overview` on the crate's `lib.rs` to get actual top-level symbols
3. Compare: missing symbols, phantom symbols, renamed symbols
4. Record findings in drift report, file bugs to ledger
5. Commit after each diagram

---

### Task 14: Audit Dataflow diagrams

**Files:**
- Read: `docs/architecture/ironclad-dataflow.md`
- Cross-reference: Multiple crates (follows request lifecycle across boundaries)
- Update: `docs/audit/architecture-drift-report.md`

**Step 1: Extract each flowchart**

The file contains multiple diagrams: runtime config reload, primary request dataflow, local model onboarding, end-to-end message flow. Audit each separately.

**Step 2: For each flow, trace the actual code path**

Starting from the entry point (e.g., channel message receipt), follow the function call chain through the codebase. At each step, verify the diagram's next node matches the actual next function call.

**Step 3: Record findings, file bugs, commit**

Pay special attention to behavioral drift — flows that were reordered or short-circuited in v0.8.0 but the diagram still shows the old path.

---

### Task 15: Audit Sequence diagrams

**Files:**
- Read: `docs/architecture/ironclad-sequences.md`
- Cross-reference: Cross-crate call patterns
- Update: `docs/audit/architecture-drift-report.md`

Same approach as Task 14 but for sequence diagrams. Focus on:
- Participant ordering (are all crates involved actually called in this order?)
- Message labels (do the function names match actual function signatures?)
- Return values (do the response types match?)

---

### Task 16: Audit circuit-breaker and router audit docs

**Files:**
- Read: `docs/architecture/circuit-breaker-audit.md`
- Read: `docs/architecture/router-audit.md`
- Cross-reference: `crates/ironclad-llm/src/`
- Update: `docs/audit/architecture-drift-report.md`

These are behavioral audit docs that describe intended runtime behavior. Verify each described behavior against the actual `v0.8.0` code path.

---

### Task 17: Wiring validation — Tier 0 → Tier 1 (core → db)

**Files:**
- Read: `crates/ironclad-core/src/lib.rs` (public API surface)
- Read: `crates/ironclad-db/src/lib.rs` (how it consumes core)
- Update: `docs/audit/wiring-validation-report.md`

**Step 1: Identify all imports of `ironclad_core` in `ironclad-db`**

```bash
# Search for all uses of ironclad_core types in the db crate
rg 'ironclad_core' crates/ironclad-db/src/ --type rust
```

**Step 2: For each imported type/trait, validate the 5-point checklist**

1. Trait contracts — are all required methods implemented?
2. Error propagation — does db handle all core error variants?
3. State assumptions — does db assume config fields that core guarantees?
4. Concurrency — does db respect core's locking contracts?
5. Lifecycle — does db depend on core's init order?

**Step 3: Record per-edge findings in wiring report**

**Step 4: File any wiring breaks to bug ledger (category: `wiring break`)**

**Step 5: Commit**

```bash
git add docs/audit/
git commit -m "docs(audit): validate core → db wiring contracts"
```

---

### Tasks 18-27: Wiring validation for remaining edges

Repeat Task 17 pattern for each dependency edge in the crate graph:

| Task | Edge | Producer crate | Consumer crate |
|------|------|---------------|----------------|
| 18 | core → llm | ironclad-core | ironclad-llm |
| 19 | core → agent | ironclad-core | ironclad-agent |
| 20 | core → wallet | ironclad-core | ironclad-wallet |
| 21 | core → channels | ironclad-core | ironclad-channels |
| 22 | core → schedule | ironclad-core | ironclad-schedule |
| 23 | core → plugin-sdk | ironclad-core | ironclad-plugin-sdk |
| 24 | core → browser | ironclad-core | ironclad-browser |
| 25 | db → llm | ironclad-db | ironclad-llm |
| 26 | db → agent | ironclad-db | ironclad-agent |
| 27 | db → channels | ironclad-db | ironclad-channels |

Continue for all remaining edges:
- db → wallet, db → schedule
- llm → agent
- agent → channels
- All crates → server (Tier 5 consumes everything)
- All crates → tests (Tier 6 exercises full stack)

For each: identify imports, validate 5-point checklist, record findings, file bugs, commit.

---

### Task 28: Remediate all Phase 1 findings

**Step 1: Triage drift report**

For each finding, decide: update diagram to match code, or fix code to match intended design.

**Step 2: Fix diagrams**

For naming and structural drift, update the Mermaid diagrams in `docs/architecture/`.

**Step 3: Fix code for behavioral/wiring issues**

For each wiring break or behavioral drift that represents a real bug:
1. Write a failing test that demonstrates the contract violation
2. Fix the code minimally
3. Run `just test-crate <affected-crate>`
4. Commit: `fix: <description> (BUG-NNN)`

**Step 4: Update diagram metadata headers**

Every diagram file gets its `<!-- last_updated: -->` header updated to `2026-02-26, version: 0.8.0`.

**Step 5: Commit all diagram updates**

```bash
git add docs/architecture/ docs/audit/
git commit -m "docs: remediate architecture drift — update all diagrams to v0.8.0"
```

---

## Phase 2: Bug Discovery Program

### Task 29: Layer 1 — Static analysis

**Step 1: Run clippy pedantic**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic -W clippy::nursery 2>&1 | tee /tmp/clippy-pedantic.log
```

Note: Some pedantic lints may be false positives. Only file genuine issues to the bug ledger.

**Step 2: Run cargo deny**

```bash
cargo deny check 2>&1 | tee /tmp/cargo-deny.log
```

**Step 3: Run semver-checks**

```bash
cargo semver-checks check-release --baseline-rev v0.7.1 2>&1 | tee /tmp/semver-check.log
```

**Step 4: Run custom anti-pattern grep**

```bash
# unwrap() in non-test code
rg '\.unwrap\(\)' crates/ --type rust -g '!*test*' -g '!*tests*' --count-matches

# todo!() in non-test code
rg 'todo!\(\)' crates/ --type rust -g '!*test*' -g '!*tests*'

# SECURITY TODO / FIXME
rg '(TODO|FIXME|HACK|BUG)' crates/ --type rust -g '!*test*'
```

**Step 5: File all findings to bug ledger, commit**

```bash
git add docs/audit/bug-ledger.md
git commit -m "docs(audit): static analysis findings — clippy pedantic, deny, semver, anti-patterns"
```

---

### Task 30: Layer 2 — Coverage gap analysis

Run per-crate, inside-out:

**Step 1: Generate per-crate coverage gap reports**

```bash
for crate in core db llm agent wallet channels schedule server plugin-sdk browser; do
    echo "=== ironclad-$crate ===" >> /tmp/coverage-gaps.log
    cargo llvm-cov -p "ironclad-$crate" --show-missing-lines 2>&1 >> /tmp/coverage-gaps.log
done
```

**Step 2: Identify high-risk uncovered regions**

For each crate, review the missing-lines output. Prioritize:
- Uncovered `match` arms (potential unhandled cases)
- Uncovered error handlers (untested failure paths)
- Uncovered `unsafe` blocks (memory safety risk)
- Functions with 0% coverage + high cyclomatic complexity

**Step 3: File to bug ledger as `coverage-gap` entries, commit**

---

### Task 31: Layer 3 — Expanded fuzz testing

**Files:**
- Create: `crates/ironclad-core/fuzz/Cargo.toml`
- Create: `crates/ironclad-core/fuzz/fuzz_targets/fuzz_config_parse.rs`
- Create: `crates/ironclad-db/fuzz/Cargo.toml`
- Create: `crates/ironclad-db/fuzz/fuzz_targets/fuzz_session_input.rs`
- Create: `crates/ironclad-llm/fuzz/Cargo.toml`
- Create: `crates/ironclad-llm/fuzz/fuzz_targets/fuzz_response_parse.rs`
- Create: `crates/ironclad-channels/fuzz/Cargo.toml`
- Create: `crates/ironclad-channels/fuzz/fuzz_targets/fuzz_webhook_payload.rs`
- Create: `crates/ironclad-plugin-sdk/fuzz/Cargo.toml`
- Create: `crates/ironclad-plugin-sdk/fuzz/fuzz_targets/fuzz_manifest_parse.rs`
- Create: `crates/ironclad-schedule/fuzz/Cargo.toml`
- Create: `crates/ironclad-schedule/fuzz/fuzz_targets/fuzz_cron_parse.rs`
- Create: `scripts/run-expanded-fuzz.sh`

**Step 1: Create fuzz target for ironclad-core config parsing**

Example fuzz target (`crates/ironclad-core/fuzz/fuzz_targets/fuzz_config_parse.rs`):
```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Should never panic, regardless of input
        let _ = ironclad_core::config::IroncladConfig::from_toml_str(s);
    }
});
```

**Step 2: Create fuzz targets for each additional crate**

Follow the same pattern — feed arbitrary bytes to every parsing/deserialization entry point.

**Step 3: Create `scripts/run-expanded-fuzz.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
SECONDS_PER_TARGET="${1:-300}"
FUZZ_DIRS=(
    crates/ironclad-core/fuzz
    crates/ironclad-db/fuzz
    crates/ironclad-llm/fuzz
    crates/ironclad-agent/fuzz
    crates/ironclad-channels/fuzz
    crates/ironclad-plugin-sdk/fuzz
    crates/ironclad-schedule/fuzz
)
for dir in "${FUZZ_DIRS[@]}"; do
    if [ -d "$dir" ]; then
        crate_name=$(basename "$(dirname "$dir")")
        for target in "$dir"/fuzz_targets/*.rs; do
            target_name=$(basename "$target" .rs)
            echo "Fuzzing $crate_name::$target_name for ${SECONDS_PER_TARGET}s..."
            (cd "$dir/.." && cargo +nightly fuzz run "$target_name" -- \
                -max_total_time="$SECONDS_PER_TARGET" \
                -max_len=4096) || echo "CRASH found in $crate_name::$target_name"
        done
    fi
done
```

**Step 4: Run all fuzz targets**

```bash
just fuzz-all 300
```

**Step 5: File any crashes to bug ledger, commit**

```bash
git add crates/*/fuzz/ scripts/run-expanded-fuzz.sh docs/audit/bug-ledger.md
git commit -m "test: add expanded fuzz targets for config, db, llm, channels, plugin-sdk, schedule"
```

---

### Task 32: Layer 4 — Mutation testing (per-crate, inside-out)

**Step 1: Run mutation testing on ironclad-core**

```bash
just mutants core 2>&1 | tee /tmp/mutants-core.log
```

**Step 2: Review surviving mutants**

Each surviving mutant means a code change that no test caught. For non-trivial survivors:
- If it's dead code → file as `dead code` bug
- If it's a missing test → file as `weak test` bug

**Step 3: Repeat for each crate in tier order**

```bash
for crate in core db llm agent wallet channels schedule server plugin-sdk browser; do
    just mutants "$crate" 2>&1 | tee "/tmp/mutants-$crate.log"
done
```

**Step 4: File all survivors to bug ledger, commit**

---

### Task 33: Layer 5 — Property-based test expansion

**Files:**
- Modify: `crates/ironclad-core/src/config.rs` (add proptest roundtrips)
- Modify: `crates/ironclad-db/src/sessions.rs` (add idempotency proptests)
- Modify: `crates/ironclad-schedule/src/cron.rs` (add cron roundtrip proptests)
- Modify: `crates/ironclad-channels/src/delivery.rs` (add delivery dedup proptests)

**Step 1: Add serialization roundtrip proptests**

For each type that serializes/deserializes, add:
```rust
proptest! {
    #[test]
    fn config_roundtrip(port in 1024u16..65535, name in "[a-zA-Z]{1,32}") {
        let original = /* construct with random values */;
        let serialized = toml::to_string(&original).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        prop_assert_eq!(original, deserialized);
    }
}
```

**Step 2: Add idempotency proptests for session create-or-find**

**Step 3: Add monotonicity proptests for sequence IDs**

**Step 4: Run proptests, file any failures to bug ledger, commit**

```bash
cargo test --workspace -- proptest
git add crates/ docs/audit/bug-ledger.md
git commit -m "test: expand property-based testing — roundtrips, idempotency, monotonicity"
```

---

### Task 34: Layer 6 — Integration fault injection

**Files:**
- Modify: `crates/ironclad-tests/src/lib.rs` (add fault injection test module)
- Create: `crates/ironclad-tests/src/fault_injection.rs`

**Step 1: Write fault injection tests for each crate boundary**

Test that when a dependency returns an error, the consumer degrades gracefully:
```rust
#[tokio::test]
async fn agent_handles_db_connection_failure_gracefully() {
    // Setup: agent with a db that returns errors
    // Act: send a request through the agent
    // Assert: returns a meaningful error, does NOT panic
}
```

**Step 2: Cover the highest-risk edges from wiring validation**

Prioritize edges that the wiring report flagged as concerns.

**Step 3: Run tests, file failures to bug ledger, commit**

```bash
cargo test -p ironclad-tests fault_injection::
git add crates/ironclad-tests/ docs/audit/bug-ledger.md
git commit -m "test: add integration fault injection tests at crate boundaries"
```

---

### Task 35: Layer 7 — CLI/Web parity testing (with path tracing)

**Files:**
- Create: `crates/ironclad-tests/src/parity.rs`
- Modify: `crates/ironclad-server/src/lib.rs` (add trace collector behind `#[cfg(test)]`)

**Step 1: Inventory shared CLI/API operations**

Compare the 24 CLI commands against the 85 API routes. List every operation available through both.

**Step 2: Implement lightweight trace collector**

Add a `#[cfg(test)]` module that records function call traces at crate boundaries. Each traced operation produces a `Vec<TraceEvent>` with `(crate_name, function_name, operation)`.

**Step 3: Write parity tests for each shared operation**

For each shared operation:
```rust
#[tokio::test]
async fn parity_session_list_cli_matches_api() {
    // Setup shared state
    let state = setup_test_app_state().await;

    // Execute via API
    let api_result = api_call(&state, "GET", "/api/v1/sessions").await;
    let api_trace = collect_trace();

    // Execute via CLI handler
    let cli_result = cli_call(&state, &["sessions", "list"]).await;
    let cli_trace = collect_trace();

    // Assert outcome parity
    assert_eq!(api_result.sessions, cli_result.sessions);

    // Assert path parity
    assert_eq!(api_trace, cli_trace, "CLI and API took different code paths");
}
```

**Step 4: Run parity tests, file divergences to bug ledger, commit**

```bash
cargo test -p ironclad-tests parity::
git add crates/ docs/audit/bug-ledger.md
git commit -m "test: add CLI/web parity tests with path tracing for all shared operations"
```

---

### Task 36: Layer 8 — Cross-platform delta testing

**Files:**
- Modify: `.github/workflows/ci.yml` (add cross-platform test jobs)
- Create: `crates/ironclad-tests/src/platform.rs`

**Step 1: Add platform-gated tests**

```rust
#[cfg(target_os = "windows")]
#[test]
fn windows_upgrade_rename_and_swap() {
    // Test that running executable can be upgraded via rename-and-swap
}

#[cfg(unix)]
#[test]
fn unix_signal_handling_graceful_shutdown() {
    // Test SIGTERM triggers graceful shutdown
}

#[cfg(target_os = "windows")]
#[test]
fn windows_service_stop_flushes_delivery_queue() {
    // Test SERVICE_CONTROL_STOP allows delivery queue to flush
}
```

**Step 2: Extend CI to run tests on all 5 platform targets**

Add to `.github/workflows/ci.yml`:
```yaml
  test-cross-platform:
    needs: [lint]
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: macos-latest
            target: aarch64-apple-darwin
          - os: windows-latest
            target: x86_64-pc-windows-msvc
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace
```

**Step 3: Run locally on current platform, file any failures, commit**

```bash
cargo test --workspace -- --include-ignored platform_
git add .github/workflows/ci.yml crates/ironclad-tests/ docs/audit/bug-ledger.md
git commit -m "test: add cross-platform delta tests and CI jobs for macOS/Linux/Windows"
```

---

### Task 37: Finalize discovery — verify exit criteria

**Step 1: Check all 8 layers completed**

Review `docs/audit/bug-ledger.md`. Verify every layer has entries (or explicit "no findings").

**Step 2: Check mutation testing survival rates**

```bash
for crate in core db llm agent wallet channels schedule server plugin-sdk browser; do
    echo "=== $crate ==="
    grep -c "SURVIVED" "/tmp/mutants-$crate.log" || echo "0 survivors"
    grep -c "KILLED" "/tmp/mutants-$crate.log" || echo "0 killed"
done
```

Target: <15% survival rate per crate.

**Step 3: Verify fuzz targets are stable**

Re-run each fuzz target for 60 seconds. Zero new crashes in the final 30 seconds = stable.

**Step 4: Mark discovery phase complete in bug ledger, commit**

---

## Phase 3: Coverage Ramp to 95% (Inside-Out by Tier)

Coverage ramp overlaps with Phase 2 — many discovery tests contribute to coverage. This phase fills the remaining gaps.

### Task 38: Tier 0 coverage — ironclad-core to 95%

**Step 1: Identify gaps**

```bash
just coverage-gaps core
```

**Step 2: Write tests for uncovered code paths**

Focus on:
- Every config field default and validation rule
- Every error variant construction and matching
- Personality system loading paths
- Keystore crypto operations including failure modes

**Step 3: Verify coverage**

```bash
just coverage-crate core
```

Target: >= 95% line coverage for `ironclad-core`.

**Step 4: Commit**

```bash
git add crates/ironclad-core/
git commit -m "test(core): reach 95% coverage — config, errors, personality, keystore"
```

---

### Tasks 39-48: Coverage ramp for remaining crates

Repeat Task 38 pattern for each crate in tier order:

| Task | Crate | Tier | Key coverage focus |
|------|-------|------|--------------------|
| 39 | ironclad-db | 1 | 32-table CRUD, FTS5, embeddings, WAL, migrations |
| 40 | ironclad-llm | 2 | Semantic cache, circuit breaker, router, format translation, OAuth |
| 41 | ironclad-agent | 3 | ReAct loop, 10 tool categories, policy engine, injection defense, memory |
| 42 | ironclad-wallet | 4 | Treasury policy, yield calculations, x402 |
| 43 | ironclad-channels | 4 | Adapters, delivery queue, retry/dead-letter |
| 44 | ironclad-schedule | 4 | Cron edge cases, lease contention, DST |
| 45 | ironclad-plugin-sdk | 4 | Manifest validation, hot-reload, sandbox |
| 46 | ironclad-browser | 4 | CDP lifecycle, process cleanup, evaluate limits |
| 47 | ironclad-server | 5 | All 85 routes, SSE, config hot-reload |
| 48 | ironclad-tests | 6 | Full-stack journeys, fault injection, platform tests |

For each:
1. Run `just coverage-gaps <crate>`
2. Write tests for uncovered paths (deterministic, isolated, fast)
3. Verify >= 95% with `just coverage-crate <crate>`
4. Commit with `test(<crate>): reach 95% coverage — <focus areas>`

---

### Task 49: Update coverage baseline and verify workspace-wide 95%

**Step 1: Run full workspace coverage**

```bash
just coverage-summary
```

**Step 2: Verify all crates individually**

```bash
for crate in core db llm agent wallet channels schedule server plugin-sdk browser tests; do
    echo "=== $crate ==="
    cargo llvm-cov -p "ironclad-$crate" 2>&1 | grep '^TOTAL'
done
```

Every crate must show >= 95%.

**Step 3: Update baseline**

```bash
just coverage-update-baseline
```

Expected: `.coverage-baseline` updated to `95.XX`.

**Step 4: Commit**

```bash
git add .coverage-baseline
git commit -m "chore: ratchet coverage baseline to 95%"
```

---

## Phase 4: Iterative Bug Fix and Retest

### Task 50: Triage and build fix queue

**Step 1: Sort bug ledger by tier then severity**

Read `docs/audit/bug-ledger.md`. Reorder all Open entries:
1. Tier 0 Critical, Tier 0 High, Tier 0 Medium, Tier 0 Low
2. Tier 1 Critical, ...
3. Continue through Tier 6

**Step 2: Group related bugs**

Multiple symptoms of the same root cause → single fix entry.

**Step 3: Populate fix queue**

Write the ordered queue to `docs/audit/fix-queue.md`.

**Step 4: Commit**

```bash
git add docs/audit/
git commit -m "docs(audit): triage bug ledger and populate fix queue"
```

---

### Task 51: Batch 0 — Fix all Tier 0 (core) bugs

For each bug in the fix queue where Tier = 0:

**Step 1: Write failing test**

```bash
# Add test to crates/ironclad-core/src/<relevant_module>.rs
cargo test -p ironclad-core <test_name> -- --nocapture
# Expected: FAIL
```

**Step 2: Fix minimally**

**Step 3: Verify locally**

```bash
cargo test -p ironclad-core <test_name>
cargo test -p ironclad-core
cargo test -p ironclad-db  # downstream tier
```

**Step 4: Commit**

```bash
git add crates/ironclad-core/ docs/audit/bug-ledger.md docs/testing/regression-matrix.md
git commit -m "fix(core): <description> (BUG-NNN)"
```

**Step 5: Run outside-in retest**

```bash
just test-regression
just test-integration
```

---

### Tasks 52-57: Batch 1-6 — Fix remaining tiers

Repeat Task 51 pattern for each batch:

| Task | Batch | Tier | Crates | Retest gate |
|------|-------|------|--------|-------------|
| 52 | 1 | 1 | db | regression + Tier 2-3 integration |
| 53 | 2 | 2 | llm | regression + agent integration |
| 54 | 3 | 3 | agent | regression + domain crate integration |
| 55 | 4 | 4 | wallet, channels, schedule, plugin-sdk, browser | full integration + parity + platform |
| 56 | 5 | 5 | server | full stack retest |
| 57 | 6 | 6 | tests | `just test-v080-go-live` |

---

### Task 58: Final verification — feature freeze exit criteria

**Step 1: Verify bug ledger is clean**

```bash
grep -c "| Open |" docs/audit/bug-ledger.md
# Expected: 0
grep -c "| In Progress |" docs/audit/bug-ledger.md
# Expected: 0
```

**Step 2: Verify per-crate coverage >= 95%**

```bash
for crate in core db llm agent wallet channels schedule server plugin-sdk browser tests; do
    pct=$(cargo llvm-cov -p "ironclad-$crate" 2>&1 | grep '^TOTAL' | awk '{print $4}' | tr -d '%')
    echo "$crate: ${pct}%"
    if (( $(echo "$pct < 95.0" | bc -l) )); then
        echo "  FAIL: below 95%"
    fi
done
```

**Step 3: Verify all architecture diagrams are current**

```bash
grep -r 'last_updated.*0\.8\.0' docs/architecture/ | wc -l
# Should match total diagram file count
```

**Step 4: Verify mutation testing survival < 15%**

**Step 5: Verify CLI/web parity**

```bash
cargo test -p ironclad-tests parity:: -- --nocapture
```

**Step 6: Run the canonical go-live gate**

```bash
just test-v080-go-live
```

Expected: All stages pass.

**Step 7: Declare freeze lifted**

```bash
git add .
git commit -m "release: v0.8.0 stabilization complete — feature freeze lifted

All exit criteria met:
- Bug ledger: zero open entries
- Per-crate coverage: >= 95%
- Architecture diagrams: zero drift
- Wiring validation: all edges pass
- CLI/web parity: zero divergences
- Cross-platform: zero platform-specific failures
- Mutation testing: < 15% survival per crate
- go-live gate: passing"
```

---

## Task Summary

| Task | Phase | Description | Commit |
|------|-------|-------------|--------|
| 0 | Pre | Install tooling, add justfile targets | `chore:` |
| 1 | 1A | Create audit scaffold | `docs:` |
| 2-16 | 1A | Audit all 17 architecture diagram files | `docs(audit):` per file |
| 17-27 | 1B | Validate wiring for all crate dependency edges | `docs(audit):` per edge |
| 28 | 1C | Remediate all Phase 1 findings | `fix:` + `docs:` |
| 29 | 2 | Layer 1: Static analysis | `docs(audit):` |
| 30 | 2 | Layer 2: Coverage gap analysis | `docs(audit):` |
| 31 | 2 | Layer 3: Expanded fuzz testing | `test:` |
| 32 | 2 | Layer 4: Mutation testing | `docs(audit):` |
| 33 | 2 | Layer 5: Property-based test expansion | `test:` |
| 34 | 2 | Layer 6: Integration fault injection | `test:` |
| 35 | 2 | Layer 7: CLI/Web parity testing | `test:` |
| 36 | 2 | Layer 8: Cross-platform delta testing | `test:` |
| 37 | 2 | Verify discovery exit criteria | `docs(audit):` |
| 38-48 | 3 | Per-crate coverage ramp to 95% (11 crates) | `test(<crate>):` |
| 49 | 3 | Update baseline to 95% | `chore:` |
| 50 | 4 | Triage and build fix queue | `docs(audit):` |
| 51-57 | 4 | Fix batches 0-6 by tier | `fix(<crate>):` per bug |
| 58 | 4 | Final verification — exit criteria | `release:` |
