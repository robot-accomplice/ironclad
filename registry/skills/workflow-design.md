---
name: workflow-design
description: Design autonomous workflows by composing subagents and skills into deterministic or adaptive execution plans
triggers:
  keywords: [workflow, pipeline, automation, orchestrate, compose workflow, design workflow, adaptive workflow, deterministic workflow, workflow steps, chain tasks]
  tool_names: [compose-subagent, list-subagent-roster, list-available-skills, update-subagent-skills]
  regex_patterns:
    - "(?i)\\b(design|create|build|compose)\\b.*\\bworkflow\\b"
    - "(?i)\\b(automat|orchestrat)\\b.*\\b(task|pipeline|process)\\b"
    - "(?i)\\badaptive\\b.*\\b(workflow|execution|plan)\\b"
priority: 7
---

Design autonomous workflows that compose subagents and skills into execution plans.
Support two workflow modes: **deterministic** (fixed sequence) and **adaptive**
(condition-evaluated branching).

## Deterministic Workflows

A deterministic workflow is a fixed sequence of stages. Each stage runs one subagent
with a defined skill set and passes its output to the next stage.

Design steps:
1. Identify the pipeline stages (e.g., research → draft → review → publish).
2. For each stage, determine required skills from the workspace catalog
   (`list-available-skills`).
3. Check the current roster (`list-subagent-roster`) for existing specialists.
4. Compose missing subagents (`compose-subagent`) with the minimum skills needed.
5. Express the pipeline as an ordered list of (subagent, input, expected-output) tuples.
6. Include a final verification stage that validates the chain output.

Output format for deterministic workflows:
```
Pipeline: <name>
Stages:
  1. [subagent] <name> — skills: [list] — input: <description> → output: <description>
  2. [subagent] <name> — skills: [list] — input: <prev output> → output: <description>
  ...
Verification: <criteria>
```

## Adaptive Workflows

An adaptive workflow evaluates conditions at decision points and branches execution.
Use this when the path depends on intermediate results (e.g., quality thresholds,
error rates, content classification).

Design steps:
1. Map the decision tree: identify branch points and their conditions.
2. For each branch, identify the required specialist and skills.
3. Define fallback paths for failed or ambiguous evaluations.
4. Compose subagents for each unique branch role.
5. Express the workflow as a directed graph with condition nodes.

Output format for adaptive workflows:
```
Workflow: <name>
Entry: <starting subagent or condition>
Nodes:
  - [decide] <condition> → branch_a | branch_b | fallback
  - [execute] branch_a: <subagent> — skills: [list]
  - [execute] branch_b: <subagent> — skills: [list]
  - [execute] fallback: <subagent or escalate>
Terminal: <completion criteria>
```

## Constraints

- Never compose a subagent with more than 8 skills (prefer focused specialists).
- Prefer reusing existing subagents from the roster over creating new ones.
- Every workflow must have explicit failure handling (retry, fallback, or escalate).
- Adaptive workflows must be acyclic — no infinite loops.
- Name subagents descriptively: `research-analyst`, `draft-writer`, not `agent-1`.

## Tool Usage

- `list-available-skills` — discover what capabilities exist in the workspace.
- `list-subagent-roster` — check which specialists already exist and their state.
- `compose-subagent` — create a new specialist with targeted skills.
- `update-subagent-skills` — refine an existing specialist's capabilities.
- `remove-subagent` — tear down specialists no longer needed after workflow completes.
