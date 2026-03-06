# Regression Matrix

This matrix maps recurring regression classes to deterministic test ownership.

Legend:

- `L1` unit regression tests (crate-local, fast)
- `L2` integration tests (cross-crate behavior)
- `L3` release smoke tests (tag-gated)

## R-CH: Channel Reliability

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-CH-01 | Telegram/webhook ingress parity with poll handling | `crates/ironclad-server/src/api/routes/agent.rs`, `crates/ironclad-server/src/api/routes/mod.rs` | L2 |
| R-CH-02 | Processing failures must produce fallback user-visible reply | `crates/ironclad-server/src/api/routes/agent.rs` | L2 |
| R-CH-03 | Retry queue permanent vs transient error classification | `crates/ironclad-channels/src/delivery.rs`, `crates/ironclad-channels/src/router.rs` | L1 |

## R-MEM: Session/Memory Isolation

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-MEM-01 | Session scope separation (`agent`/`peer`/`group`) | `crates/ironclad-db/src/sessions.rs`, `crates/ironclad-server/src/api/routes/agent.rs` | L1/L2 |
| R-MEM-02 | Memory recall must not self-echo assistant turn summaries | `crates/ironclad-agent/src/retrieval.rs`, `crates/ironclad-tests/src/memory_integration.rs` | L1/L2 |
| R-MEM-03 | Memory/session restart consistency and readback | `crates/ironclad-tests/src/memory_integration.rs`, `crates/ironclad-db/src/memory.rs` | L2 |

## R-RT: Routing/Fallback/Circuit

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-RT-01 | Tripped breakers must be excluded from selection | `crates/ironclad-llm/src/router.rs`, `crates/ironclad-llm/src/circuit.rs` | L1 |
| R-RT-02 | Fallback order is deterministic when primary is unavailable | `crates/ironclad-tests/src/router_integration.rs`, `crates/ironclad-llm/src/provider.rs` | L2 |
| R-RT-03 | Credit/terminal provider failures do not re-probe automatically | `crates/ironclad-llm/src/circuit.rs` | L1 |

## R-SAA: Scheduler/Auth/API Contracts

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-SAA-01 | Scheduler lifecycle tick and execution invariants | `crates/ironclad-schedule/src/scheduler.rs`, `crates/ironclad-tests/src/cron_lifecycle.rs` | L1/L2 |
| R-SAA-02 | Auth exemption and protected-route matrix remains stable | `crates/ironclad-server/src/auth.rs`, `crates/ironclad-server/src/api/routes/mod.rs` | L1/L2 |
| R-SAA-03 | API idempotency/contract edge behavior | `crates/ironclad-tests/src/server_api.rs` | L2/L3 |

## R-ORCH: Delegation/Orchestration/Provenance

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-ORCH-01 | Decomposition gate chooses centralized vs delegated execution deterministically | `crates/ironclad-server/src/api/routes/agent.rs` | L1/L2 |
| R-ORCH-02 | Delegation provenance must be verifiable before any live subagent claim is surfaced | `crates/ironclad-server/src/api/routes/agent.rs` | L1/L2 |
| R-ORCH-03 | Subagent creation requires explicit user approval and supports review-first config preview | `crates/ironclad-server/src/api/routes/agent.rs`, `crates/ironclad-server/src/api/routes/subagents.rs` | L2 |
| R-ORCH-04 | Delegated model suitability checks emit user-visible model-switch notices | `crates/ironclad-server/src/api/routes/agent.rs` | L1/L2 |

## R-UAT: Operator Experience (CLI + Web)

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-UAT-01 | CLI operator-critical flows (`status`, `sessions`, `config`, `subagents`) remain functional against live runtime | `scripts/run-uat-cli-smoke.sh` | L3 |
| R-UAT-02 | Dashboard/web critical APIs (`/health`, `/api/agent/status`, `/api/config/status`, `/api/subagents`) remain functional | `scripts/run-uat-web-smoke.sh` | L3 |
| R-UAT-03 | Runtime config reload/status APIs remain stable for dashboard controls | `crates/ironclad-tests/src/server_api.rs`, `scripts/run-uat-web-smoke.sh` | L2/L3 |

## R-REL: Release Artifact & Provenance Integrity

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-REL-01 | Release notes/changelog/version consistency for tagged release | `CHANGELOG.md`, `docs/releases/v0.8.0.md`, `scripts/run-release-doc-gate.sh` | L3 |
| R-REL-02 | Install surfaces remain aligned with shipped release metadata | `install.sh`, `install.ps1`, `scripts/run-release-doc-gate.sh` | L3 |
| R-REL-03 | Provenance manifest generation and self-test stay release-blocking | `scripts/generate-provenance.sh`, `.github/workflows/release.yml` | L3 |

## R-LS: Live Source Soak (Behavior Contract)

| ID | Regression Class | Primary Coverage | Layer |
| --- | --- | --- | --- |
| R-LS-01 | Internal orchestration/delegation metadata must never leak to user-visible replies | `crates/ironclad-server/src/api/routes/agent/guards.rs`, `crates/ironclad-server/src/api/routes/agent/channel_message.rs`, `crates/ironclad-server/src/api/routes/agent/core.rs` | L1/L2/L3 |
| R-LS-02 | Filesystem agency prompts (`~`, Downloads/Documents/Pictures folders) must route to executable shortcuts, not model-only refusals | `crates/ironclad-server/src/api/routes/agent/intents.rs`, `crates/ironclad-server/src/api/routes/agent/core.rs` | L1/L2/L3 |
| R-LS-03 | Foreign assistant identity/persona bleed must be stripped consistently | `crates/ironclad-server/src/api/routes/agent/guards.rs` | L1/L2/L3 |
| R-LS-04 | Geopolitical/delegation replies must include no stale-memory disclaimers and no execution-fabrication blocks | `crates/ironclad-server/src/api/routes/agent/core.rs`, `crates/ironclad-server/src/api/routes/agent/guards.rs`, `scripts/run-agent-behavior-soak.py` | L2/L3 |

Reference matrix (prompt-level, operator-observed failures): `docs/testing/live-source-soak-matrix.md`.

## Governance

- Every bugfix PR should add or update at least one regression ID above.
- Release workflow must execute the regression battery and fail on regressions.
- PR CI reports regression battery status (informational at first rollout stage).
