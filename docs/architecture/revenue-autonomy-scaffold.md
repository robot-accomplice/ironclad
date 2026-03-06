# Revenue Autonomy Scaffold (v0.9.5)

## Purpose

Define a single, auditable control plane for autonomous revenue pursuit so each strategy (paid services, micro-bounty, data feeds) reuses the same intake, gating, execution, and settlement path.

## Core Principles

- One canonical lifecycle for all revenue jobs.
- No strategy-specific bypasses around policy, authority, or settlement.
- Every completed job must have an evidence bundle and ledger trail.
- Reliability and behavioral correctness are prioritized over raw throughput.

## Canonical Lifecycle

1. Opportunity Intake
2. Qualification Gate
3. Economic Scoring
4. Execution Planning
5. Fulfillment + Evidence Capture
6. Settlement + Ledger
7. Feedback + Auto-tuning

## Runtime Components

### 1) Opportunity Intake

**Responsibility**: normalize incoming opportunities into one schema.

**Normalized fields**:

- `job_id`
- `source` (`service_api`, `bounty_board`, `oracle_feed`, `internal_schedule`)
- `strategy`
- `expected_revenue_usdc`
- `deadline`
- `required_skills`
- `proof_requirements`

### 2) Qualification Gate

**Responsibility**: hard fail unsafe/invalid opportunities before execution.

Checks:

- authority and policy eligibility
- workspace boundary compliance
- tool availability
- compliance tags (allowed/disallowed category)
- maximum risk threshold

### 3) Economic Scoring

**Responsibility**: rank opportunities by expected net value, not gross value.

Score baseline:

`score = (expected_net_usdc * confidence) - (latency_penalty + failure_risk + policy_risk)`

### 4) Execution Planner

**Responsibility**: produce deterministic execution plans.

Plan fields:

- `executor` (`self` or named subagent)
- `model_policy` (primary + fallback constraints)
- `max_runtime_ms`
- `max_cost_usdc`
- `retry_budget`

### 5) Fulfillment + Evidence Capture

**Responsibility**: prove execution quality and correctness.

Evidence bundle:

- invoked tools/commands
- relevant output excerpts
- artifact hashes
- external source references (if applicable)
- acceptance checks and result status

### 6) Settlement + Ledger

**Responsibility**: produce immutable, idempotent financial outcomes.

Records:

- gross revenue
- attributable costs
- net realized profit
- tax allocation (if enabled)
- retained earnings

### 7) Feedback + Auto-tuning

**Responsibility**: continuously improve strategy/model/task routing.

Tracked outcomes:

- win/loss by strategy
- payout delay
- failure modes by model/provider
- quality score by job type

## Phase 1 Implementation (Current Branch)

Minimal production slice implemented in server API:

- single paid service type: `geopolitical-sitrep-verified`
- `POST /api/services/quote`
- `POST /api/services/requests/{id}/payment/verify`
- `POST /api/services/requests/{id}/fulfill`
- `GET /api/services/requests/{id}`
- `GET /api/services/catalog`

Persistence:

- `service_requests` table (status transitions: `quoted -> payment_verified -> completed`)
- revenue ledger entry via `transactions` (`tx_type = service_revenue`)

## Acceptance Criteria

- Service flow survives restart (DB-backed state)
- Invalid state transitions are rejected
- Payment verification requires recipient and amount match quote
- Fulfillment allowed only after payment verification
- Transactions ledger contains revenue events for paid requests
- Integration tests cover quote -> verify -> fulfill end-to-end

## Deferred to Next Slice

- on-chain payment proof verification (currently shape + quote-matching verification)
- multi-service catalog from config
- automated micro-bounty intake adapters
- profitability-aware strategy scheduler
- tax destination transfers for realized profit
