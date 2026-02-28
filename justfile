set dotenv-load

default:
    @just --list

# ── Build ──────────────────────────────────────────────

# Debug build (all crates)
build:
    cargo build

# Release build (optimized single binary)
release:
    cargo build --release

# Build a specific crate
build-crate crate:
    cargo build -p ironclad-{{crate}}

# Check workspace without producing binaries
check:
    cargo check --workspace

# ── Test ───────────────────────────────────────────────

# Run full test suite (unit + integration)
test:
    cargo test --workspace

# Run tests for a specific crate (core, db, llm, agent, wallet, channels, schedule, server, tests)
test-crate crate:
    cargo test -p ironclad-{{crate}}

# Run only integration tests
test-integration:
    cargo test -p ironclad-tests

# Run server API integration tests (used by CI)
test-integration-api:
    cargo test -p ironclad-tests server_api::

# Run focused regression battery (deterministic and release-oriented)
test-regression:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo test -p ironclad-agent retriever_skips_turn_summary_working_entries
    cargo test -p ironclad-channels process_webhook_update_advances_id_without_rewinding
    cargo test -p ironclad-channels send_to_permanent_error_does_not_enqueue_retry
    cargo test -p ironclad-db retrieve_working_is_session_isolated
    cargo test -p ironclad-tests memory_retrieval_excludes_turn_summary_echoes
    cargo test -p ironclad-tests scoped_sessions_remain_isolated_between_peer_and_group
    cargo test -p ironclad-tests router_falls_through_multiple_blocked_candidates
    cargo test -p ironclad-tests cron_schedule_rejects_invalid_timestamps
    cargo test -p ironclad-tests cron_schedule_rejects_invalid_expressions
    cargo test -p ironclad-server webhook_telegram_non_message_update_advances_offset
    cargo test -p ironclad-server query_token_not_accepted_for_non_ws_paths
    cargo test -p ironclad-server channels_dead_letter_limit_is_clamped

# Release-critical deterministic battery (workspace + integration + regression)
test-release-critical:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo test --workspace --locked
    cargo test -p ironclad-tests server_api:: --locked
    just test-regression

# Long-running soak/fuzz battery (bounded runtime by env)
# Env knobs:
#   SOAK_ROUNDS=5 FUZZ_SECONDS=45 just test-soak-fuzz
test-soak-fuzz:
    bash scripts/run-soak-fuzz.sh

# CLI UAT smoke against a running server
# Env knobs:
#   BASE_URL=http://127.0.0.1:18789 API_KEY=... just test-uat-cli
test-uat-cli:
    bash scripts/run-uat-cli-smoke.sh

# Web/dashboard UAT smoke against a running server
# Env knobs:
#   BASE_URL=http://127.0.0.1:18789 API_KEY=... just test-uat-web
test-uat-web:
    bash scripts/run-uat-web-smoke.sh

# Release docs + artifact/provenance consistency gate
test-release-doc-gate:
    RELEASE_TARGET_VERSION="${RELEASE_TARGET_VERSION:-0.8.1}" bash scripts/run-release-doc-gate.sh

# Canonical v0.8.0 go-live gate (life-or-death mode)
test-v080-go-live:
    #!/usr/bin/env bash
    set -euo pipefail
    just test-release-critical
    just test-soak-fuzz
    bash scripts/run-uat-stack.sh
    just test-release-doc-gate

# Run integration smoke checks against a live server
# Usage:
#   just smoke
#   BASE_URL=http://127.0.0.1:8787 just smoke
#   API_KEY=... just smoke
smoke:
    bash scripts/run-smoke.sh

# Run tests matching a name filter
test-filter filter:
    cargo test --workspace -- {{filter}}

# Run tests with output shown
test-verbose:
    cargo test --workspace -- --nocapture

# ── Quality ────────────────────────────────────────────

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format + lint
check-all: fmt-check lint
    cargo test --workspace --no-run

# ── Coverage ───────────────────────────────────────────

# Run tests with coverage (requires cargo-llvm-cov)
coverage:
    cargo llvm-cov --workspace --html
    @echo "Report: target/llvm-cov/html/index.html"

# Print coverage summary to terminal
coverage-summary:
    cargo llvm-cov --workspace

# Per-crate coverage
coverage-crate crate:
    cargo llvm-cov -p ironclad-{{crate}} --html
    @echo "Report: target/llvm-cov/html/index.html"

# Update .coverage-baseline to current coverage (ratchet forward)
coverage-update-baseline:
    #!/usr/bin/env bash
    set -euo pipefail
    pct=$(cargo llvm-cov --workspace 2>&1 | grep '^TOTAL' | awk '{print $4}' | tr -d '%')
    if [ -z "$pct" ]; then
        echo "ERROR: Could not parse coverage"
        exit 1
    fi
    old="none"
    if [ -f .coverage-baseline ]; then
        old=$(tr -d '[:space:]' < .coverage-baseline)
    fi
    echo "$pct" > .coverage-baseline
    echo "Updated .coverage-baseline: ${old}% → ${pct}%"

# Enforce minimum coverage threshold (70% required, 80% goal)
coverage-check:
    #!/usr/bin/env bash
    set -euo pipefail
    output=$(cargo llvm-cov --workspace 2>&1 | grep '^TOTAL')
    pct=$(echo "$output" | awk '{print $4}' | tr -d '%')
    echo "Total line coverage: ${pct}%"
    if (( $(echo "$pct < 80.0" | bc -l) )); then
        echo "FAIL: Coverage ${pct}% is below the 80% minimum"
        exit 1
    elif (( $(echo "$pct < 85.0" | bc -l) )); then
        echo "WARN: Coverage ${pct}% is below the 85% goal"
    else
        echo "PASS: Coverage ${pct}% meets the 80% goal"
    fi

# ── Run ────────────────────────────────────────────────

# Run the server (debug build) with optional config path
run config="":
    {{ if config == "" { "cargo run --bin ironclad -- serve" } else { "cargo run --bin ironclad -- serve -c " + config } }}

# Run the server (release build)
run-release config="":
    {{ if config == "" { "cargo run --release --bin ironclad -- serve" } else { "cargo run --release --bin ironclad -- serve -c " + config } }}

# Run local source with installed config/data paths (~/.ironclad by default)
run-installed-config config="~/.ironclad/ironclad.toml":
    cargo run --bin ironclad -- serve -c {{config}}

# Initialize a new workspace in the given directory
init dir=".":
    cargo run --bin ironclad -- init {{dir}}

# Validate a config file
check-config config="ironclad.toml":
    cargo run --bin ironclad -- check -c {{config}}

# ── Management CLI ────────────────────────────────────

# Show agent status overview
status url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} status

# List sessions
sessions url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} sessions list

# Show session details
session id url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} sessions show {{id}}

# List skills
skills url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} skills list

# Reload skills from disk
skills-reload url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} skills reload

# List cron jobs
cron url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} cron

# Show inference costs
costs url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} metrics costs

# Show cache stats
cache-stats url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} metrics cache

# Show wallet info
wallet url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} wallet

# Show running config
show-config url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} config

# Show circuit breaker status
breaker url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} breaker

# Browse memory (tier: working, episodic, semantic, search)
memory tier url="http://127.0.0.1:18789":
    cargo run --bin ironclad -- --url {{url}} memory {{tier}}

# ── Docs ───────────────────────────────────────────────

# Generate rustdoc for all crates
doc:
    cargo doc --workspace --no-deps --open

# Generate docs without opening browser
doc-build:
    cargo doc --workspace --no-deps

# ── Database ───────────────────────────────────────────

# Open the SQLite database with the CLI
db path="~/.ironclad/state.db":
    sqlite3 {{path}}

# Show all tables in the database
db-tables path="~/.ironclad/state.db":
    sqlite3 {{path}} ".tables"

# Dump schema
db-schema path="~/.ironclad/state.db":
    sqlite3 {{path}} ".schema"

# ── Dev Utilities ──────────────────────────────────────

# Watch for changes and rebuild (requires cargo-watch)
watch:
    cargo watch -x check -x "test --workspace"

# Watch a specific crate
watch-crate crate:
    cargo watch -x "test -p ironclad-{{crate}}"

# Count lines of Rust source
loc:
    @find crates -name '*.rs' | xargs wc -l | tail -1

# Count tests across the workspace
test-count:
    @cargo test --workspace 2>&1 | rg "test result:" | awk '{sum += $4} END {print sum " tests total"}'

# List all crate names
crates:
    @ls crates | sed 's/ironclad-//'

# Tree view of the workspace (depth 2)
tree:
    @tree crates -L 2 -I target

# Show binary size (release)
size: release
    @ls -lh target/release/ironclad | awk '{print $5 " " $9}'

# ── CI ────────────────────────────────────────────────

# Run the full CI pipeline locally (mirrors .github/workflows/ci.yml)
ci-test:
    #!/usr/bin/env bash
    set -euo pipefail

    PASS=0
    FAIL=0
    STAGES=()

    run_stage() {
        local name="$1"; shift
        echo ""
        echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        echo "  Stage: $name"
        echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        if "$@" 2>&1; then
            echo "  ✔ $name passed"
            PASS=$((PASS + 1))
            STAGES+=("✔ $name")
        else
            echo "  ✘ $name FAILED"
            FAIL=$((FAIL + 1))
            STAGES+=("✘ $name")
        fi
    }

    CRATES=(
        ironclad-core
        ironclad-db
        ironclad-llm
        ironclad-agent
        ironclad-wallet
        ironclad-schedule
        ironclad-channels
        ironclad-server
        ironclad-plugin-sdk
        ironclad-browser
        ironclad-tests
    )

    echo "╔══════════════════════════════════════════════════════╗"
    echo "║           Ironclad CI — Local Pipeline               ║"
    echo "╚══════════════════════════════════════════════════════╝"

    # Stage 1: Format
    run_stage "Format" cargo fmt --all -- --check

    # Stage 2: Lint
    run_stage "Lint" cargo clippy --workspace --all-targets -- -D warnings

    # Stage 3: Test (per-crate)
    for crate in "${CRATES[@]}"; do
        run_stage "Test ($crate)" cargo test -p "$crate" --verbose --locked
    done

    # Stage 4: Coverage gate (80% floor + no regression)
    COVERAGE_PCT=""

    coverage_gate() {
        local output baseline
        output=$(cargo llvm-cov --workspace 2>&1)
        echo "$output"
        COVERAGE_PCT=$(echo "$output" | grep '^TOTAL' | awk '{print $4}' | tr -d '%')

        if [ -z "$COVERAGE_PCT" ]; then
            echo "  ERROR: Could not parse coverage from cargo llvm-cov output"
            return 1
        fi

        echo ""
        echo "  Total coverage: ${COVERAGE_PCT}%"

        if (( $(echo "$COVERAGE_PCT < 80.0" | bc -l) )); then
            echo "  FAIL: Coverage ${COVERAGE_PCT}% is below the 80% minimum"
            return 1
        fi

        if [ -f ".coverage-baseline" ]; then
            baseline=$(tr -d '[:space:]' < .coverage-baseline)
            echo "  Baseline: ${baseline}% → Current: ${COVERAGE_PCT}%"
            if (( $(echo "$COVERAGE_PCT < $baseline" | bc -l) )); then
                echo "  FAIL: Coverage regressed from ${baseline}% to ${COVERAGE_PCT}%"
                return 1
            fi
        else
            echo "  WARN: No .coverage-baseline file — skipping regression check"
        fi
        return 0
    }

    if command -v cargo-llvm-cov &>/dev/null; then
        run_stage "Coverage" coverage_gate
    else
        echo ""
        echo "  ⊘ Coverage skipped (install: cargo install cargo-llvm-cov)"
        STAGES+=("⊘ Coverage (skipped)")
    fi

    # Stage 5: Build (debug)
    run_stage "Build (debug)" cargo build --bin ironclad --locked

    # Stage 6: Build (release)
    run_stage "Build (release)" cargo build --release --bin ironclad --locked

    # Stage 7: Security Audit
    if command -v cargo-audit &>/dev/null; then
        run_stage "Security Audit" cargo audit
    else
        echo ""
        echo "  ⊘ Security Audit skipped (install: cargo install cargo-audit)"
        STAGES+=("⊘ Security Audit (skipped)")
    fi

    # Stage 8: Docs
    run_stage "Docs" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items

    # Summary
    echo ""
    echo "╔══════════════════════════════════════════════════════╗"
    echo "║                   CI Summary                         ║"
    echo "╚══════════════════════════════════════════════════════╝"
    for s in "${STAGES[@]}"; do
        echo "  $s"
    done
    echo ""
    echo "  Passed: $PASS   Failed: $FAIL"
    echo ""
    if [ "$FAIL" -gt 0 ]; then
        echo "  ✘ CI FAILED"
        exit 1
    else
        echo "  ✔ CI PASSED"
    fi

    if [ -n "$COVERAGE_PCT" ] && [ -f .coverage-baseline ]; then
        old=$(tr -d '[:space:]' < .coverage-baseline)
        echo "  Coverage: ${COVERAGE_PCT}% (baseline: ${old}%)"
        echo "  Note: baseline is CI-authoritative. Use 'just coverage-update-baseline' to update manually."
    fi

# ── Clean ──────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean

# ── Dependencies ───────────────────────────────────────

# Check for outdated dependencies (requires cargo-outdated)
outdated:
    cargo outdated --workspace

# Audit dependencies for security vulnerabilities (requires cargo-audit)
audit:
    cargo audit

# Dependency tree
deps:
    cargo tree --workspace --depth 1

# ── Release ────────────────────────────────────────────

# Validate versions and changelog before tagging a release
release-preflight:
    #!/usr/bin/env bash
    set -euo pipefail
    ws_ver=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    echo "Workspace version: $ws_ver"

    # Check all crates resolve to the same version
    server_ver=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
      | jq -r '.packages[] | select(.name == "ironclad-server") | .version')
    if [ "$server_ver" != "$ws_ver" ]; then
        echo "✘ ironclad-server version ($server_ver) does not match workspace ($ws_ver)"
        exit 1
    fi
    echo "✔ All crate versions match: $ws_ver"

    # Check CHANGELOG entry exists
    if ! grep -q "## \[$ws_ver\]" CHANGELOG.md; then
        echo "✘ No CHANGELOG.md entry for $ws_ver"
        exit 1
    fi
    echo "✔ CHANGELOG.md entry found"

    # Check tag doesn't already exist
    if git rev-parse "v$ws_ver" >/dev/null 2>&1; then
        echo "✘ Tag v$ws_ver already exists"
        exit 1
    fi
    echo "✔ Tag v$ws_ver is available"

    echo ""
    echo "Ready to release: v$ws_ver"
    echo "  Run: just release-tag"

# Create and push a release tag (triggers the release workflow)
release-tag:
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    just release-preflight
    echo ""
    read -p "Tag and push v$ver? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 0
    fi
    git tag -a "v$ver" -m "Release v$ver"
    git push origin "v$ver"
    echo ""
    echo "✔ Tagged and pushed v$ver"
    echo "  Monitor: https://github.com/robot-accomplice/ironclad/actions"

# ── Git Hooks ──────────────────────────────────────────

# Install git hooks (pre-commit for format, pre-push for full CI gate)
install-hooks:
    cp hooks/pre-commit .git/hooks/pre-commit
    chmod +x .git/hooks/pre-commit
    cp hooks/pre-push .git/hooks/pre-push
    chmod +x .git/hooks/pre-push
    @echo "✔ Installed pre-commit hook (format check) and pre-push hook (full CI gate)"

# ── Install Dev Tools ──────────────────────────────────

# Install recommended cargo tools + gosh scripting engine
install-tools: install-gosh install-hooks
    cargo install cargo-watch cargo-llvm-cov cargo-outdated cargo-audit

# Check for Go toolchain (prerequisite for gosh)
check-go:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v go &>/dev/null; then
        echo "✔ Go $(go version | awk '{print $3}') found at $(which go)"
    else
        echo "✘ Go not found. Install from https://go.dev/dl/"
        echo "  macOS:  brew install go"
        echo "  Linux:  See https://go.dev/doc/install"
        exit 1
    fi

# Install gosh cross-platform shell (preferred plugin scripting engine)
install-gosh: check-go
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v gosh &>/dev/null; then
        echo "✔ gosh already installed at $(which gosh)"
    else
        echo "Installing gosh..."
        go install github.com/drewwalton19216801/gosh@latest
        if command -v gosh &>/dev/null; then
            echo "✔ gosh installed at $(which gosh)"
        else
            echo "✔ gosh built. Ensure \$GOPATH/bin (or \$HOME/go/bin) is in your PATH."
        fi
    fi

# ── Stabilization Discovery ────────────────────────────

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
