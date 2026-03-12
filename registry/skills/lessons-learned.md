---
name: lessons-learned
description: Extract lessons from incidents, delivery cycles, and learned procedures
triggers:
  keywords: [retrospective, lessons, postmortem, learned, improve, retro]
priority: 6
version: "0.2.0"
author: local
---

Convert outcomes into actionable lessons with root-cause framing, corrective actions, and process updates that prevent repeated failures.

When conducting a retrospective or postmortem:

1. **Surface learned procedures**: Check the `learned/` skills directory and `learned_skills` table for recently synthesized procedures relevant to the incident. Reference their success/failure ratios to ground the discussion in data.

2. **Root-cause framing**: For each finding, classify as systemic (process/tooling gap), tactical (one-off mistake), or environmental (external dependency). Only systemic findings warrant process changes.

3. **Corrective actions**: Each action must have an owner, a deadline, and a verification method. Prefer automated prevention (hooks, guards, skills) over manual checklists.

4. **Procedure reinforcement**: If a learned skill contributed to the resolution, note it — this reinforces the skill's priority in the learning loop. If a learned skill's procedure was part of the problem, flag it for review or deprioritization.
