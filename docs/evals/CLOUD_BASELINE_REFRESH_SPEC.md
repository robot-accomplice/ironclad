# Cloud Baseline Auto-Refresh Spec (v1)

## Goal

Prevent stale cloud benchmark baselines by refreshing provider-side latency/speed/price values once per month.

## Script

- Path: `scripts/refresh-cloud-baselines.sh`
- Modes:
  - default: dry-run (parse + print proposed updates)
  - `--write`: apply updates in-place

## Inputs

- Public benchmark pages (currently Artificial Analysis provider pages)
- Existing baseline files in `docs/evals/`

## Outputs

When `--write` is used, the script updates:

- `docs/evals/runner_properties.baseline.csv`
  - `runner_version` set to `aa_snapshot_<YYYY-MM-DD>`
  - `ttft_ms`, `p50_token_ms`, `tokens_per_sec`
  - `cost_per_1k_tokens_usd` when blended price is parseable
- `docs/evals/task_efficacy.baseline.csv`
  - `eval_run_id` set to `AA-<YYYY-MM-DD>`
  - cloud row `ttft_ms`, `tokens_per_sec`
- `docs/evals/CLOUD_BENCHMARK_SOURCES.md`
  - `Snapshot date` line

## Current Cloud Rows Covered

- `openai_responses` (`gpt-5-mini-minimal`)
- `anthropic_messages` (`claude-4-5-sonnet`)
- `google_vertex_openai` (`gemini-2-5-pro`)
- `groq_openai` (`llama-3-3-instruct-70b`)
- `mistral_api` (`mistral-large-3`)

## Known Caveats

- Some provider pages only expose “top-N” values in textual summaries; specific provider price may not always be parseable.
- `groq_openai` price is intentionally left unchanged if provider-specific blended price is not extractable from stable text.
- Public benchmark methodologies can change; parser patterns may need periodic adjustment.

## Monthly Runbook

1. Dry-run:
   - `scripts/refresh-cloud-baselines.sh`
2. Review proposed metric deltas.
3. Apply:
   - `scripts/refresh-cloud-baselines.sh --write`
4. Validate:
   - confirm CSV formatting and spot-check values against source pages.
5. Commit doc/data refresh in one changeset.

## Future Hardening

- Add CI check that warns when snapshot date is older than 45 days.
- Add secondary source adapters and confidence tiers in `THIRD_PARTY_MODEL_BENCHMARKS.csv`.
