# New Skill Prioritization (Majority Users)

## Method

Prioritization is based on user-facing workflows that appear repeatedly in:

- `README.md` quick start and operational guidance
- `docs/CLI.md` command surface
- `docs/DEPLOYMENT.md` setup/operations runbooks
- `docs/ENV.md` provider auth and credential precedence
- `ironclad-site` install and CLI docs pages

Scoring dimensions:

- **Reach**: how many users encounter this workflow
- **Impact**: user pain reduced when skill exists
- **Frequency**: how often users repeat the workflow
- **Risk**: severity when users make mistakes manually

## Wave 1 (Shipped Baseline)

Wave 1 is now present in code/registry and should be treated as adoption + quality-hardening scope (not net-new delivery scope).

### Built-in skills (always available)

1. `install-setup-assistant`
   - Problem: first-run install/init/setup confusion
   - Triggers: install, setup, init workspace, first run
   - Acceptance:
     - Produces minimal install path for user OS
     - Includes post-install verification steps
     - Includes recovery path for failed setup

2. `runtime-diagnostics`
   - Problem: unclear failures in health/logs/breaker/runtime state
   - Triggers: debug runtime, health check, logs, breaker state
   - Acceptance:
     - Triages health endpoint, logs, and breaker status in one flow
     - Produces likely root-cause classes
     - Produces actionable next command(s)

3. `provider-auth-troubleshooter`
   - Problem: provider key/auth precedence and login errors
   - Triggers: auth failed, API key issues, provider login
   - Acceptance:
     - Validates env/keystore/ref precedence
     - Distinguishes auth errors vs transport/model errors
     - Recommends least-risk credential fix

4. `introspection`
   - Problem: inconsistent self-awareness of available skills/tools/routes
   - Triggers: what tools can you use, what skills are available, introspect capabilities
   - Acceptance:
     - Lists currently available tools/skills without fabrication
     - States capability limits before claiming execution
     - Uses runtime status surfaces for verification when available

### Downloadable/optional skills (shipped)

1. `skill-creation`
   - Problem: users struggle to author well-triggered custom skills
   - Triggers: create skill, custom skill, skill template
   - Acceptance:
     - Produces valid frontmatter and instruction body
     - Includes trigger quality checks and sample test prompts

2. `model-routing-tuner`
   - Problem: unclear cost/latency/quality tuning trade-offs
   - Triggers: route model, tune routing, reduce cost, reduce latency
   - Acceptance:
     - Captures constraints and proposes measurable routing changes
     - Defines rollback guard if quality regresses

3. `update-and-rollback`
   - Problem: unsafe upgrades without reliable rollback
   - Triggers: update ironclad, rollback release, restore backup
   - Acceptance:
     - Requires backup and version checkpoint before upgrade
     - Produces deterministic rollback sequence

4. `session-operator`
   - Problem: context/session lifecycle mistakes
   - Triggers: export session, resume conversation, list sessions
   - Acceptance:
     - Provides minimum-step workflow for session goal
     - Includes continuity checks before destructive operations

5. `code-analysis-bug-hunting`
   - Problem: users need high-signal correctness/regression analysis
   - Triggers: bug hunt, code analysis, find regressions
   - Acceptance:
     - Ranks findings by severity
     - Explains impact and minimal fix path
     - Notes highest-value missing tests

## Next-Wave Candidates (Prioritized)

These are ordered to match the updated 2026 roadmap and current platform sequencing.

- `prompt-quality-auditor` (prompt robustness and ambiguity checks)
- `workflow-planner` (multi-step runbook generation)
- `config-drift-checker` (expected vs runtime behavior checks)
- `session-hygiene-manager` (session lifecycle cleanup/export hygiene)
- `release-readiness-assistant` (operator preflight and rollback discipline)
- `fix-validation` (regression-proof bugfix verification)
- `security-misconfig-hunter` (fail-open and insecure-default detection)
- `test-gap-analyzer` (coverage + missing-case recommendations)
- `plugin-troubleshooter` (plugin loading/permissions diagnosis)
- `policy-guardrail-designer` (approval/policy strategy authoring)
- `mcp-integration-helper` (external tool/service MCP integration)
- `incident-retrospective-writer` (post-incident synthesis)

## Dependency/Sequencing Notes

- Prioritize skills that can ship against stable `v0.8.x` reliability surfaces first.
- For skills depending on broader `v0.9.x` platform work (for example deeper orchestration or advanced telemetry), run discovery/spec-first and defer built-in promotion.
- Prefer CLI-first delivery where dashboard/runtime surfaces are still in flux.

## Built-in vs Optional Policy

Promote a skill to built-in when all are true:

1. Workflow is first-run or outage-recovery critical
2. Reach is broad (majority users)
3. Incorrect manual handling causes high-impact failures
4. Behavior is stable enough to avoid rapid churn
5. Runtime dependencies are stable (not planning-only roadmap items)

Keep optional when:

- workflow is advanced/tuning-heavy
- capability evolves quickly
- audience is narrower than majority users
