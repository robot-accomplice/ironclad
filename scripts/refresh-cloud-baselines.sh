#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/refresh-cloud-baselines.sh [--write]

Default mode is dry-run (prints proposed updates).
Use --write to apply updates to:
  - docs/evals/runner_properties.baseline.csv
  - docs/evals/task_efficacy.baseline.csv
  - docs/evals/CLOUD_BENCHMARK_SOURCES.md
EOF
}

WRITE=0
if [[ "${1:-}" == "--write" ]]; then
  WRITE=1
elif [[ $# -gt 0 ]]; then
  usage
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

fetch_url() {
  local url="$1"
  local out="$2"
  if command -v ghola >/dev/null 2>&1; then
    ghola -o "$out" "$url" >/dev/null
  elif command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
  else
    echo "error: neither ghola nor curl found" >&2
    exit 1
  fi
}

# Provider pages are used for provider-specific latency/speed/price.
fetch_url "https://artificialanalysis.ai/models/gpt-5-mini-minimal/providers" "$TMP_DIR/gpt5_providers.txt"
fetch_url "https://artificialanalysis.ai/models/claude-4-5-sonnet/providers" "$TMP_DIR/claude45_providers.txt"
fetch_url "https://artificialanalysis.ai/models/gemini-2-5-pro/providers" "$TMP_DIR/gemini25_providers.txt"
fetch_url "https://artificialanalysis.ai/models/llama-3-3-instruct-70b/providers" "$TMP_DIR/llama33_providers.txt"
fetch_url "https://artificialanalysis.ai/models/mistral-large-3/providers" "$TMP_DIR/mistrall3_providers.txt"

python3 - "$ROOT_DIR" "$TMP_DIR" "$WRITE" <<'PY'
import csv
import datetime as dt
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
tmp = pathlib.Path(sys.argv[2])
write = sys.argv[3] == "1"
today = dt.date.today().isoformat()

runner_csv = root / "docs/evals/runner_properties.baseline.csv"
efficacy_csv = root / "docs/evals/task_efficacy.baseline.csv"
sources_md = root / "docs/evals/CLOUD_BENCHMARK_SOURCES.md"

def read_text(name: str) -> str:
    return (tmp / name).read_text(encoding="utf-8", errors="ignore")

def extract(text: str, pattern: str, label: str) -> float:
    m = re.search(pattern, text, flags=re.IGNORECASE | re.MULTILINE)
    if not m:
        raise RuntimeError(f"unable to parse {label} with pattern: {pattern}")
    return float(m.group(1))

def p50_ms(tps: float) -> float:
    return round(1000.0 / tps, 2)

gpt = read_text("gpt5_providers.txt")
claude = read_text("claude45_providers.txt")
gemini = read_text("gemini25_providers.txt")
llama = read_text("llama33_providers.txt")
mistral = read_text("mistrall3_providers.txt")

# Parse provider-specific values from FAQ/list sections.
metrics = {
    "openai_responses": {
        "ttft_ms": int(round(extract(gpt, r"OpenAI\)\(([0-9.]+)s\)", "gpt ttft") * 1000)),
        "tokens_per_sec": extract(gpt, r"OpenAI\)\(([0-9.]+)\s*t/s\)", "gpt speed"),
        "blended_per_1m": extract(gpt, r"OpenAI\)\(\$([0-9.]+)\s*per 1M tokens\)", "gpt blended"),
    },
    "anthropic_messages": {
        "ttft_ms": int(round(extract(claude, r"Anthropic\)\(([0-9.]+)s\)", "claude ttft") * 1000)),
        "tokens_per_sec": extract(claude, r"Anthropic\)\(([0-9.]+)\s*t/s\)", "claude speed"),
        "blended_per_1m": extract(claude, r"Anthropic\)\(\$([0-9.]+)\s*per 1M tokens\)", "claude blended"),
    },
    "google_vertex_openai": {
        "ttft_ms": int(round(extract(gemini, r"Google Vertex\)\(([0-9.]+)s\)", "gemini vertex ttft") * 1000)),
        "tokens_per_sec": extract(gemini, r"Google Vertex\)\(([0-9.]+)\s*t/s\)", "gemini vertex speed"),
        "blended_per_1m": extract(gemini, r"Google Vertex\)\(\$([0-9.]+)\s*per 1M tokens\)", "gemini vertex blended"),
    },
    "groq_openai": {
        "ttft_ms": int(round(extract(llama, r"Groq\)\(([0-9.]+)s\)", "llama groq ttft") * 1000)),
        "tokens_per_sec": extract(llama, r"Groq\)\(([0-9.]+)\s*t/s\)", "llama groq speed"),
        # Groq price is not consistently in the top-price FAQ bullets; keep existing baseline.
        "blended_per_1m": None,
    },
    "mistral_api": {
        "ttft_ms": int(round(extract(mistral, r"Mistral\)\(([0-9.]+)s\)", "mistral ttft") * 1000)),
        "tokens_per_sec": extract(mistral, r"Mistral\)\(([0-9.]+)\s*t/s\)", "mistral speed"),
        "blended_per_1m": extract(mistral, r"Mistral\)\(\$([0-9.]+)\s*per 1M tokens\)", "mistral blended"),
    },
}

for runner, m in metrics.items():
    m["p50_token_ms"] = p50_ms(m["tokens_per_sec"])
    if m["blended_per_1m"] is not None:
        m["cost_per_1k_tokens_usd"] = round(m["blended_per_1m"] / 1000.0, 8)

print("Parsed cloud metrics:")
for runner, m in metrics.items():
    print(f"- {runner}: ttft_ms={m['ttft_ms']} tps={m['tokens_per_sec']} p50_token_ms={m['p50_token_ms']} blended_per_1m={m['blended_per_1m']}")

def update_runner_csv() -> None:
    with runner_csv.open(newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
        fieldnames = rows[0].keys() if rows else []

    for row in rows:
        rid = row["runner_id"]
        if rid not in metrics:
            continue
        m = metrics[rid]
        row["runner_version"] = f"aa_snapshot_{today}"
        row["ttft_ms"] = str(m["ttft_ms"])
        row["tokens_per_sec"] = str(m["tokens_per_sec"])
        row["p50_token_ms"] = f"{m['p50_token_ms']:.2f}"
        if m["blended_per_1m"] is not None:
            row["cost_per_1k_tokens_usd"] = f"{m['cost_per_1k_tokens_usd']:.8f}".rstrip("0").rstrip(".")

    if write:
        with runner_csv.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=fieldnames)
            w.writeheader()
            w.writerows(rows)

def update_efficacy_csv() -> None:
    with efficacy_csv.open(newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
        fieldnames = rows[0].keys() if rows else []

    for row in rows:
        rid = row["runner_id"]
        if rid not in metrics:
            continue
        m = metrics[rid]
        row["eval_run_id"] = f"AA-{today}"
        row["ttft_ms"] = str(m["ttft_ms"])
        row["tokens_per_sec"] = str(m["tokens_per_sec"])

    if write:
        with efficacy_csv.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=fieldnames)
            w.writeheader()
            w.writerows(rows)

def update_sources_md() -> None:
    text = sources_md.read_text(encoding="utf-8")
    text = re.sub(
        r"Snapshot date:\s*`[0-9]{4}-[0-9]{2}-[0-9]{2}`",
        f"Snapshot date: `{today}`",
        text,
        count=1,
    )
    if write:
        sources_md.write_text(text, encoding="utf-8")

update_runner_csv()
update_efficacy_csv()
update_sources_md()

if write:
    print("Applied updates to cloud baseline CSVs and source snapshot date.")
else:
    print("Dry-run only. Re-run with --write to apply changes.")
PY
