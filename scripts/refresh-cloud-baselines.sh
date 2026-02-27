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

def section_between(text: str, start: str, end: str, label: str) -> str:
    pattern = re.compile(start + r"(.*?)" + end, flags=re.IGNORECASE | re.DOTALL)
    m = pattern.search(text)
    if not m:
        raise RuntimeError(f"unable to locate section {label}")
    return m.group(1)

def extract_provider_value(section: str, provider_label: str, unit_label: str, label: str) -> float:
    # Extract value from ranking cards where provider and value are in the same row.
    # unit_label should be one of: "t/s", "s", "$".
    if unit_label == "$":
        pattern = rf">{re.escape(provider_label)}</span></div><span[^>]*>\s*<span class=\"text-xs text-gray-400\">\$</span><span[^>]*>([0-9.]+)</span>"
    elif unit_label == "t/s":
        pattern = rf">{re.escape(provider_label)}</span></div><span[^>]*>\s*<span[^>]*>([0-9.]+)</span><span class=\"text-xs text-gray-400\">\s*<!-- -->t/s</span>"
    elif unit_label == "s":
        pattern = rf">{re.escape(provider_label)}</span></div><span[^>]*>\s*<span[^>]*>([0-9.]+)</span><span class=\"text-xs text-gray-400\">\s*<!-- -->s</span>"
    else:
        raise RuntimeError(f"unsupported unit label: {unit_label}")
    m = re.search(pattern, section, flags=re.IGNORECASE | re.DOTALL)
    if not m:
        raise RuntimeError(f"unable to parse {label} for provider {provider_label}")
    return float(m.group(1))

# Parse provider-specific values from top summary cards.
gpt_speed_section = section_between(gpt, r"Fastest</h3>", r"<p>Output speed</p>", "gpt speed")
gpt_latency_section = section_between(gpt, r"Lowest Latency</h3>", r"<p>Time to first token</p>", "gpt latency")
gpt_price_section = section_between(gpt, r"Lowest Price</h3>", r"<p>Blended price \(per 1M tokens\)</p>", "gpt price")

claude_speed_section = section_between(claude, r"Fastest</h3>", r"<p>Output speed</p>", "claude speed")
claude_latency_section = section_between(claude, r"Lowest Latency</h3>", r"<p>Time to first token</p>", "claude latency")
claude_price_section = section_between(claude, r"Lowest Price</h3>", r"<p>Blended price \(per 1M tokens\)</p>", "claude price")

gemini_speed_section = section_between(gemini, r"Fastest</h3>", r"<p>Output speed</p>", "gemini speed")
gemini_latency_section = section_between(gemini, r"Lowest Latency</h3>", r"<p>Time to first token</p>", "gemini latency")
gemini_price_section = section_between(gemini, r"Lowest Price</h3>", r"<p>Blended price \(per 1M tokens\)</p>", "gemini price")

llama_speed_section = section_between(llama, r"Fastest</h3>", r"<p>Output speed</p>", "llama speed")
llama_latency_section = section_between(llama, r"Lowest Latency</h3>", r"<p>Time to first token</p>", "llama latency")
llama_price_section = section_between(llama, r"Lowest Price</h3>", r"<p>Blended price \(per 1M tokens\)</p>", "llama price")

mistral_speed_section = section_between(mistral, r"Fastest</h3>", r"<p>Output speed</p>", "mistral speed")
mistral_latency_section = section_between(mistral, r"Lowest Latency</h3>", r"<p>Time to first token</p>", "mistral latency")
mistral_price_section = section_between(mistral, r"Lowest Price</h3>", r"<p>Blended price \(per 1M tokens\)</p>", "mistral price")

metrics = {
    "openai_responses": {
        "ttft_ms": int(round(extract_provider_value(gpt_latency_section, "OpenAI", "s", "gpt ttft") * 1000)),
        "tokens_per_sec": extract_provider_value(gpt_speed_section, "OpenAI", "t/s", "gpt speed"),
        "blended_per_1m": extract_provider_value(gpt_price_section, "OpenAI", "$", "gpt blended"),
    },
    "anthropic_messages": {
        "ttft_ms": int(round(extract_provider_value(claude_latency_section, "Anthropic", "s", "claude ttft") * 1000)),
        "tokens_per_sec": extract_provider_value(claude_speed_section, "Anthropic", "t/s", "claude speed"),
        "blended_per_1m": extract_provider_value(claude_price_section, "Anthropic", "$", "claude blended"),
    },
    "google_vertex_openai": {
        "ttft_ms": int(round(extract_provider_value(gemini_latency_section, "Google Vertex", "s", "gemini vertex ttft") * 1000)),
        "tokens_per_sec": extract_provider_value(gemini_speed_section, "Google Vertex", "t/s", "gemini vertex speed"),
        "blended_per_1m": extract_provider_value(gemini_price_section, "Google Vertex", "$", "gemini vertex blended"),
    },
    "groq_openai": {
        "ttft_ms": int(round(extract_provider_value(llama_latency_section, "Groq", "s", "llama groq ttft") * 1000)),
        "tokens_per_sec": extract_provider_value(llama_speed_section, "Groq", "t/s", "llama groq speed"),
        # Groq price is not always in the "Lowest Price" ranking card; keep existing baseline if absent.
        "blended_per_1m": (
            extract_provider_value(llama_price_section, "Groq", "$", "llama groq blended")
            if "Groq" in llama_price_section
            else None
        ),
    },
    "mistral_api": {
        "ttft_ms": int(round(extract_provider_value(mistral_latency_section, "Mistral", "s", "mistral ttft") * 1000)),
        "tokens_per_sec": extract_provider_value(mistral_speed_section, "Mistral", "t/s", "mistral speed"),
        "blended_per_1m": extract_provider_value(mistral_price_section, "Mistral", "$", "mistral blended"),
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
