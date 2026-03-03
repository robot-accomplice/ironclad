# Ironclad Skills Roadmap 2026

## Scope

This roadmap schedules at least 12 months of skill delivery with quarterly review gates.

Cadence:

- Monthly releases (target: 2-3 skills/month)
- Quarterly re-ranking based on telemetry/support demand

## Quarterly Plan

### Q1 2026 - Onboarding and Reliability Foundations

Focus: reduce setup friction and failure-to-resolution time.

- `install-setup-assistant` (built-in)
- `runtime-diagnostics` (built-in)
- `provider-auth-troubleshooter` (built-in)
- `update-and-rollback` (optional)
- `session-operator` (optional)
- `skill-creation` (optional)

Exit criteria:

- Median setup completion time reduced
- Support tickets for auth/setup/runtime startup down quarter-over-quarter
- All above skills pass trigger quality checks and regression tests

### Q2 2026 - Productivity and Control Plane Workflows

Focus: improve day-2 operations and tuning.

- `model-routing-tuner`
- `prompt-quality-auditor`
- `workflow-planner` (multi-step runbook generation)
- `config-drift-checker` (compare expected vs runtime config behavior)
- `session-hygiene-manager` (archive/cleanup/export cadence)
- `release-readiness-assistant` (safe release preflight for operators)

Exit criteria:

- Reduced time-to-tune for model routing changes
- Fewer manual config mistakes in support channels
- At least two workflows fully automatable end-to-end

### Q3 2026 - Quality, Security, and Testing Skills

Focus: harden code and operational safety.

- `code-analysis-bug-hunting`
- `fix-validation` (verify bugfixes against regression tests)
- `security-misconfig-hunter` (fail-open and insecure defaults)
- `test-gap-analyzer` (coverage + missing-case recommendations)
- `incident-retrospective-writer`
- `dependency-risk-reviewer`

Exit criteria:

- Increased pre-merge defect detection
- Improved regression test coverage for production issues
- Security-related misconfiguration findings trend down

### Q4 2026 - Advanced Orchestration and Integrations

Focus: ecosystem expansion and autonomous operations.

- `mcp-integration-helper`
- `plugin-troubleshooter`
- `cross-channel-operator` (telegram/discord/signal/email workflow support)
- `policy-guardrail-designer`
- `cost-anomaly-investigator` (inference cost spikes)
- `knowledge-workflow-curator` (capture/index/retrieval hygiene)

Exit criteria:

- Faster integration time for external systems
- Reduced plugin/channel setup failures
- Measurable improvement in operator confidence on guardrails

## Month-by-Month Release Skeleton

- Jan: install-setup-assistant, runtime-diagnostics
- Feb: provider-auth-troubleshooter, update-and-rollback
- Mar: session-operator, skill-creation
- Apr: model-routing-tuner, prompt-quality-auditor
- May: workflow-planner, config-drift-checker
- Jun: session-hygiene-manager, release-readiness-assistant
- Jul: code-analysis-bug-hunting, fix-validation
- Aug: security-misconfig-hunter, test-gap-analyzer
- Sep: incident-retrospective-writer, dependency-risk-reviewer
- Oct: mcp-integration-helper, plugin-troubleshooter
- Nov: cross-channel-operator, policy-guardrail-designer
- Dec: cost-anomaly-investigator, knowledge-workflow-curator

## Governance

Each quarter:

1. Review adoption and trigger-hit rates
2. Re-score roadmap items by impact and implementation cost
3. Promote/demote built-in candidates based on stability and reach
4. Publish roadmap delta in release notes and docs

## Promotion Path (Optional to Built-in)

A skill is eligible for built-in promotion if:

- Triggered by majority user cohorts
- Demonstrates stable behavior over at least one quarter
- Has low false-trigger rate and high task completion quality
- Addresses a high-risk workflow where mistakes are expensive
