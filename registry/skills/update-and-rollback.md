---
name: update-and-rollback
description: Run safe upgrade workflows with prechecks, backup, verification, and rollback steps
triggers:
  keywords: [upgrade, update, rollback, restore, backup, recover version]
  regex_patterns:
    - "(?i)\\b(update|upgrade)\\b.*\\bironclad\\b"
    - "(?i)\\b(rollback|restore)\\b.*\\b(version|release)\\b"
priority: 6
---

Guide safe update execution with a clear recovery path.

Workflow:
1. Confirm current version and target version.
2. Create backup before upgrade (config + state data).
3. Apply the update with explicit verification checks.
4. Validate health and core flows after update.
5. If checks fail, execute rollback with a minimal-downtime sequence.
