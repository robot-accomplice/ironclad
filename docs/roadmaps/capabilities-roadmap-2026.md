# Ironclad Capabilities Roadmap 2026

## Purpose

This roadmap is the **single source of truth** for all user-facing capability delivery in 2026. It covers two layers:

- **Plugins** (P.1–P.12) — Runtime primitives loaded/unloaded through the PluginRegistry. They add tool types, execution models, integration surfaces, and architectural capabilities. Lower-level.
- **Skills** — Operator-facing packs that consume plugin and platform infrastructure. Higher-level. Delivered as built-in (always active) or optional (catalog install).

A third layer, **embedded functionality**, lives in the [core project roadmap](../ROADMAP.md) and [v0.9.x release series](../releases/v0.9.x-series.md). Embedded items are referenced here when they gate or unlock plugin/skill work.

**Plugin test**: Can it be loaded/unloaded at runtime through the PluginRegistry without touching core agent internals? If it reaches into the memory retrieval hot path or modifies core system behavior, it belongs in the embedded roadmap.

**Skill cadence**: 0–2 net-new skills/month, plus quality updates. Quarterly re-ranking based on telemetry and support demand.

---

## Competitive Landscape (March 2026)

### What others are shipping

| Framework | Key Differentiators | Stars/Adoption |
|---|---|---|
| **OpenAI Agents SDK** | Agents-as-tools (typed handoffs), hosted tools (Codex sandbox, file search), guardrails as first-class primitives, human-in-the-loop pause/resume, MCP integration | Proprietary, massive adoption |
| **LangGraph** | Graph-based orchestration with typed state, interrupt/resume at any node, persistent checkpoints, conditional branching, "Agentic Mesh" vision | LangChain ecosystem |
| **CrewAI** | Role-based multi-agent crews, dead-simple API, code interpreter integration, task delegation patterns | 44K+ GitHub stars |
| **Legacy** | Local-first personal agent, 2,800+ community skills (SkillHub), heartbeat/cron proactive execution, markdown-based skill format, Docker sandbox, multi-channel (Signal/Telegram/Discord/WhatsApp/Slack/iMessage) | 140K+ GitHub stars, viral growth |
| **Claude MCP ecosystem** | Tool Search (85% token reduction), MCP Apps (interactive UI), donated to Linux Foundation as open standard, massive connector expansion | 97M+ monthly SDK downloads |

### Where Ironclad is strong

- **Memory system**: 5-tier hybrid (FTS5 + vector) — more sophisticated than any competitor's persistence model
- **Security posture**: Constant-time auth, RiskLevel gating, injection defense, abuse protection — structural, not bolted-on
- **MCP server integration**: P.1 MCP Gateway server half implemented; ahead of CrewAI, on par with OpenAI
- **Multi-channel delivery**: Telegram + WhatsApp wired with webhook auth; Legacy has more channels but less auth rigor
- **WASM sandbox**: 2.7 shipped — capability-gated, memory-limited, JSON-oriented; ahead of CrewAI, on par with OpenAI Codex for lightweight sandboxing

### Gaps to close

1. **Tool Search / Dynamic Selection** — critical once MCP client ships (Claude MCP ecosystem)
2. **Agent Delegation** — agents-as-tools, industry convergence point (OpenAI, LangGraph)
3. **Container Execution** — Docker/nsjail for heavyweight sandboxing beyond WASM (OpenAI, CrewAI, Legacy)
4. **Proactive Background Execution** — Legacy heartbeat/cron model (addressed by P.7 Sensor Mesh)
5. **Community Skill Distribution** — Legacy SkillHub (2,800+ skills); core infra ships v0.9.6, P.10 adds trust layer
6. **Graph-Based Orchestration** — LangGraph directed graphs vs our linear tool-call loop

---

## Current Baseline (Already Shipped)

### Skills in production

From `registry/builtin-skills.json` and `registry/manifest.json`:

- **Built-in**: `install-setup-assistant`, `runtime-diagnostics`, `provider-auth-troubleshooter`
- **Optional**: `update-and-rollback`, `session-operator`, `skill-creation`, `model-routing-tuner`, `code-analysis-bug-hunting`

Runtime/CLI surfaces already wired: `/api/skills/catalog*`, `ironclad skills catalog *`, install/activate/reload/list flows.

### Embedded items that unlock plugins and skills

| Embedded Item | Status | What It Unlocks |
|---|---|---|
| 3.2 MCP Integration | Shipped | P.1 MCP Gateway (client side remaining) |
| 1.2 Approval Workflow API | Shipped | P.5 Policy-as-Code (general-purpose DSL) |
| 1.22 Introspection Skill | Shipped v0.9.0 | P.3 Tool Search (discovery surface) |
| 2.7 WASM Plugin Runtime | Shipped | P.9 Container Execution (heavyweight complement) |
| 2.12 Episodic Digest | Shipped v0.9.0 | Dream Mode (relocated to embedded) |
| 3.3 Multi-Agent Orchestration | Partial | P.6 Agent Delegation + P.12 Workflow Graphs |
| 2.14 Skills Catalog | v0.9.6 | P.10 Skill Forge (trust layer on top) |
| 2.21 Skill Registry Protocol | v0.9.6 | P.10 Skill Forge (publish workflow) |

---

## Unified Month-by-Month Schedule

Each month interleaves plugin and skill delivery. Items are grouped by delivery type.

```
Jan   Skills:  install-setup-assistant, runtime-diagnostics (shipped, built-in)
Feb   Skills:  provider-auth-troubleshooter, update-and-rollback (shipped)
Mar   Skills:  session-operator, skill-creation, model-routing-tuner,
               code-analysis-bug-hunting (shipped)
      Plugins: P.1 MCP Gateway client, P.2 OpenAPI Importer, P.3 Tool Search (start)

Apr   Skills:  prompt-quality-auditor
      Plugins: P.3 Tool Search (ship), P.4 Flight Recorder,
               P.5 Policy-as-Code (start), P.9 security design review

May   Skills:  workflow-planner
      Plugins: P.5 Policy-as-Code (ship), P.6 Agent Delegation,
               P.7 Sensor Mesh (start)

Jun   Skills:  config-drift-checker, session-hygiene-manager
      Plugins: P.7 Sensor Mesh (ship), P.8 Adversarial Red Team,
               P.9 Container Execution (ship), P.10 Skill Forge (start),
               P.12 Workflow Graphs spec

Jul   Skills:  release-readiness-assistant, fix-validation
      Plugins: P.10 Skill Forge (ship)

Aug   Skills:  security-misconfig-hunter
      Plugins: P.11 A2A v0.3 Bridge

Sep   Skills:  test-gap-analyzer, incident-retrospective-writer
      Plugins: P.12 Workflow Graphs

Oct   Skills:  dependency-risk-reviewer, mcp-integration-helper

Nov   Skills:  plugin-troubleshooter, policy-guardrail-designer

Dec   Skills:  cross-channel-operator, cost-anomaly-investigator
               (or knowledge-workflow-curator if dependency gates not met)
```

---

## Plugin Detail

### Wave 1 — Foundation (March–April)

Infrastructure for everything else. Fits Tier 1 "Wire the Last Mile".

| # | Plugin | Effort | Depends On | Target | What It Unlocks |
|---|---|---|---|---|---|
| P.1 | **MCP Gateway** | Medium | 3.2 (partial MCP exists) | Server: done. Client: **March** | Bidirectional: expose Ironclad tools as MCP server + consume any MCP tool. 97M+ monthly SDK downloads — the on-ramp to the entire tool ecosystem |
| P.2 | **OpenAPI Tool Importer** | Low | Plugin SDK | **March** | Auto-generate Tool trait impls from OpenAPI/Swagger specs. Instantly wraps any REST API as an agent tool with JSON Schema validation |
| P.3 | **Tool Search** | Low-Med | ToolRegistry, 1.22 Introspection | **March–April** | Introspection endpoint with semantic ranking over tool definitions. Top-K pruning before prompt assembly. 80%+ token reduction at scale. Essential before MCP client goes live |
| P.4 | **Conversation Flight Recorder** | Low-Med | Turn/turn_tools schema | **April** | Full ReAct trace capture with replay, diff, and export (JSON, OTLP). Replay engine and compliance export layer. Complements 2.15 Ops Telemetry |

**Exit criteria**: MCP client consuming at least one external tool server end-to-end. OpenAPI importer generating a working Tool impl from a spec. Tool Search returning ranked results via introspection API. Flight Recorder exporting a full session trace.

### Wave 2 — Differentiators (April–June)

These create moat. Fits Tier 2 "New Capabilities".

| # | Plugin | Effort | Depends On | Target | What It Unlocks |
|---|---|---|---|---|---|
| P.5 | **Policy-as-Code** | Medium | Policy engine, authority levels, 1.2 Approval Workflow | **April–May** | Declarative behavior rules in a Cedar/Rego-like DSL. User-authored guardrails that compile to Ironclad's existing policy evaluation. The enterprise adoption unlock |
| P.6 | **Agent Delegation** | Medium | P.1 MCP client, ToolRegistry, 3.3 (partial) | **May** | One agent invokes a specialized sub-agent as a tool call with scoped context transfer. Delivers remaining scope of 3.3 Multi-Agent Orchestration (agent-as-tool invocation). Industry convergence point |
| P.7 | **Sensor Mesh** | Medium | Channel adapters, cron scheduler | **May–June** | Continuous data feeds (RSS, webhooks, MQTT, system metrics) as ambient agent context. Transforms agents from reactive to proactively aware |
| P.8 | **Adversarial Red Team** | Medium | 4-layer injection defense | **June** | Automated attack library (prompt injection, jailbreak, exfiltration) that tests agent defenses. Produces compliance scorecards. Turns security into a testable, provable feature |
| P.9 | **Container Execution** | Medium | Security design review, 2.7 WASM Runtime | **June** (design in April) | Heavyweight isolated environment (Docker/nsjail) for generated code needing system packages, file I/O, or network access. Complements 2.7 WASM sandbox |

**Exit criteria**: Policy-as-Code compiling and enforcing at least 3 rule types. Agent Delegation round-tripping with context isolation. Sensor Mesh ingesting at least one ambient feed. Red Team producing a scorecard. Container executing untrusted code with no host escape path.

### Wave 3 — Ecosystem (June–September)

Extends Ironclad's reach. Fits Tier 2/3 boundary.

| # | Plugin | Effort | Depends On | Target | What It Unlocks |
|---|---|---|---|---|---|
| P.10 | **Skill Forge** | Medium | 2.14 Skills Catalog, 2.21 Skill Registry Protocol | **June–July** | Trust scoring and dependency resolution layer on top of core publish/discover infrastructure (v0.9.6). Adds trust scores, semantic dependency resolution, `ironclad skill publish` with signed manifests |
| P.11 | **A2A v0.3 Bridge** | Medium | Existing zero-trust A2A (`ironclad-channels/a2a.rs`) | **August** | Google's Agent-to-Agent protocol (150+ orgs, Linux Foundation). Agent Cards for capability discovery. Complements proprietary A2A with standards-track interop |
| P.12 | **Workflow Graphs** | High | P.6, orchestration module | **September** (spec in June) | Graph-based orchestration with typed state, conditional branching, checkpoint/resume. Delivers remaining 3.3 scope (workflow patterns). Answers LangGraph's architectural model |

**Exit criteria**: Skill Forge trust scoring operational on at least 10 published skills. A2A v0.3 Bridge completing a capability discovery round-trip with a standards-compliant peer. At least one Workflow Graph executing with checkpoint/resume.

---

## Skill Detail

### Q1 2026 — Delivered Baseline + Trigger Quality Hardening

Focus: consolidate shipped skills, raise activation quality.

| Skill | Type | Status |
|---|---|---|
| `install-setup-assistant` | built-in | shipped |
| `runtime-diagnostics` | built-in | shipped |
| `provider-auth-troubleshooter` | built-in | shipped |
| `update-and-rollback` | optional | shipped |
| `session-operator` | optional | shipped |
| `skill-creation` | optional | shipped |
| `model-routing-tuner` | optional | shipped |
| `code-analysis-bug-hunting` | optional | shipped |

**Exit criteria**: Trigger precision/recall measured and tuned for all shipped skills. Reduced setup/auth/runtime support friction. No duplicate skill identities across built-in and optional catalogs.

### Q2 2026 — Operator Productivity and Day-2 Workflows

Focus: improve routine operations while v0.8.x reliability work stabilizes.

| Skill | What It Does | Plugin Dependency |
|---|---|---|
| `prompt-quality-auditor` | Prompt effectiveness scoring and improvement suggestions | — |
| `workflow-planner` | Multi-step runbook generation | P.6 Agent Delegation (enhanced) |
| `config-drift-checker` | Expected vs runtime behavior checks | — |
| `session-hygiene-manager` | Archive/cleanup/export cadence | — |
| `release-readiness-assistant` | Safe release preflight checks | — |

**Exit criteria**: Reduced time-to-tune for routing/prompt adjustments. Fewer manual config/release errors. At least two day-2 workflows usable end-to-end.

### Q3 2026 — Quality and Security Skill Pack

Focus: pre-merge defect detection and safer remediation loops.

| Skill | What It Does | Plugin Dependency |
|---|---|---|
| `fix-validation` | Verify bugfixes against regression tests | P.9 Container Execution |
| `security-misconfig-hunter` | Fail-open and insecure defaults detection | P.8 Adversarial Red Team |
| `test-gap-analyzer` | Coverage + missing-case recommendations | P.9 Container Execution |
| `incident-retrospective-writer` | Structured incident retrospectives | — |
| `dependency-risk-reviewer` | Dependency risk assessment | — |
| `code-analysis-bug-hunting` v2 | Quality pass (not a new ID) | — |

**Exit criteria**: Higher pre-merge defect interception rate. Better regression coverage. Downward trend in security-misconfiguration recurrence.

### Q4 2026 — Integrations and Control-Plane Specialization

Focus: ecosystem expansion and policy-heavy operations.

| Skill | What It Does | Plugin Dependency |
|---|---|---|
| `mcp-integration-helper` | MCP tool server setup and troubleshooting | P.1 MCP Gateway (client) |
| `plugin-troubleshooter` | Plugin diagnostics and repair | PluginRegistry |
| `cross-channel-operator` | Telegram/Discord/Signal/email support | Channel adapters |
| `policy-guardrail-designer` | Policy authoring guidance | P.5 Policy-as-Code |
| `cost-anomaly-investigator` | Inference cost anomaly detection | Cost telemetry |
| `knowledge-workflow-curator` | Knowledge management workflow design | — | **Note:** Core capture/synthesis loop shipped in v0.9.6 (learning.rs + learned_skills). Remaining scope is retrieval-hygiene: stale-entry pruning, dedup across procedural_memory + learned_skills, and cross-session knowledge graph indexing. |

**Exit criteria**: Faster integration setup and troubleshooting. Lower plugin/channel setup failure rate. Clear operator-facing policy/guardrail recommendations.

---

## How Plugins Unlock Skills

| Plugin | Enables Skills | Quarter |
|---|---|---|
| P.1 MCP Gateway (client) | `mcp-integration-helper` | Q4 |
| P.3 Tool Search | Accelerates all skills with large tool registries | Q1+ |
| P.5 Policy-as-Code | `policy-guardrail-designer` | Q4 |
| P.6 Agent Delegation | `workflow-planner` (enhanced) | Q2 |
| P.8 Adversarial Red Team | `security-misconfig-hunter` | Q3 |
| P.9 Container Execution | `fix-validation`, `test-gap-analyzer` | Q3 |
| P.10 Skill Forge | `skill-creation` v2 (publish workflow) | Q3+ |

---

## Plugin Dependency Graph

```
P.1 MCP Gateway (server: done, client: March)
  |
  +-- P.2 OpenAPI Tool Importer --- instant REST API wrapping
  |
  +-- P.3 Tool Search ------------- ships before tool count explodes
  |
  +-- P.4 Flight Recorder --------- observability / compliance
  |
  +-- P.5 Policy-as-Code ---------- enterprise guardrails (extends 1.2)
        |
        +-- P.6 Agent Delegation -- multi-agent convergence (delivers 3.3)
        |     |
        |     +-- P.12 Workflow Graphs -- graph orchestration (delivers 3.3)
        |
        +-- P.7 Sensor Mesh ------ proactive ambient context
        |
        +-- P.8 Adversarial Red Team -- testable security
        |
        +-- P.9 Container Execution -- heavyweight sandboxing (complements 2.7)
              |
              +-- P.10 Skill Forge -- trust layer on 2.14+2.21
                    |
                    +-- P.11 A2A v0.3 Bridge -- standards interop
```

---

## Skill Promotion Path (Optional to Built-in)

A skill is eligible for built-in promotion if:

- Triggered by majority user cohorts
- Demonstrates stable behavior for at least one quarter
- Has low false-trigger rate and high completion quality
- Addresses a high-risk workflow where mistakes are expensive
- Depends only on stable runtime APIs (not planning-only roadmap items)

Built-in promotion should lag at least one quarter behind optional rollout unless the workflow is outage-critical.

---

## Items Relocated to Embedded Roadmap

These items were originally in the plugin roadmap but fail the plugin test — they operate on core agent internals.

| Item | Why Not a Plugin | Where It Belongs |
|---|---|---|
| **Dream Mode** | Background cognitive processing on all 5 memory tiers — a heartbeat daemon extension | Embedded: v1.x Memory & Cognition track, depends on 2.12 Episodic Digest |
| **Trust Ladder** | Dynamic reputation modifies A2A protocol and policy engine — both core systems | Embedded: v1.x after P.11 A2A v0.3 Bridge stabilizes |
| **Temporal Reasoner** | Time-aware memory queries extend retrieval system (2.12) — core memory tier infrastructure | Embedded: v1.x Memory & Cognition track |
| **Context Genome** | Session-to-session transfer learning operates on memory tiers + agent loop internals | Embedded: v1.x after Dream Mode ships |

### Removed from consideration

| Candidate | Reason |
|---|---|
| Inference Arbitrage | Redundant with existing tiered inference (v0.9.1) |
| Memory Federation | Over-engineered — 5-tier memory system is already leading |
| Interactive UI / MCP Apps | We're not a chat UI host; revisit if web dashboard ships |

---

## Governance

### Monthly (all items)

1. Check plugin and skill delivery against schedule targets
2. Validate cross-dependencies are not blocking downstream items
3. Re-assess priority based on competitive moves and operator demand
4. Gate Wave 3 plugins on Wave 1+2 stability

### Quarterly (skills)

1. Review adoption, trigger-hit rates, and false-trigger rates
2. Promote/demote built-in candidates based on stability and reach
3. Publish roadmap deltas in release notes and docs

### Quarterly (plugins)

1. Evaluate relocated items (Dream Mode, Trust Ladder, Temporal Reasoner, Context Genome) for embedded roadmap scheduling
2. Review whether any skills should be promoted to plugins or vice versa

---

## Alignment Constraints

- v0.8.x remains reliability-first (durable delivery, abuse protection, cron-conformant rotation)
- Skills depending on future platform work should be discovery/spec-first until runtime surfaces are stable
- Guardrail-sensitive skills stay conservative until abuse-protection and rotation semantics are stable in production
- High-autonomy orchestration skills remain optional until sustained stability data is available

---

## Audit Trail

### March 2026 — Plugin placement audit

Cross-referenced all 16 original plugin candidates against skills roadmap, skill registries (`builtin-skills.json`, `manifest.json`), and embedded roadmap (`ROADMAP.md`, `v0.9.x-series.md`).

Results:
- **8 correctly placed** (P.1–P.8)
- **4 relocated to embedded** (Dream Mode, Trust Ladder, Temporal Reasoner, Context Genome) — fail plugin test
- **2 rescoped** (P.9 narrowed to container-only since 2.7 WASM exists; P.10 narrowed to trust/dep-resolution layer since 2.14+2.21 cover core publish/discover)
- **1 high overlap documented** (P.10 vs 2.14+2.21 — resolved by scoping P.10 as trust layer on top)
- **1 borderline kept** (P.4 Flight Recorder — could be embedded with 2.15, but replay engine is self-contained enough for plugin)

Net: 16 -> 12 true plugins.

### March 2026 — Roadmap unification

Merged `plugins-roadmap-2026.md` and `skills-roadmap-2026.md` into this unified document (`capabilities-roadmap-2026.md`) to eliminate cross-document confusion.

---

## Competitive Research Sources

- [Legacy](https://legacy.ai/) — local-first AI agent, 140K+ stars, SkillHub marketplace
- [Legacy Architecture (DeepWiki)](https://deepwiki.com/legacy/legacy/3-agents) — agent/heartbeat/workspace model
- [Legacy Skills Guide (DigitalOcean)](https://www.digitalocean.com/resources/articles/what-are-legacy-skills) — skill.md format, SkillHub ecosystem
- [Legacy Wikipedia](https://en.wikipedia.org/wiki/Legacy) — project history and community metrics
