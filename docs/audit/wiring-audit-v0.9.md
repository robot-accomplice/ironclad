# Wiring Re-Audit — v0.9.2 Phase 0 (Tier 1–3)

<!-- last_updated: 2026-03-02 -->

## Scope

This document is the **post-remediation re-audit** for the v0.9.2 wiring effort.
It supersedes prior Tier 1–3 BREAK findings with current production-path status.

Reference targets:

- `docs/releases/v0.9.2.md` (Phase 0 gates)
- prior baseline findings in earlier wiring audit snapshot

## Tier 1–3 Gate Matrix

| Gate | Prior State | Current State | Status |
| --- | --- | --- | --- |
| Unified request pipeline (API/channel/streaming) | Divergent paths | Shared `prepare_inference` + `execute_inference_pipeline` | PASS |
| `post_turn_ingest` tool results | `&[]` hardcoded | Actual tool results passed from ReAct/streaming | PASS |
| API-path gate system note | Channel-only | API + channel both wire gate note | PASS |
| Multi-tool parsing | First tool only | `parse_tool_calls` + provider parsers handle multiple calls | PASS |
| OpenAI Responses + Google tool/system wiring | Partial/missing | Tools/system/multimodal translations + parsing wired | PASS |
| Quality warm start | Cold-start only | `QualityTracker` seeded from `inference_costs` | PASS |
| Shared confidence evaluator | Local ad-hoc evaluator | Uses `LlmService.confidence` | PASS |
| Escalation read feedback | Writes only | Escalation acceptance read and applied as routing bias | PASS |
| Approval resume | No continuation | Async replay/re-exec queued after approval | PASS |
| SpawnManager cleanup | Dead module present | `spawning.rs` removed; no source references | PASS |
| Dead routing cleanup | Dead selectors + `uniroute` present | `uniroute.rs` removed; dead selectors removed | PASS |
| Importance decay wiring | Not called | Governor tick calls decay pass | PASS |
| Context pruning wiring | Helpers unused | `needs_pruning -> soft_trim` wired in context build | PASS |
| Checkpoint load wiring | Save-only | `load_checkpoint` called during inference prep | PASS |

## ModelRouter Alignment Decision

`ModelRouter` is **retained intentionally**.

Rationale:

- It is no longer dead code; it actively provides runtime override/fallback state.
- Removing it now would be architecture churn with no Tier 1–3 reliability gain.
- The dead routing target is satisfied by removing dead surfaces (`uniroute`, dead selectors) while keeping active runtime router responsibilities.

Resulting policy for v0.9.2 docs:

- Replace “remove ModelRouter” with “remove dead routing surfaces; keep active runtime router”.

## Diagram Alignment Notes

Updated diagrams to reflect implemented reality:

- `docs/architecture/ironclad-c4-llm.md`
  - removed `uniroute` component/detail
  - updated router and tiered blocks to active flow
- `docs/architecture/ironclad-c4-agent.md`
  - removed `spawning.rs` / `SpawnManager` components and edges

## Residual Non-Phase-0 Items

The following are outside Tier 1–3 Phase 0 release blockers and remain tracked separately:

- abuse protection subsystem (1.17)
- MCP discovery realism / protocol endpoint depth
- broader orchestration execution telemetry depth
- feature phases after Phase 0 in `v0.9.2.md`

## Verification Commands

```bash
just ci-test
cargo check -q
```

Phase 0 wiring outcome: **Tier 1–3 blockers cleared**.
