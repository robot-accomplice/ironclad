#!/bin/sh
# claude-code plugin script
# Invoked by Ironclad's ScriptPlugin with IRONCLAD_INPUT as JSON.
#
# Expected input fields:
#   prompt          (required) — the task description for Claude Code
#   working_dir     (optional) — directory to run in (defaults to $HOME)
#   max_turns       (optional) — max agentic turns (default: 10)
#   max_budget_usd  (optional) — cost cap per invocation (default: 0.50)
#   session_id      (optional) — resume a previous session
#   continue_last   (optional) — continue the most recent session (boolean)
#   allowed_tools   (optional) — override default tool allowlist
#
# Output: JSON with result, cost, session_id, and status.

set -eu

# ── Structural recursion guard ────────────────────────────────────
# Prevent recursive delegation: if we detect that we're already inside
# a delegated Claude Code session, refuse to run. This is a structural
# enforcement per skills-catalog-contract.md, not prompt-based.
DEPTH="${IRONCLAD_DELEGATION_DEPTH:-0}"
MAX_DEPTH=1
if [ "$DEPTH" -ge "$MAX_DEPTH" ]; then
    printf '{"success":false,"error":"delegation depth %d >= max %d — recursive AI-CLI delegation blocked"}\n' \
        "$DEPTH" "$MAX_DEPTH"
    exit 0
fi

# ── Dependency check ──────────────────────────────────────────────
if ! command -v claude >/dev/null 2>&1; then
    printf '{"success":false,"error":"claude CLI not found. Install Claude Code: https://docs.anthropic.com/en/docs/claude-code"}\n'
    exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
    printf '{"success":false,"error":"jq not found. Install jq for JSON parsing."}\n'
    exit 1
fi

# ── Parse input ───────────────────────────────────────────────────
INPUT="${IRONCLAD_INPUT:-{}}"

PROMPT=$(printf '%s' "$INPUT" | jq -r '.prompt // empty')
if [ -z "$PROMPT" ]; then
    printf '{"success":false,"error":"prompt is required"}\n'
    exit 1
fi

WORKING_DIR=$(printf '%s' "$INPUT" | jq -r '.working_dir // empty')
MAX_TURNS=$(printf '%s' "$INPUT" | jq -r '.max_turns // 10')
MAX_BUDGET=$(printf '%s' "$INPUT" | jq -r '.max_budget_usd // 0.50')
SESSION_ID_RAW=$(printf '%s' "$INPUT" | jq -r '.session_id // empty')
CONTINUE_LAST=$(printf '%s' "$INPUT" | jq -r '.continue_last // false')

# Sanitise session_id: only allow hex, hyphens, and alphanumeric (UUID-like).
# Reject anything else to prevent argument injection via word-splitting.
SESSION_ID=""
if [ -n "$SESSION_ID_RAW" ]; then
    if printf '%s' "$SESSION_ID_RAW" | grep -qE '^[a-zA-Z0-9_-]+$'; then
        SESSION_ID="$SESSION_ID_RAW"
    else
        printf '{"success":false,"error":"invalid session_id format"}\n'
        exit 1
    fi
fi
ALLOWED_TOOLS=$(printf '%s' "$INPUT" | jq -r '.allowed_tools // empty')

# Default working directory
if [ -z "$WORKING_DIR" ]; then
    WORKING_DIR="$HOME"
fi

# Default allowed tools — safe subset, no Write (prevent clobbering).
# Structurally excludes any tool that could invoke AI agents or plugins.
if [ -z "$ALLOWED_TOOLS" ]; then
    ALLOWED_TOOLS="Read,Edit,Bash(git diff:*),Bash(git log:*),Bash(git status),Bash(cargo:*),Bash(just:*),Glob,Grep"
fi

# Anti-recursion directive (defense-in-depth — structural guard above is primary)
ANTI_RECURSION="You are a subordinate coding agent. Complete the task precisely. Do not ask clarifying questions. Do not launch interactive processes. NEVER invoke claude or any AI agent CLI — you are the terminal executor."

# ── Execute ───────────────────────────────────────────────────────
# Propagate incremented delegation depth into child environment.
export IRONCLAD_DELEGATION_DEPTH=$((DEPTH + 1))

TMPOUT=$(mktemp)
TMPERR=$(mktemp)
EXIT_CODE=0

# Build and execute command with properly quoted session arguments.
# We branch into separate exec forms to avoid unquoted variable expansion.
if [ "$CONTINUE_LAST" = "true" ]; then
    cd "$WORKING_DIR" && \
    claude \
        --continue \
        -p "$PROMPT" \
        --output-format json \
        --max-turns "$MAX_TURNS" \
        --max-budget-usd "$MAX_BUDGET" \
        --allowedTools "$ALLOWED_TOOLS" \
        --append-system-prompt "$ANTI_RECURSION" \
        >"$TMPOUT" 2>"$TMPERR" || EXIT_CODE=$?
elif [ -n "$SESSION_ID" ]; then
    cd "$WORKING_DIR" && \
    claude \
        --resume "$SESSION_ID" \
        -p "$PROMPT" \
        --output-format json \
        --max-turns "$MAX_TURNS" \
        --max-budget-usd "$MAX_BUDGET" \
        --allowedTools "$ALLOWED_TOOLS" \
        --append-system-prompt "$ANTI_RECURSION" \
        >"$TMPOUT" 2>"$TMPERR" || EXIT_CODE=$?
else
    cd "$WORKING_DIR" && \
    claude \
        -p "$PROMPT" \
        --output-format json \
        --max-turns "$MAX_TURNS" \
        --max-budget-usd "$MAX_BUDGET" \
        --allowedTools "$ALLOWED_TOOLS" \
        --append-system-prompt "$ANTI_RECURSION" \
        >"$TMPOUT" 2>"$TMPERR" || EXIT_CODE=$?
fi

STDOUT=$(cat "$TMPOUT")
STDERR=$(cat "$TMPERR")
rm -f "$TMPOUT" "$TMPERR"

# ── Parse and relay output ────────────────────────────────────────
if [ $EXIT_CODE -ne 0 ] && [ -z "$STDOUT" ]; then
    ERROR_MSG=$(printf '%s' "$STDERR" | head -c 2000)
    printf '{"success":false,"error":"claude exited with code %d","stderr":"%s"}\n' \
        "$EXIT_CODE" "$(printf '%s' "$ERROR_MSG" | jq -Rs '.'| sed 's/^"//;s/"$//')"
    exit 0
fi

# Try to parse as JSON and extract key fields
RESULT_JSON=$(printf '%s' "$STDOUT" | jq -c 'if type == "object" then . else empty end' 2>/dev/null || true)

if [ -n "$RESULT_JSON" ]; then
    SUBTYPE=$(printf '%s' "$RESULT_JSON" | jq -r '.subtype // "unknown"')
    RESULT_TEXT=$(printf '%s' "$RESULT_JSON" | jq -r '.result // ""')
    CC_SESSION=$(printf '%s' "$RESULT_JSON" | jq -r '.session_id // ""')
    COST=$(printf '%s' "$RESULT_JSON" | jq -r '.total_cost_usd // 0')
    TURNS=$(printf '%s' "$RESULT_JSON" | jq -r '.num_turns // 0')
    DURATION=$(printf '%s' "$RESULT_JSON" | jq -r '.duration_ms // 0')

    SUCCESS="true"
    if [ "$SUBTYPE" != "success" ]; then
        SUCCESS="false"
    fi

    jq -nc \
        --argjson success "$SUCCESS" \
        --arg subtype "$SUBTYPE" \
        --arg result "$RESULT_TEXT" \
        --arg session_id "$CC_SESSION" \
        --argjson cost "$COST" \
        --argjson turns "$TURNS" \
        --argjson duration_ms "$DURATION" \
        --argjson exit_code "$EXIT_CODE" \
        '{
            success: $success,
            subtype: $subtype,
            result: $result,
            session_id: $session_id,
            cost_usd: $cost,
            turns_used: $turns,
            duration_ms: $duration_ms,
            exit_code: $exit_code
        }'
else
    RAW=$(printf '%s' "$STDOUT" | head -c 8000)
    jq -nc \
        --arg raw "$RAW" \
        --argjson exit_code "$EXIT_CODE" \
        '{
            success: false,
            error: "failed to parse Claude Code JSON output",
            raw_output: $raw,
            exit_code: $exit_code
        }'
fi
