# Contributing to Ironclad

Thank you for your interest in contributing to Ironclad. This guide covers the branching model, pull request workflow, and quality requirements.

## Branching Model

Ironclad uses a **modified git flow** with two long-lived branches:

```
feature/my-feature ──► develop ──► main (tagged vX.Y.Z)
bugfix/fix-thing   ──►         ──►
```

| Branch | Purpose | Merge strategy |
| --- | --- | --- |
| `main` | Stable releases. Every merge is tagged (e.g. `v0.2.0`). | Merge commit from `develop` |
| `develop` | Integration branch. All feature work targets here. | Squash merge from feature/bugfix branches |
| `feature/*` | New functionality, branched from `develop`. | Squash merge into `develop` via PR |
| `bugfix/*` | Bug fixes, branched from `develop`. | Squash merge into `develop` via PR |

Both `main` and `develop` are **protected** -- direct pushes, force pushes, and branch deletion are blocked. All changes enter through pull requests with passing CI.

## Getting Started

1. Fork the repository (or create a branch if you have write access).
2. Branch from `develop`:

```bash
git checkout develop
git pull origin develop
git checkout -b feature/my-feature
```

3. Install dev tooling:

```bash
just install-tools
```

This installs `cargo-watch`, `cargo-llvm-cov`, `cargo-outdated`, `cargo-audit`, the `gosh` scripting engine, and a pre-push git hook that runs the full CI gate locally.

## Branch Naming

Use one of these prefixes:

- `feature/<short-description>` -- new functionality (e.g. `feature/streaming-responses`)
- `bugfix/<short-description>` -- bug fixes (e.g. `bugfix/cache-ttl-overflow`)

Keep descriptions short, lowercase, and hyphen-separated.

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>: <short summary>
```

Common types:

| Type | When to use |
| --- | --- |
| `feat` | New feature |
| `fix` | Bug fix |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `test` | Adding or updating tests |
| `docs` | Documentation only |
| `chore` | Build, CI, dependency updates |
| `perf` | Performance improvement |

Examples:

```
feat: add streaming response support to LLM pipeline
fix: resolve cache TTL overflow on 32-bit timestamps
chore: update tokio to 1.43
```

Since feature branches are squash-merged, individual commit messages within a branch are flexible -- the PR title becomes the final commit message on `develop`.

## Before Submitting a PR

Run the quality checks locally. The CI pipeline will run the same checks, but catching issues early saves time.

```bash
# Format
just fmt

# Lint
just lint

# Run full test suite
just test

# Run the complete CI pipeline locally (format + lint + per-crate tests + coverage gate + build + audit + docs)
just ci-test
```

### Quality Gates

| Gate | Requirement |
| --- | --- |
| Format | `cargo fmt --all -- --check` passes |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` passes |
| Tests | All tests pass across 11 crates |
| Coverage | Line coverage >= 80% floor, no regression below `.coverage-baseline` |
| Build | Debug and release builds succeed |
| Security | `cargo audit` finds no known vulnerabilities |
| Docs | `cargo doc --workspace --no-deps` builds without warnings |

### Coverage

Ironclad enforces an 80% minimum line coverage floor with a ratcheting baseline -- coverage must not regress below the value in `.coverage-baseline`. If your change adds significant new code, add tests to maintain or improve the baseline.

```bash
# Check coverage
just coverage-summary

# Generate HTML report
just coverage
```

## Pull Request Workflow

1. **Target branch:** `develop` (not `main`).
2. **PR title:** Use conventional commit format -- this becomes the squash-merge commit message.
3. **Description:** Explain what the change does and why. Link any related issues.
4. **CI must pass:** All status checks must be green before merge.
5. **Merge method:** Squash merge. GitHub will combine your commits into a single commit on `develop`.
6. **Final completion step:** Update relevant documentation, including `docs/architecture/` and any impacted architecture/dataflow diagrams, before marking the PR ready to merge.

## Release Process

Releases are cut by maintainers:

1. `develop` is merged into `main` via a **merge commit** (preserving the integration history).
2. The merge commit on `main` is tagged with the version number (`vX.Y.Z`).
3. The tag triggers any release automation (crates.io publish, Docker image, etc.).

## Project Structure

Ironclad is a Rust workspace with 11 crates. When contributing, work within the appropriate crate:

| Crate | Area |
| --- | --- |
| `ironclad-core` | Shared types, config, errors |
| `ironclad-db` | SQLite persistence |
| `ironclad-llm` | LLM client pipeline |
| `ironclad-agent` | Agent core, tools, policy |
| `ironclad-wallet` | Ethereum wallet, payments |
| `ironclad-schedule` | Cron/heartbeat scheduler |
| `ironclad-channels` | Chat adapters, A2A protocol |
| `ironclad-plugin-sdk` | Plugin system |
| `ironclad-browser` | Headless browser automation |
| `ironclad-server` | HTTP API, CLI, dashboard |
| `ironclad-tests` | Integration tests |

Run tests for a specific crate during development:

```bash
just test-crate agent    # runs cargo test -p ironclad-agent
just watch-crate agent   # re-runs on file changes
```

## Documentation Standards

### Code Documentation
- Every new public type, trait, and function must have doc comments
- Crate-level `//!` doc comments are required in all `lib.rs` files
- Run `cargo doc --no-deps` and fix any warnings before submitting

### Architecture Documentation
- Every new module gets added to its crate's C4 diagram
- Every new data flow gets a corresponding flowchart in `ironclad-dataflow.md`
- Every new API route gets documented in `docs/API.md`
- Every new config field gets documented in `docs/CONFIGURATION.md`
- All architecture docs must include a `<!-- last_updated: YYYY-MM-DD, version: X.Y.Z -->` header

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](LICENSE).
