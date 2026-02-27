# Cloud Benchmark Sources (Baseline Snapshot)

Snapshot date: `2026-02-25`

Primary source: Artificial Analysis model + provider benchmarking pages (performance medians and blended pricing).

## Mappings Used

- `openai_responses` + `gpt-5-mini-minimal`
  - model: `https://artificialanalysis.ai/models/gpt-5-mini-minimal`
  - providers: `https://artificialanalysis.ai/models/gpt-5-mini-minimal/providers`
  - used values: `ttft=0.66s`, `output_speed=67.9 t/s`, `blended_price=$0.69 / 1M`

- `anthropic_messages` + `claude-4-5-sonnet`
  - model: `https://artificialanalysis.ai/models/claude-4-5-sonnet`
  - providers: `https://artificialanalysis.ai/models/claude-4-5-sonnet/providers`
  - used values (Anthropic provider): `ttft=1.13s`, `output_speed=71.6 t/s`, `blended_price=$6.00 / 1M`

- `google_vertex_openai` + `gemini-2-5-pro`
  - model: `https://artificialanalysis.ai/models/gemini-2-5-pro`
  - providers: `https://artificialanalysis.ai/models/gemini-2-5-pro/providers`
  - used values (Vertex provider): `ttft=37.88s`, `output_speed=135.4 t/s`, `blended_price=$3.44 / 1M`

- `groq_openai` + `llama-3-3-instruct-70b`
  - model: `https://artificialanalysis.ai/models/llama-3-3-instruct-70b`
  - providers: `https://artificialanalysis.ai/models/llama-3-3-instruct-70b/providers`
  - used values (Groq provider): `ttft=0.25s`, `output_speed=335.5 t/s`, `blended_price=$0.64 / 1M` (model-page median baseline)

- `mistral_api` + `mistral-large-3`
  - model: `https://artificialanalysis.ai/models/mistral-large-3`
  - providers: `https://artificialanalysis.ai/models/mistral-large-3/providers`
  - used values (Mistral provider): `ttft=0.59s`, `output_speed=51.3 t/s`, `blended_price=$0.75 / 1M`

## Notes

- `cost_per_1k_tokens_usd` in CSVs is derived by dividing blended `$/1M` by 1000.
- Provider-side benchmark values are medians over rolling windows and may drift.
- These values cover performance/cost only. Task-class quality fields still need direct task benchmark runs to populate `accuracy`, `pass_at_k`, `schema_adherence`, and `factuality_score`.
- Model-level intelligence/performance snapshots are captured in `THIRD_PARTY_MODEL_BENCHMARKS.csv` for broader prefill coverage beyond cloud provider rows.
