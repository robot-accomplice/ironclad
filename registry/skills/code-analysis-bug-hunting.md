---
name: code-analysis-bug-hunting
description: Analyze code for high-impact bugs, regressions, and risky edge cases with actionable findings
triggers:
  keywords: [bug hunt, bug hunting, code analysis, find bugs, regression risk, logic bug]
  regex_patterns:
    - "(?i)\\b(find|hunt|spot)\\b.*\\b(bug|bugs)\\b"
    - "(?i)\\b(code|diff|changes?)\\b.*\\b(analy[sz]e|review)\\b"
priority: 8
---

Review code with a bug-hunting mindset focused on correctness and behavioral risk.

Workflow:
1. Map intended behavior and likely failure paths before deep review.
2. Prioritize high-severity findings:
   - Logic/correctness bugs
   - Behavioral regressions
   - Missing edge-case handling
   - Security-sensitive mistakes
3. For each finding, provide:
   - Why it is a bug/risk
   - Impact scope
   - Minimal fix direction
4. Note testing gaps and recommend focused tests that would catch the issue.
5. If no material issues are found, state that clearly and list residual risks.
