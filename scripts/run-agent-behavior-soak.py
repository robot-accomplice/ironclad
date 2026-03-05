#!/usr/bin/env python3
import json
import os
import sys
import time
import urllib.error
import urllib.request


BASE_URL = os.environ.get("BASE_URL", "http://127.0.0.1:18789").rstrip("/")
TIMEOUT = int(os.environ.get("SOAK_TIMEOUT_SECONDS", "240"))


def send_message(prompt, session_id=None):
    payload = {"content": prompt}
    if session_id:
        payload["session_id"] = session_id
    req = urllib.request.Request(
        BASE_URL + "/api/agent/message",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    started = time.time()
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        body = json.loads(resp.read().decode("utf-8", "replace"))
    body["_latency_s"] = round(time.time() - started, 2)
    return body


def assert_true(condition, message):
    if not condition:
        raise AssertionError(message)


def main():
    print(f"[behavior-soak] base_url={BASE_URL}")
    session_id = None

    # 1) random tool use should include verified tool execution output.
    r1 = send_message(
        "tell me about the tools you can use, pick one at random, and use it", session_id
    )
    session_id = r1.get("session_id")
    c1 = str(r1.get("content") or "")
    assert_true("did not execute a tool" not in c1.lower(), "tool-use prompt was not executed")
    assert_true("output" in c1.lower(), "tool-use prompt did not return tool output evidence")
    print(f"[behavior-soak] tools_random_use ok latency={r1['_latency_s']}s")

    # 2) model identity must exactly reflect API execution model.
    r2 = send_message("can you confirm for me that you are still using moonshot?", session_id)
    c2 = str(r2.get("content") or "")
    model2 = str(r2.get("model") or "")
    assert_true(model2, "response missing model field")
    assert_true(model2 in c2, "model identity reply does not match executed model field")
    print(f"[behavior-soak] model_identity ok model={model2} latency={r2['_latency_s']}s")

    # 3) delegation must execute real delegation path (or explicitly fail with delegated tool attempt).
    r3 = send_message(
        "order a subagent to list the markdown files in ~/code/ironclad and return the count only",
        session_id,
    )
    c3 = str(r3.get("content") or "")
    assert_true(
        "did not execute a delegated subagent task" not in c3.lower(),
        "delegation prompt blocked without actual delegation attempt",
    )
    assert_true(
        ("delegated_subagent=" in c3) or ("attempted delegation" in c3.lower()) or ("orchestrate-subagents" in c3.lower()),
        "delegation response lacks evidence of real delegation path",
    )
    print(f"[behavior-soak] delegation ok latency={r3['_latency_s']}s")

    # 4) cron must create a schedule and report it.
    r4 = send_message(
        "schedule a cron job that runs every 5 minutes and tell me exactly what was scheduled",
        session_id,
    )
    c4 = str(r4.get("content") or "")
    assert_true("scheduled cron job" in c4.lower(), "cron prompt did not schedule successfully")
    assert_true("*/5 * * * *" in c4, "cron response missing expected expression")
    print(f"[behavior-soak] cron ok latency={r4['_latency_s']}s")

    # 5) tilde handling should work.
    r5 = send_message("give me the file distribution in the folder ~", session_id)
    c5 = str(r5.get("content") or "")
    assert_true("file distribution for" in c5.lower(), "tilde path distribution did not run")
    assert_true("did not execute a tool" not in c5.lower(), "tilde path request was not executed")
    print(f"[behavior-soak] tilde_distribution ok latency={r5['_latency_s']}s")

    # 6) absolute path handling should also work.
    r6 = send_message("give me the file distribution in the folder /Users/jmachen", session_id)
    c6 = str(r6.get("content") or "")
    assert_true("file distribution for" in c6.lower(), "absolute path distribution did not run")
    assert_true("did not execute a tool" not in c6.lower(), "absolute path request was not executed")
    print(f"[behavior-soak] abs_distribution ok latency={r6['_latency_s']}s")

    print("[behavior-soak] PASS")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except urllib.error.HTTPError as e:
        print(f"[behavior-soak] HTTP error: {e.code} {e.reason}", file=sys.stderr)
        raise
    except Exception as e:
        print(f"[behavior-soak] FAIL: {e}", file=sys.stderr)
        raise
