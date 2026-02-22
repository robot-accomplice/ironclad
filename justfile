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

# Enforce minimum coverage threshold (80% required, 90% goal)
coverage-check:
    #!/usr/bin/env bash
    set -euo pipefail
    output=$(cargo llvm-cov --workspace 2>&1 | grep '^TOTAL')
    pct=$(echo "$output" | awk '{print $4}' | tr -d '%')
    echo "Total line coverage: ${pct}%"
    if (( $(echo "$pct < 80.0" | bc -l) )); then
        echo "FAIL: Coverage ${pct}% is below the 80% minimum"
        exit 1
    elif (( $(echo "$pct < 90.0" | bc -l) )); then
        echo "WARN: Coverage ${pct}% is below the 90% goal"
    else
        echo "PASS: Coverage ${pct}% meets the 90% goal"
    fi

# ── Run ────────────────────────────────────────────────

# Run the server (debug build) with optional config path
run config="":
    {{ if config == "" { "cargo run --bin ironclad-server -- serve" } else { "cargo run --bin ironclad-server -- serve -c " + config } }}

# Run the server (release build)
run-release config="":
    {{ if config == "" { "cargo run --release --bin ironclad-server -- serve" } else { "cargo run --release --bin ironclad-server -- serve -c " + config } }}

# Initialize a new workspace in the given directory
init dir=".":
    cargo run --bin ironclad-server -- init {{dir}}

# Validate a config file
check-config config="ironclad.toml":
    cargo run --bin ironclad-server -- check -c {{config}}

# ── Management CLI ────────────────────────────────────

# Show agent status overview
status url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} status

# List sessions
sessions url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} sessions list

# Show session details
session id url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} sessions show {{id}}

# List skills
skills url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} skills list

# Reload skills from disk
skills-reload url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} skills reload

# List cron jobs
cron url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} cron

# Show inference costs
costs url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} metrics costs

# Show cache stats
cache-stats url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} metrics cache

# Show wallet info
wallet url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} wallet

# Show running config
show-config url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} config

# Show circuit breaker status
breaker url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} breaker

# Browse memory (tier: working, episodic, semantic, search)
memory tier url="http://127.0.0.1:18789":
    cargo run --bin ironclad-server -- --url {{url}} memory {{tier}}

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
    @ls -lh target/release/ironclad-server | awk '{print $5 " " $9}'

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
        run_stage "Test ($crate)" cargo test -p "$crate" --verbose
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
    run_stage "Build (debug)" cargo build --bin ironclad-server

    # Stage 6: Build (release)
    run_stage "Build (release)" cargo build --release --bin ironclad-server

    # Stage 7: Security Audit
    if command -v cargo-audit &>/dev/null; then
        run_stage "Security Audit" cargo audit
    else
        echo ""
        echo "  ⊘ Security Audit skipped (install: cargo install cargo-audit)"
        STAGES+=("⊘ Security Audit (skipped)")
    fi

    # Stage 8: Docs
    run_stage "Docs" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

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

    if [ -n "$COVERAGE_PCT" ]; then
        old="none"
        if [ -f .coverage-baseline ]; then
            old=$(tr -d '[:space:]' < .coverage-baseline)
        fi
        echo "$COVERAGE_PCT" > .coverage-baseline
        echo "  Ratcheted .coverage-baseline: ${old}% → ${COVERAGE_PCT}%"
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

# ── Git Hooks ──────────────────────────────────────────

# Install git hooks (pre-push runs full CI gate before push)
install-hooks:
    cp hooks/pre-push .git/hooks/pre-push
    chmod +x .git/hooks/pre-push
    @echo "✔ Installed pre-push hook (runs: just ci-test with mandatory coverage gates)"

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
