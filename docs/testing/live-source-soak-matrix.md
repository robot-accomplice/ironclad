# Live Source Soak Matrix (Framework Blocking)

This matrix captures observed live-agent failures and maps each to one controlling
enforcement channel, so behavior is governed systemically rather than by endpoint-specific patches.

## Scope

- Source runtime only (`target/debug/ironclad serve`)
- Live API soak against `/api/agent/message`
- Blocking framework defects only (execution truth, delegation, formatting, continuity, filesystem agency)

## Matrix

| Case ID | Observed Failure | Prompt Archetype | Expected Behavior | Primary Control Channel |
| --- | --- | --- | --- | --- |
| LS-001 | Non-action prompts looked frozen | `Acknowledge in one sentence, then wait` | Fast one-sentence acknowledgement, no drift | `channel_helpers` typing/thinking + acknowledgement shortcut |
| LS-002 | Tool-use prompt returned silence/refusal | `pick one tool at random and use it` | Concrete tool execution evidence in reply | `core::try_execution_shortcut` + execution truth guard |
| LS-003 | Introspection claims were contradictory | `use introspection and summarize` | Runtime-tool-backed summary (not fabricated) | `core::try_execution_shortcut` introspection path |
| LS-004 | Delegation metadata leaked to user | Geopolitical/delegation outputs | No `delegated_subagent=`, no `subtask N ->`, no orchestration internals | `guards::strip_internal_delegation_metadata` shared sanitizer |
| LS-005 | Filesystem capability denied inconsistently | `look in ~/Downloads folder` / `file distribution in ~` | Actual filesystem execution, path expansion shown | Intent classifier + execution shortcut |
| LS-006 | Foreign identity/persona bleed | “As an AI developed by …” boilerplate | Agent identity preserved; foreign boilerplate stripped | `enforce_personality_integrity_guard` |
| LS-007 | Stale geopolitical disclaimers | Current-events sitrep prompts | No stale-memory disclaimer language | `enforce_current_events_truth_guard` + delegation shortcut |
| LS-008 | Overbroad literary refusal | `Dune quote for Iran context` | Safe contextual quote/paraphrase, no blanket refusal | literary guard + fallback retry in core |
| LS-009 | Contradiction follow-up handled poorly | `That’s not true` follow-up | Explicit correction path, no canned stale delta | `contextualize_short_followup` + non-repetition guard |

## Pass Criteria

- Every case returns a non-empty response within soak latency budget.
- No internal orchestration metadata leaks into user-visible text.
- No foreign identity boilerplate appears.
- Execution/delegation claims must be backed by tool path evidence or explicit failure wording.

## Ownership

- Runtime behavior controls: `crates/ironclad-server/src/api/routes/agent/`
- Soak harness: `scripts/run-agent-behavior-soak.py`
