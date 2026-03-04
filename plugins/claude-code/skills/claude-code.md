---
name: claude-code
description: Recognize when to delegate coding tasks to Claude Code CLI via the claude-code plugin
triggers:
  keywords: [claude code, claude-code, claude cli, delegate code, coding agent, code task]
  tool_names: [claude-code]
  regex_patterns:
    - "(?i)\\b(use|run|launch|invoke|delegate to)\\b.*\\bclaude\\b.*\\b(code|cli)\\b"
    - "(?i)\\bdelegate\\b.*\\b(coding|implementation|refactor)\\b"
priority: 7
---

## When to Delegate to Claude Code

Use the `claude-code` plugin tool when:
- The task requires coordinated edits across multiple files
- The task benefits from an agentic Read/Edit/Bash loop
- The task would take many sequential tool calls through your own tools
- The user explicitly requests Claude Code delegation

Do NOT delegate when:
- The task is a single-file edit or quick lookup — use your own tools
- The task requires real-time conversation or clarification — Claude Code runs headless
- The task involves secrets, credentials, or sensitive environment variables

## Output Interpretation

After a `claude-code` tool invocation, check the returned JSON:
- `success: true` + `subtype: "success"` — task completed; report the result summary, cost, and turns used
- `success: false` + `subtype: "error_max_turns"` — ran out of turns; consider resuming with the returned `session_id`
- `success: false` + `error` field — execution failed; report the error to the user

## Session Continuity

The tool returns a `session_id` that can be passed back in a follow-up call to resume a partial task.
If the user says "continue" or "keep going", pass the previous `session_id` to the next invocation.
Alternatively, set `continue_last: true` to resume the most recent session.

## Cost Awareness

Report cost after every invocation. Track cumulative cost across resumed sessions.
If cumulative cost exceeds $2.00 for a single logical task, pause and confirm with the user.
