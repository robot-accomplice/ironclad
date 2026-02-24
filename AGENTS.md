# Ironclad Development Guide

Ironclad is an autonomous AI agent runtime built in Rust (11-crate workspace). See `README.md` for architecture overview and `CONTRIBUTING.md` for PR workflow.

## Cursor Cloud specific instructions

### Quick reference

| Task | Command |
|------|---------|
| Build | `cargo build --workspace` |
| Test | `cargo test --workspace` (1864 tests) |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Format check | `cargo fmt --all -- --check` |
| Format fix | `cargo fmt --all` |
| Run server | `cargo run --bin ironclad -- serve -c ironclad-ui-test.toml` |
| Full local CI | `just ci-test` |

The `justfile` has many more targets — run `just --list` to see all.

### Rust toolchain

The workspace requires **Rust >= 1.85** (edition 2024). Some transitive dependencies (e.g. `wasmer`) require newer toolchains; use `rustup default stable` to get the latest. The VM default toolchain (1.83) is too old.

### System dependencies

- `libssl-dev` and `pkg-config` are required for the `openssl-sys` crate (used by HTTP client dependencies). Install with `apt-get install -y libssl-dev pkg-config`.
- SQLite is bundled via `rusqlite` with the `bundled` feature — no system SQLite needed.

### Running the server

Use `ironclad-ui-test.toml` for development — it uses an **in-memory SQLite database** and binds to `127.0.0.1:19789`. The dashboard is served at the root URL.

No external services (databases, message queues, etc.) are needed for build/test/run. An LLM provider (e.g. Ollama) is only required if you want to send actual inference requests through the agent pipeline; all tests use mocks.

### Testing notes

- `cargo test --workspace` runs all unit and integration tests across 11 crates. Tests are self-contained with in-memory databases.
- Per-crate testing: `just test-crate <name>` (e.g. `just test-crate agent`).
- Integration tests: `just test-integration`.
- Smoke tests against a live server: `just smoke` (requires a running server, default `http://127.0.0.1:8787`).

### Git hooks

The repo ships hooks in `hooks/` (pre-commit: format check, pre-push: full CI gate). Install with `just install-hooks`. These are not installed by default.
