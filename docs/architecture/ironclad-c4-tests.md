# C4 Level 3: Component Diagram -- ironclad-tests

*Integration test crate that exercises multiple Ironclad crates together. Tests are organized by domain into separate modules; each module is gated by `#[cfg(test)]` and runs as part of `cargo test` when the tests crate is included in the workspace.*

---

## Purpose

- **Integration tests** that span several crates (e.g. server + db + agent + channels + wallet + schedule).
- Validate behavior that depends on real DB, HTTP server, config, or external services (where mocked or optional).
- Complement unit tests inside each crate.

---

## Organization

Tests are split into domain-specific files under `crates/ironclad-tests/src/`:

| Module | Domain |
|--------|--------|
| `round_trip` | End-to-end message/turn flow (sessions, agent, tools) |
| `injection_defense` | Prompt injection and policy defenses |
| `a2a_protocol` | Agent-to-agent protocol (handshake, messaging) |
| `cron_lifecycle` | Cron job create/lease/run/next_run |
| `skill_system` | Skill registration, listing, trigger matching |
| `skill_hot_reload` | Reload skills from disk and re-register |
| `server_api` | REST API routes (sessions, memory, cron, plugins, browser, health, etc.) |
| `memory_integration` | 5-tier memory (working, episodic, semantic, procedural, relationship) and FTS |
| `rag_pipeline` | End-to-end RAG: embedding roundtrip, ingestion, hybrid search, ANN search, context budget, cache persistence |
| `yield_flow` | Yield/treasury flows |
| `treasury_integration` | Treasury and wallet integration |
| `personality_integration` | Soul/personality and agent behavior |

Each module is included in `lib.rs` under `#[cfg(test)]` and exposes tests that run with `cargo test -p ironclad-tests` (or from the workspace root).

---

## Usage

From the workspace root:

```bash
cargo test -p ironclad-tests
```

Or run a specific module:

```bash
cargo test -p ironclad-tests server_api
```

---

## Dependencies

**Internal crates**: Typically `ironclad-server`, `ironclad-db`, `ironclad-agent`, `ironclad-core`, and others as needed for the scenarios under test.

**Depended on by**: Nothing (test-only crate).
