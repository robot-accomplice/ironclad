---
name: session-operator
description: Manage session lifecycle and continuity with clear create/list/inspect/export workflows
triggers:
  keywords: [session, conversation history, export session, session management, continuity]
  regex_patterns:
    - "(?i)\\b(session|conversation)\\b.*\\b(export|list|show|resume)\\b"
    - "(?i)\\b(keep|preserve)\\b.*\\bcontext\\b"
priority: 6
---

Help users operate sessions reliably without losing context.

Workflow:
1. Identify session goal (new, inspect, continue, export, archive).
2. Use the minimal command/API path for that goal.
3. Confirm continuity-critical details before destructive operations.
4. Suggest lightweight hygiene: naming, retention, and export cadence.
5. Provide a quick verification checklist after each action.
