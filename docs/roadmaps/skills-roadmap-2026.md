# Ironclad Skills Roadmap 2026

## Scope

This roadmap tracks one year of skill delivery, aligned to actual runtime/registry state and the core project roadmap.

Cadence:

- Monthly releases (target: 0-2 net-new skills/month, plus quality updates)
- Quarterly re-ranking based on telemetry and support demand

## Current Baseline (Already in Code/Registry)

From `registry/builtin-skills.json` and `registry/manifest.json`:

- Built-in now: `install-setup-assistant`, `runtime-diagnostics`, `provider-auth-troubleshooter`
- Optional now: `update-and-rollback`, `session-operator`, `skill-creation`, `model-routing-tuner`, `code-analysis-bug-hunting`

From runtime/CLI:

- Catalog surfaces exist in API and CLI (`/api/skills/catalog*`, `ironclad skills catalog *`)
- Skill install/activate/reload/list flows are already wired

This means Q1 is no longer a pure "future plan"; it is mostly shipped and should focus on quality and adoption.

## Alignment Constraints from Project Roadmap

- `v0.8.x` remains reliability-first (durable delivery, abuse protection, cron-conformant rotation, skills catalog UX hardening)
- Skills that depend on future platform work (for example deep plugin diagnostics, broad orchestration, advanced cost telemetry) should be discovery/spec-first until required runtime surfaces are stable
- Built-in promotion should lag at least one quarter behind optional rollout unless the workflow is outage-critical

## Quarterly Plan

### Q1 2026 - Delivered Baseline + Trigger Quality Hardening

Focus: consolidate what is already shipped and raise activation quality.

- `install-setup-assistant` (built-in) - shipped
- `runtime-diagnostics` (built-in) - shipped
- `provider-auth-troubleshooter` (built-in) - shipped
- `update-and-rollback` (optional) - shipped
- `session-operator` (optional) - shipped
- `skill-creation` (optional) - shipped
- `model-routing-tuner` (optional) - shipped
- `code-analysis-bug-hunting` (optional) - shipped

Exit criteria:

- Trigger precision/recall measured and tuned for all shipped Wave 1 skills
- Reduced setup/auth/runtime support friction versus prior quarter
- No duplicate skill identities across built-in and optional catalogs

### Q2 2026 - Operator Productivity and Day-2 Workflows

Focus: improve routine operations while `v0.8.x` reliability work stabilizes.

- `prompt-quality-auditor`
- `workflow-planner` (multi-step runbook generation)
- `config-drift-checker` (expected vs runtime behavior checks)
- `session-hygiene-manager` (archive/cleanup/export cadence)
- `release-readiness-assistant` (safe release preflight)

Dependency notes:

- Favor CLI-first where dashboard/runtime surfaces are still evolving
- Keep guardrail-sensitive skills conservative until abuse-protection and rotation semantics are stable in production

Exit criteria:

- Reduced time-to-tune for routing/prompt adjustments
- Fewer manual config/release errors in operator workflows
- At least two day-2 workflows usable end-to-end from skill invocation

### Q3 2026 - Quality and Security Skill Pack

Focus: pre-merge defect detection and safer remediation loops.

- `fix-validation` (verify bugfixes against regression tests)
- `security-misconfig-hunter` (fail-open and insecure defaults)
- `test-gap-analyzer` (coverage + missing-case recommendations)
- `incident-retrospective-writer`
- `dependency-risk-reviewer`
- `code-analysis-bug-hunting` v2 (quality pass, not a new ID)

Exit criteria:

- Higher pre-merge defect interception rate
- Better regression coverage for production issue classes
- Downward trend in security-misconfiguration recurrence

### Q4 2026 - Integrations and Control-Plane Specialization

Focus: skills that support ecosystem expansion and policy-heavy operations.

- `mcp-integration-helper`
- `plugin-troubleshooter`
- `cross-channel-operator` (telegram/discord/signal/email support)
- `policy-guardrail-designer`
- `cost-anomaly-investigator`
- `knowledge-workflow-curator`

Dependency notes:

- Promote from discovery to implementation only when required runtime surfaces are stable (`v0.9.x` track)
- Keep high-autonomy orchestration skills optional until sustained stability data is available

Exit criteria:

- Faster integration setup and troubleshooting cycles
- Lower plugin/channel setup failure rate
- Clear operator-facing explanations for policy/guardrail recommendations

## Month-by-Month Release Skeleton

- Jan: shipped baseline built-ins (`install-setup-assistant`, `runtime-diagnostics`)
- Feb: shipped `provider-auth-troubleshooter`, `update-and-rollback`
- Mar: shipped `session-operator`, `skill-creation`, `model-routing-tuner`, `code-analysis-bug-hunting`
- Apr: `prompt-quality-auditor`
- May: `workflow-planner`
- Jun: `config-drift-checker`, `session-hygiene-manager`
- Jul: `release-readiness-assistant`, `fix-validation`
- Aug: `security-misconfig-hunter`
- Sep: `test-gap-analyzer`, `incident-retrospective-writer`
- Oct: `dependency-risk-reviewer`, `mcp-integration-helper`
- Nov: `plugin-troubleshooter`, `policy-guardrail-designer`
- Dec: `cross-channel-operator`, `cost-anomaly-investigator` (or `knowledge-workflow-curator` if dependency gates are not met)

## Governance

Each quarter:

1. Review adoption, trigger-hit rates, and false-trigger rates
2. Re-score roadmap items by impact, implementation cost, and dependency readiness
3. Promote/demote built-in candidates based on stability and reach
4. Publish roadmap deltas in release notes and docs

## Promotion Path (Optional to Built-in)

A skill is eligible for built-in promotion if:

- Triggered by majority user cohorts
- Demonstrates stable behavior for at least one quarter
- Has low false-trigger rate and high completion quality
- Addresses a high-risk workflow where mistakes are expensive
- Depends only on stable runtime APIs (not planning-only roadmap items)
