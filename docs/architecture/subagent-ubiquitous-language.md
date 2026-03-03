# Subagent Ubiquitous Language

## Canonical Definition

A **subagent** is:
- independently taskable by the primary agent,
- configured with a fixed set of skills,
- personality-free (no voice/persona payload),
- model-selectable and task-directed by the primary agent via built-in Ironclad orchestration.

A **model-proxy** is not a subagent. It is a routing/proxy record and is excluded from taskable-subagent semantics.

## Current-State Gap Audit (Pre-Fix)

- `role` values were loosely enforced (`specialist`, `model-proxy`, etc.), allowing semantic drift.
- subagent create/update APIs had no first-class `skills` payload, so ownership frequently appeared empty.
- unknown fields were silently accepted, so persona-like payload keys could be sent without rejection.
- roster payload assigned global enabled skills to orchestrator, making ownership appear concentrated on the primary agent.
- `/status` and diagnostics used generic `subagents` wording, which blurred taskable subagents and proxies.

## Enforced Contract (Post-Fix)

- role validation is strict: `subagent` or `model-proxy` (legacy `specialist` normalized to `subagent`).
- fixed skills are stored on each subagent (`skills_json`) from explicit API `skills`.
- personality payloads are rejected (`personality` unsupported for subagents).
- `model-proxy` records cannot own skills.
- roster exposes taskable subagents and model proxies separately.
- status diagnostics use `taskable_subagents_*` terminology and exclude proxies.
- taskable subagent model supports three modes:
  - fixed provider/model (for example `openai/gpt-4o-mini`),
  - `auto` (Ironclad router chooses),
  - `orchestrator` (primary agent-selected model at assignment time).

## Assignment-Time Model Selection

```mermaid
flowchart TD
  subgraph Inputs["Assignment Inputs"]
    ROLE["role"]
    MODEL["configured model value"]
    TASK["task content / intent"]
    CMD["orchestrator current model"]
  end

  subgraph Validation["Contract Validation"]
    V1["role must be subagent or model-proxy"]
    V2["model cannot be empty"]
    V3["model-proxy cannot use auto/orchestrator"]
  end

  subgraph Decision["Model Resolution"]
    D1{"role == model-proxy?"}
    D2{"model == auto?"}
    D3{"model == orchestrator?"}
    FIXED["use configured provider/model"]
    AUTO["use Ironclad router result\n(select_routed_model(task))"]
    ORCHESTRATOR["use orchestrator-selected active model"]
  end

  subgraph Outputs["Resolved Assignment Model"]
    OUT["provider/model used for assignment"]
  end

  ROLE --> V1
  MODEL --> V2
  ROLE --> V3
  MODEL --> V3

  V1 --> D1
  V2 --> D1
  V3 --> D1

  D1 -->|yes| FIXED
  D1 -->|no| D2
  D2 -->|yes| AUTO
  D2 -->|no| D3
  D3 -->|yes| ORCHESTRATOR
  D3 -->|no| FIXED

  TASK --> AUTO
  CMD --> ORCHESTRATOR

  FIXED --> OUT
  AUTO --> OUT
  ORCHESTRATOR --> OUT
```

## Live Forensics Pipeline

```mermaid
flowchart TD
  subgraph Runtime["Inference Runtime"]
    TASK["Incoming task/turn"]
    SELECT["Model selection audit\n(candidates + strategy)"]
    INFER["Inference execution"]
  end

  subgraph Persistence["Forensics Storage"]
    DB["model_selection_events table"]
  end

  subgraph Streaming["Live Updates"]
    BUS["EventBus\nmodel_selection event"]
    WS["/ws"]
  end

  subgraph UI["Dashboard"]
    METRICS["Metrics: Live Model Selection Log"]
    CONTEXT["Context Explorer:\nTurn Model Selection Forensics"]
  end

  subgraph API["Forensics APIs"]
    LATEST["GET /api/models/selections"]
    PER_TURN["GET /api/turns/{id}/model-selection"]
  end

  TASK --> SELECT --> INFER
  SELECT --> DB
  SELECT --> BUS --> WS
  DB --> LATEST --> METRICS
  DB --> PER_TURN --> CONTEXT
  WS --> METRICS
  WS --> CONTEXT
```

## Dataflow

```mermaid
flowchart TD
  subgraph Entry["Entry Points"]
    API_SUB["/api/subagents"]
    API_ROSTER["/api/roster"]
    API_STATUS["/api/agent/status and /status"]
  end

  subgraph Domain["Domain Rules"]
    ROLE_VALIDATE["Role validation\n(subagent vs model-proxy)"]
    SKILL_FIX["Fixed skill ownership\n(per-subagent skills_json)"]
    NO_PERSONA["No personality enforcement"]
  end

  subgraph Storage["Persistence"]
    DB_SUB["sub_agents table"]
  end

  subgraph Runtime["Runtime + Presentation"]
    REGISTRY["Subagent registry\n(taskable only)"]
    ROSTER_VIEW["Roster response\n(orchestrator + taskable subagents)"]
    STATUS_VIEW["Runtime diagnostics\n(taskable_subagents_*)"]
  end

  API_SUB --> ROLE_VALIDATE
  API_SUB --> SKILL_FIX
  API_SUB --> NO_PERSONA
  ROLE_VALIDATE --> DB_SUB
  SKILL_FIX --> DB_SUB
  NO_PERSONA --> DB_SUB

  DB_SUB --> REGISTRY
  DB_SUB --> ROSTER_VIEW
  DB_SUB --> STATUS_VIEW

  API_ROSTER --> ROSTER_VIEW
  API_STATUS --> STATUS_VIEW
```

## Component Boundaries

```mermaid
flowchart TD
  subgraph API["ironclad-server API Layer"]
    SUB_ROUTES["routes/subagents.rs"]
    ADMIN_ROUTES["routes/admin.rs"]
    AGENT_ROUTES["routes/agent.rs"]
  end

  subgraph DB["ironclad-db"]
    AGENTS_DAO["agents.rs"]
  end

  subgraph UI["Dashboard SPA"]
    ROSTER_UI["Agents > Roster"]
  end

  SUB_ROUTES --> AGENTS_DAO
  ADMIN_ROUTES --> AGENTS_DAO
  AGENT_ROUTES --> AGENTS_DAO
  ADMIN_ROUTES --> ROSTER_UI
```
