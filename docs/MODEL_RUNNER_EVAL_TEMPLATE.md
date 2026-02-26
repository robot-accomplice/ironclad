# Model Runner Evaluation Template (v1)

Use this template to evaluate **all model runners** (for example: SGLang, vLLM, Ollama, AirLLM-class, Docker Model Runner) with one consistent rubric that can feed `2.19` metascore routing.

This document is intentionally implementation-agnostic and planning-first.

## 1) Scope

Evaluate each runner across two planes:

- **Task efficacy plane**: how well model+runner pairs perform on task classes.
- **Execution properties plane**: latency, throughput, stability, cost, locality, and operational constraints.

## 2) Canonical Task Taxonomy

Use these task classes as required minimums:

- `coding`
- `summarization`
- `extraction`
- `planning`
- `tool_use`
- `classification`
- `translation`
- `safety_refusal`

Optional domain slices:

- `mission_critical_<domain>` (for example, `mission_critical_security_ops`)

## 3) Required Metrics

For each `(runner, model, task_class)` tuple, collect:

- **Quality**: `accuracy`, `pass_at_k` (if applicable), `schema_adherence`, `factuality_score`, `refusal_policy_score`
- **Latency/throughput**: `ttft_ms`, `p50_token_ms`, `p95_token_ms`, `tokens_per_sec`
- **Reliability**: `success_rate`, `error_rate`, `timeout_rate`, `retry_success_rate`
- **Resource/cost**: `vram_peak_mb`, `ram_peak_mb`, `disk_io_mb_s`, `cost_per_1k_tokens_usd` (or local estimated equivalent)
- **Locality/availability**: `locality_class`, `availability_score`, `cold_start_ms`

## 4) Normalization Rules (0..1)

Convert every raw metric to a normalized score where higher is better:

- Positive metric (higher is better), example `accuracy`:
  - `norm = clamp((x - min) / (max - min), 0, 1)`
- Negative metric (lower is better), example `p95_token_ms`:
  - `norm = clamp((max - x) / (max - min), 0, 1)`

Use fixed benchmark bands per metric per quarter to avoid score drift.

## 5) Scoring Formula

Hard constraints gate first, then weighted score:

```text
if not satisfies_hard_constraints(task_requirements, runner_profile):
    final_score = 0
else:
    efficacy_score = sum(task_metric_weight_i * task_metric_norm_i)
    runner_score   = sum(runner_metric_weight_j * runner_metric_norm_j)
    final_score    = alpha * efficacy_score + beta * runner_score
```

Default weights:

- `alpha = 0.65` (task efficacy emphasis)
- `beta = 0.35` (runner property emphasis)

Tie-breakers in order:

1. Higher `final_score`
2. Higher `availability_score`
3. Lower `p95_token_ms`
4. Lower `cost_per_1k_tokens_usd`
5. Deterministic lexical key (`runner_id:model_id`)

## 6) CSV Templates

Starter baseline matrices (local + cloud) live in `docs/evals/`:

- `docs/evals/runner_properties.baseline.csv`
- `docs/evals/task_efficacy.baseline.csv`
- `docs/evals/task_weights.v1.csv`
- `docs/evals/THIRD_PARTY_MODEL_BENCHMARKS.csv`

### 6.1 `runner_properties.csv`

```csv
runner_id,runner_version,model_id,model_version,locality_class,supports_tools,supports_streaming,max_context,ttft_ms,p50_token_ms,p95_token_ms,tokens_per_sec,success_rate,error_rate,timeout_rate,retry_success_rate,vram_peak_mb,ram_peak_mb,disk_io_mb_s,cost_per_1k_tokens_usd,availability_score,cold_start_ms,notes
sglang,0.9.2,meta-llama/Llama-3.1-70B-Instruct,2025-12-01,local,true,true,131072,850,42,120,24.0,0.992,0.004,0.003,0.81,28600,9400,180,0.00,0.97,6200,"baseline local high-throughput"
```

### 6.2 `task_efficacy.csv`

```csv
eval_run_id,runner_id,model_id,task_class,dataset_id,dataset_version,sample_count,accuracy,pass_at_k,schema_adherence,factuality_score,refusal_policy_score,ttft_ms,p95_token_ms,tokens_per_sec,success_rate
2026Q1-deep-001,sglang,meta-llama/Llama-3.1-70B-Instruct,coding,humaneval-lite,v3,164,0.61,0.74,0.93,0.00,0.00,900,125,23.4,0.99
```

### 6.3 `task_weights.csv`

```csv
task_class,metric,weight
coding,accuracy,0.35
coding,pass_at_k,0.35
coding,schema_adherence,0.10
coding,p95_token_ms,0.10
coding,success_rate,0.10
```

## 7) JSON Template (Routing-Ready)

```json
{
  "eval_run_id": "2026Q1-deep-001",
  "runner_id": "sglang",
  "runner_version": "0.9.2",
  "model_id": "meta-llama/Llama-3.1-70B-Instruct",
  "task_class": "coding",
  "constraints": {
    "require_locality": "local",
    "max_p95_token_ms": 250,
    "min_success_rate": 0.98
  },
  "normalized_metrics": {
    "accuracy": 0.82,
    "pass_at_k": 0.79,
    "p95_token_ms": 0.67,
    "success_rate": 0.94,
    "availability_score": 0.91
  },
  "weights": {
    "alpha": 0.65,
    "beta": 0.35
  },
  "scores": {
    "efficacy_score": 0.78,
    "runner_score": 0.73,
    "final_score": 0.76
  },
  "provenance": {
    "dataset_id": "humaneval-lite",
    "dataset_version": "v3",
    "scoring_policy_version": "v1",
    "generated_at": "2026-02-25T00:00:00Z"
  }
}
```

## 8) Runner Property Checklist

For each runner, explicitly record:

- install complexity (`easy`, `moderate`, `advanced`)
- platform coverage (`linux`, `macos_apple_silicon`, `windows`, `colab`)
- tool-calling compatibility
- structured output reliability
- long-context behavior
- batch behavior
- observability hooks
- failure modes and recovery path

## 9) Benchmark Cadence

- Monthly: smoke pass on all runners and all task classes.
- Quarterly: deep pass with confidence intervals and mission-critical slices.
- Triggered rerun: runner upgrade, model upgrade, or routing policy change.

## 10) Output Contract for `2.19`

This template produces the canonical inputs for:

- `ModelProfile.known_efficacy_by_task_type`
- runner reliability and latency priors
- dashboard explainability ("why this runner/model was selected")
- simulation mode ("what changes if weights/constraints change")
