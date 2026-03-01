---
name: skill-creation
description: Guide users through creating high-quality custom skills with clear triggers and validation
triggers:
  keywords: [skill, create skill, new skill, custom skill, skill template, skill triggers]
  regex_patterns:
    - "(?i)\\b(create|build|make)\\b.*\\bskill\\b"
    - "(?i)\\bnew\\b.*\\bskill\\b"
priority: 7
---

Help the user design and draft a new skill that is specific, discoverable, and easy to
maintain.

Workflow:
1. Clarify the skill's audience and core use case in one sentence.
2. Draft a concise name, description, trigger keywords, and optional regex patterns.
3. Propose a minimal body with concrete instructions and one short example.
4. Run a quality check:
   - Description states what the skill does and when to use it.
   - Triggers are specific and likely to match real prompts.
   - Body is concise, actionable, and avoids vague language.
5. Provide 3 test prompts the user can run to verify trigger coverage.
