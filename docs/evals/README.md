# Model Runner Eval Starter Pack

This folder is the **v1 baseline matrix** for evaluating local and cloud model runners using `docs/MODEL_RUNNER_EVAL_TEMPLATE.md`.

## Files

- `runner_properties.baseline.csv`: one row per `(runner, model)` with execution/property fields.
- `task_efficacy.baseline.csv`: one row per `(runner, model, task_class)` for efficacy scoring.
- `task_weights.v1.csv`: default metric weights by task class.
- `CLOUD_BENCHMARK_SOURCES.md`: provenance for cloud baseline performance/cost values.
- `THIRD_PARTY_MODEL_BENCHMARKS.csv`: prefilled third-party model benchmarks (partial rows allowed).
- `CLOUD_BASELINE_REFRESH_SPEC.md`: monthly auto-refresh behavior and runbook.

## Included Runner Families

- Local/self-hosted: `sglang`, `vllm`, `ollama`, `docker_model_runner`, `airllm`
- Cloud/API: `openai_responses`, `anthropic_messages`, `google_vertex_openai`, `groq_openai`, `mistral_api`

## Locality Classes

- `local`: execution and data stay on operator-controlled host.
- `region_bound_cloud`: cloud execution constrained to approved region(s).
- `global_cloud`: cloud execution without strict region pinning.
- `hybrid`: mixed local and cloud path.

## Usage

1. Duplicate the baseline CSVs per eval run (`2026Q1-deep-001`, etc.).
2. Fill measured metrics from benchmark harness output.
3. Keep unknown fields empty until measured.
4. Store provenance (`eval_run_id`, dataset version, policy version) in `task_efficacy` rows.

## Prefill Policy

- Populate from third-party sources whenever values are available and methodologically transparent.
- Keep runner-specific measured fields blank if only model-level public values exist.
- Prefer partial provenance-backed rows over guessed values.
- When importing model-level indices (for example, AA Intelligence Index), store them in provenance fields (such as `dataset_version`) unless they map cleanly to an existing metric column.

## Monthly Refresh

- Dry-run: `scripts/refresh-cloud-baselines.sh`
- Apply: `scripts/refresh-cloud-baselines.sh --write`
