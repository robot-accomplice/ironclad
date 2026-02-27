# Metascore v1 Scoring Contract (Spec-Only)

This document defines a deterministic scoring contract for model routing based on the evaluation matrices in `docs/evals/`.

Status: planning/spec only. No implementation is implied by this document.

## 1) Purpose

Given a task and candidate runner/model profiles, produce:

- ranked candidates
- one selected candidate
- explainable contribution breakdown
- simulation diff for alternative policy weights/constraints

## 2) Inputs

Required inputs:

- `task_requirements` (hard constraints + soft preferences)
- `runner_properties` rows (from `runner_properties.baseline.csv` or run-specific snapshot)
- `task_efficacy` rows (run-specific task performance)
- `task_weights` (from `task_weights.v1.csv` or override policy)
- optional model-level priors (from `THIRD_PARTY_MODEL_BENCHMARKS.csv`)

## 3) Hard-Constraint Gate

A candidate is ineligible (`eligible=false`) if any hard constraint fails:

- locality mismatch (`require_locality`)
- budget cap exceeded (`max_cost_per_1k_tokens_usd`)
- latency ceiling exceeded (`max_p95_token_ms`)
- minimum reliability unmet (`min_success_rate`)
- context requirement unmet (`min_context_tokens`)

Ineligible candidates get `final_score=0` and are listed with explicit fail reasons.

## 4) Normalization

All scored metrics are normalized to `[0, 1]`.

- positive metric (higher is better):  
  `norm = clamp((x - min) / (max - min), 0, 1)`
- negative metric (lower is better):  
  `norm = clamp((max - x) / (max - min), 0, 1)`

Normalization bands are policy-versioned and fixed for a scoring window.

## 5) Core Score

For each eligible candidate:

```text
efficacy_score = sum(task_metric_weight_i * norm_i)
runner_score   = sum(runner_metric_weight_j * norm_j)
prior_score    = optional prior boost/penalty from model-level benchmark confidence

final_score = alpha * efficacy_score
            + beta  * runner_score
            + gamma * prior_score
```

Default coefficients:

- `alpha = 0.65`
- `beta = 0.30`
- `gamma = 0.05`

## 6) Tie-Breakers (Deterministic)

Applied in order:

1. higher `final_score`
2. higher `availability_score`
3. lower `p95_token_ms`
4. lower `cost_per_1k_tokens_usd`
5. lexical `(runner_id:model_id)`

## 7) Output Payload

```json
{
  "scoring_version": "metascore_v1",
  "policy_version": "weights_v1",
  "selected": {
    "runner_id": "google_vertex_openai",
    "model_id": "gemini-2-5-pro",
    "final_score": 0.81
  },
  "ranked": [
    {
      "runner_id": "google_vertex_openai",
      "model_id": "gemini-2-5-pro",
      "eligible": true,
      "final_score": 0.81,
      "components": {
        "efficacy_score": 0.79,
        "runner_score": 0.84,
        "prior_score": 0.75
      },
      "contributions": {
        "accuracy": 0.18,
        "schema_adherence": 0.11,
        "success_rate": 0.14,
        "p95_token_ms": 0.09,
        "cost_per_1k_tokens_usd": 0.08,
        "availability_score": 0.07
      },
      "constraint_results": {
        "require_locality": "pass",
        "max_cost_per_1k_tokens_usd": "pass",
        "max_p95_token_ms": "pass",
        "min_success_rate": "pass",
        "min_context_tokens": "pass"
      }
    }
  ],
  "ineligible": [
    {
      "runner_id": "airllm",
      "model_id": "llama-3-1-instruct-70b",
      "final_score": 0,
      "fail_reasons": ["max_p95_token_ms"]
    }
  ]
}
```

## 8) Simulation Contract

Simulation runs use the same scorer with hypothetical inputs:

- alternative `task_requirements`
- alternative coefficient set (`alpha`, `beta`, `gamma`)
- alternative task metric weights

Simulation output adds:

- `baseline_selected`
- `simulated_selected`
- `selection_changed` boolean
- top-N delta table with per-metric contribution diffs

## 9) Confidence and Provenance

Each candidate score carries:

- `data_freshness_days`
- `source_mix` (`measured`, `third_party`, `hybrid`)
- `confidence_tier` (`high`, `medium`, `partial`)
- source pointers (`eval_run_id`, benchmark snapshot date, policy version)

## 10) Non-Goals (v1)

- online learning/auto-weight tuning
- probabilistic/Bayesian selection
- dynamic policy mutation during active request handling

These can be added in future versions (`metascore_v2+`).
