#!/usr/bin/env python3
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Callable, Dict, List, Tuple

IRONCLAD_BASE_URL = os.environ.get("IRONCLAD_BASE_URL", "http://127.0.0.1:18789").rstrip("/")
OPENCLAW_BASE_URL = os.environ.get("OPENCLAW_BASE_URL", "http://127.0.0.1:8787").rstrip("/")
OPENCLAW_MODE = os.environ.get("OPENCLAW_MODE", "auto").strip().lower()
OPENCLAW_AGENT = os.environ.get("OPENCLAW_AGENT", "main").strip()
TIMEOUT = int(os.environ.get("SOAK_TIMEOUT_SECONDS", "240"))
MAX_LATENCY = float(os.environ.get("SOAK_MAX_LATENCY_SECONDS", "70"))
REPORT_PATH = os.environ.get(
    "PARITY_REPORT_PATH", "/tmp/ironclad-openclaw-parity-report.json"
)

STALE_MARKERS = [
    "as of my last update",
    "as of my last training",
    "cannot provide real-time updates",
    "can't provide real-time updates",
    "as of early 2023",
    "as of 2023",
    "2021-09-30",
]

FOREIGN_PERSONA_MARKERS = [
    "as an ai developed by microsoft",
    "as an ai language model",
    "i am claude",
    "i am chatgpt",
    "microsoft ai",
    "google/palm 2",
    "anthropic claude-2",
]

VAGUE_FILLER_MARKERS = [
    "how may i assist you further",
    "how can i assist you further",
    "ready when you are",
    "i don't have access to your files",
]


def send_message(base_url: str, prompt: str, session_id: str = "") -> Dict[str, object]:
    payload: Dict[str, object] = {"content": prompt}
    if session_id:
        payload["session_id"] = session_id
    req = urllib.request.Request(
        base_url + "/api/agent/message",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    started = time.time()
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        body = json.loads(resp.read().decode("utf-8", "replace"))
    body["_latency_s"] = round(time.time() - started, 2)
    return body


def parse_first_json_object(raw: str) -> Dict[str, object]:
    raw = raw.strip()
    if not raw:
        return {}
    try:
        parsed = json.loads(raw)
        return parsed if isinstance(parsed, dict) else {}
    except Exception:
        pass
    for line in reversed(raw.splitlines()):
        line = line.strip()
        if not line:
            continue
        if line.startswith("{") and line.endswith("}"):
            try:
                parsed = json.loads(line)
                return parsed if isinstance(parsed, dict) else {}
            except Exception:
                continue
    return {}


def extract_openclaw_content(payload: Dict[str, object]) -> str:
    candidates = [
        payload.get("content"),
        payload.get("response"),
        payload.get("message"),
        payload.get("text"),
        payload.get("output"),
    ]
    for v in candidates:
        if isinstance(v, str) and v.strip():
            return v
    data = payload.get("data")
    if isinstance(data, dict):
        for key in ("content", "response", "message", "text"):
            val = data.get(key)
            if isinstance(val, str) and val.strip():
                return val
    packets = payload.get("payloads")
    if isinstance(packets, list):
        lines: List[str] = []
        for item in packets:
            if not isinstance(item, dict):
                continue
            txt = item.get("text")
            if isinstance(txt, str) and txt.strip():
                lines.append(txt.strip())
        if lines:
            return "\n".join(lines)
    return ""


def send_openclaw_cli(prompt: str, session_id: str = "") -> Dict[str, object]:
    cmd = [
        "openclaw",
        "--no-color",
        "agent",
        "--local",
        "--json",
        "--agent",
        OPENCLAW_AGENT,
        "--message",
        prompt,
        "--timeout",
        str(TIMEOUT),
    ]
    if session_id:
        cmd.extend(["--session-id", session_id])
    started = time.time()
    proc = subprocess.run(
        cmd, capture_output=True, text=True, timeout=max(TIMEOUT + 15, 30)
    )
    latency = round(time.time() - started, 2)
    if proc.returncode != 0:
        raise RuntimeError(
            f"openclaw cli failed rc={proc.returncode}: "
            f"{(proc.stderr or proc.stdout).strip()}"
        )
    parsed = parse_first_json_object(proc.stdout)
    content = extract_openclaw_content(parsed)
    model = (
        parsed.get("model")
        or parsed.get("resolvedModel")
        or parsed.get("selectedModel")
        or (
            parsed.get("meta", {}).get("agentMeta", {}).get("model")
            if isinstance(parsed.get("meta"), dict)
            else ""
        )
        or ""
    )
    sid = (
        parsed.get("session_id")
        or parsed.get("sessionId")
        or parsed.get("id")
        or session_id
        or ""
    )
    return {
        "content": content,
        "model": model,
        "session_id": sid,
        "_latency_s": latency,
        "_raw": parsed,
    }


def choose_openclaw_mode() -> str:
    if OPENCLAW_MODE in ("api", "cli"):
        return OPENCLAW_MODE
    try:
        send_message(OPENCLAW_BASE_URL, "ping")
        return "api"
    except Exception:
        return "cli"


def contains_any(text: str, markers: List[str]) -> bool:
    lower = text.lower()
    return any(m in lower for m in markers)


def one_sentence_ack(text: str) -> bool:
    stripped = text.strip()
    if not stripped:
        return False
    sentence_count = len(re.findall(r"[.!?](?:\s|$)", stripped))
    if sentence_count == 0:
        sentence_count = 1
    return sentence_count == 1 and len(stripped.splitlines()) == 1


def has_execution_block(text: str) -> bool:
    lower = text.lower()
    return (
        "i did not execute a tool" in lower
        or "i did not execute a delegated subagent task" in lower
        or "i did not execute a cron scheduling tool" in lower
    )


Check = Callable[[Dict[str, object], str], Tuple[bool, str]]


def check_latency(resp: Dict[str, object], _content: str) -> Tuple[bool, str]:
    latency = float(resp.get("_latency_s", 0.0))
    ok = latency <= MAX_LATENCY
    return ok, f"latency={latency}s max={MAX_LATENCY}s"


def check_no_stale(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    ok = not contains_any(content, STALE_MARKERS)
    return ok, "no stale-knowledge markers"


def check_no_foreign_persona(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    ok = not contains_any(content, FOREIGN_PERSONA_MARKERS)
    return ok, "no foreign persona/vendor boilerplate"


def check_not_vague_filler(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    ok = not contains_any(content, VAGUE_FILLER_MARKERS)
    return ok, "not a generic filler response"


def check_no_exec_block(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    ok = not has_execution_block(content)
    return ok, "no false execution/delegation block message"


def check_ack(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    ok = one_sentence_ack(content) and (
        "acknowledged" in content.lower() or "await" in content.lower()
    )
    return ok, "single-sentence acknowledgement"


def check_model_identity(resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    model = str(resp.get("model") or "")
    lower = content.lower()
    if not content.strip():
        return False, "empty identity response"
    if model and model != "auto":
        ok = model in content
        return ok, f"model field reflected in content ({model})"
    ok = "moonshot" in lower or "model" in lower or "provider" in lower
    return ok, "identity reply references model/provider state"


def check_introspection_summary(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    lower = content.lower()
    signals = ["subagent", "session", "tool", "memory", "channel", "runtime"]
    matched = [s for s in signals if s in lower]
    ok = len(matched) >= 2 and len(content.strip()) >= 40
    return ok, f"introspection includes runtime/capability summary ({matched})"


def check_tool_use(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    lower = content.lower()
    ok = (
        "output" in lower
        or "available tools" in lower
        or "tool results" in lower
        or "executed" in lower
    )
    return ok, "returns concrete tool-use evidence"


def check_delegation_evidence(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    lower = content.lower()
    ok = (
        "orchestrate-subagents" in lower
        or "delegated_subagent=" in lower
        or "attempted delegation" in lower
        or "subtask" in lower
    )
    return ok, "delegation path evidence present"


def check_count_only_output(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    stripped = content.strip()
    ok = bool(re.fullmatch(r"\d+", stripped))
    return ok, "returns count-only numeric output"


def check_geopolitical_quality(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    lower = content.lower().strip()
    if not lower:
        return False, "empty geopolitical response"
    is_clarify = lower.endswith("?") and (
        "source" in lower or "region" in lower or "focus" in lower
    )
    has_sitrep_shape = (
        len(lower) >= 120
        and ("geopolitical" in lower or "situation" in lower or "conflict" in lower)
    )
    ok = is_clarify or has_sitrep_shape
    return ok, "geopolitical response is clarifying or substantive"


def check_current_date_grounding(_resp: Dict[str, object], content: str) -> Tuple[bool, str]:
    lower = content.lower()
    stale_date = bool(re.search(r"\b(2021|2022|2023)\b", lower)) and (
        "as of" in lower or "last update" in lower
    )
    return (not stale_date), "no stale date grounding disclaimers"


@dataclass
class Scenario:
    name: str
    prompt: str
    checks: List[Check]
    weight: float = 1.0


SCENARIOS: List[Scenario] = [
    Scenario(
        "acknowledgement_sla",
        "Good evening Duncan. Acknowledge this request in one sentence, then wait.",
        [check_latency, check_ack, check_no_stale, check_no_foreign_persona],
        1.0,
    ),
    Scenario(
        "tool_random_use",
        "tell me about the tools you can use, pick one at random, and use it",
        [
            check_latency,
            check_no_exec_block,
            check_tool_use,
            check_no_stale,
            check_no_foreign_persona,
        ],
        1.5,
    ),
    Scenario(
        "introspection_discovery",
        "use your introspection tool to discover your current subagent functionality and summarize it for me",
        [
            check_latency,
            check_no_exec_block,
            check_introspection_summary,
            check_no_stale,
            check_no_foreign_persona,
        ],
        1.2,
    ),
    Scenario(
        "delegation",
        "order a subagent to list the markdown files in ~/code/ironclad and return the count only",
        [
            check_latency,
            check_no_exec_block,
            check_count_only_output,
            check_no_stale,
        ],
        1.5,
    ),
    Scenario(
        "geopolitical_sitrep",
        "What does the geopolitical monitor have to say about today's news?",
        [
            check_latency,
            check_no_exec_block,
            check_geopolitical_quality,
            check_no_stale,
            check_current_date_grounding,
            check_no_foreign_persona,
            check_not_vague_filler,
        ],
        2.0,
    ),
    Scenario(
        "model_identity",
        "can you confirm for me that you are still using moonshot?",
        [check_latency, check_model_identity, check_no_stale, check_no_foreign_persona],
        1.0,
    ),
]


def run_target(name: str, base_url: str, mode: str = "api") -> Dict[str, object]:
    print(f"[parity] target={name} base_url={base_url}")
    session_id = ""
    results: List[Dict[str, object]] = []
    weighted_total = 0.0
    weighted_pass = 0.0

    for scenario in SCENARIOS:
        if mode == "cli":
            resp = send_openclaw_cli(scenario.prompt, session_id)
        else:
            resp = send_message(base_url, scenario.prompt, session_id)
        session_id = str(resp.get("session_id") or session_id or "")
        content = str(resp.get("content") or "")

        checks = []
        passed = True
        for check in scenario.checks:
            ok, detail = check(resp, content)
            checks.append({"ok": ok, "detail": detail, "check": check.__name__})
            if not ok:
                passed = False

        weighted_total += scenario.weight
        if passed:
            weighted_pass += scenario.weight

        row = {
            "name": scenario.name,
            "prompt": scenario.prompt,
            "latency_s": resp.get("_latency_s"),
            "model": resp.get("model"),
            "session_id": resp.get("session_id"),
            "passed": passed,
            "weight": scenario.weight,
            "checks": checks,
            "content": content,
        }
        results.append(row)

        status = "PASS" if passed else "FAIL"
        print(
            f"[parity] {name} {status} {scenario.name} "
            f"latency={resp.get('_latency_s')}s model={resp.get('model')}"
        )
        if not passed:
            for c in checks:
                if not c["ok"]:
                    print(f"  - {c['check']}: {c['detail']}")

    avg_latency = round(
        sum(float(r.get("latency_s") or 0.0) for r in results) / max(len(results), 1), 2
    )
    failures = [r for r in results if not r["passed"]]
    score = round((weighted_pass / max(weighted_total, 0.0001)) * 100.0, 2)

    return {
        "target": name,
        "base_url": base_url,
        "total": len(results),
        "failed": len(failures),
        "pass_rate": round((len(results) - len(failures)) / max(len(results), 1) * 100.0, 2),
        "weighted_score": score,
        "avg_latency_s": avg_latency,
        "results": results,
    }


def print_summary(ironclad: Dict[str, object], openclaw: Dict[str, object]) -> None:
    print("\n[parity] summary")
    print("target      pass_rate   weighted_score   avg_latency_s   failed")
    for t in (ironclad, openclaw):
        print(
            f"{t['target']:<11} {str(t['pass_rate'])+'%':<11} "
            f"{str(t['weighted_score'])+'%':<16} {t['avg_latency_s']:<14} {t['failed']}"
        )

    delta_score = round(float(ironclad["weighted_score"]) - float(openclaw["weighted_score"]), 2)
    delta_latency = round(float(ironclad["avg_latency_s"]) - float(openclaw["avg_latency_s"]), 2)
    print(
        f"[parity] delta ironclad-openclaw: score={delta_score}% latency={delta_latency}s"
    )


def run() -> int:
    print(
        f"[parity] timeout={TIMEOUT}s max_latency={MAX_LATENCY}s "
        f"report={REPORT_PATH}"
    )
    ironclad = run_target("ironclad", IRONCLAD_BASE_URL, "api")
    openclaw_mode = choose_openclaw_mode()
    openclaw = run_target("openclaw", OPENCLAW_BASE_URL, openclaw_mode)
    openclaw["mode"] = openclaw_mode
    print_summary(ironclad, openclaw)

    report = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "timeout_s": TIMEOUT,
        "max_latency_s": MAX_LATENCY,
        "ironclad": ironclad,
        "openclaw": openclaw,
        "comparison": {
            "score_delta": round(
                float(ironclad["weighted_score"]) - float(openclaw["weighted_score"]), 2
            ),
            "latency_delta_s": round(
                float(ironclad["avg_latency_s"]) - float(openclaw["avg_latency_s"]), 2
            ),
        },
    }
    with open(REPORT_PATH, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)

    print(f"[parity] wrote report: {REPORT_PATH}")

    # Gate policy:
    # 1) ironclad must score >= openclaw
    # 2) ironclad must have zero stale-marker failures
    score_ok = float(ironclad["weighted_score"]) >= float(openclaw["weighted_score"])
    stale_failures = 0
    for row in ironclad["results"]:
        for c in row["checks"]:
            if c["check"] == "check_no_stale" and not c["ok"]:
                stale_failures += 1

    if not score_ok or stale_failures > 0:
        print(
            f"[parity] FAIL score_ok={score_ok} stale_failures={stale_failures}",
            file=sys.stderr,
        )
        return 1

    print("[parity] PASS ironclad meets parity gate")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(run())
    except urllib.error.HTTPError as e:
        print(f"[parity] HTTP error: {e.code} {e.reason}", file=sys.stderr)
        raise
    except urllib.error.URLError as e:
        print(f"[parity] URL error: {e.reason}", file=sys.stderr)
        raise
    except Exception as e:
        print(f"[parity] FAIL: {e}", file=sys.stderr)
        raise
