#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${PORT:-}" ]]; then
  if command -v python3 >/dev/null 2>&1; then
    PORT="$(python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"
  else
    PORT="18789"
  fi
fi
BASE_URL="${BASE_URL:-http://127.0.0.1:${PORT}}"
TMP_DIR="$(mktemp -d)"
CONFIG_FILE="${TMP_DIR}/ironclad.toml"
SERVER_LOG="${TMP_DIR}/ironclad-server.log"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

cat >"$CONFIG_FILE" <<EOF
[agent]
name = "UATBot"
id = "uat-bot"

[server]
bind = "127.0.0.1"
port = ${PORT}

[database]
path = "${TMP_DIR}/state.db"

[models]
primary = "ollama/qwen3:8b"
EOF

echo "Building ironclad binary for UAT..."
cargo build --bin ironclad --locked 2>&1
IRONCLAD_BIN="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r '.target_directory')/debug/ironclad"

echo "Starting local server for UAT: ${BASE_URL}"
"$IRONCLAD_BIN" serve -c "$CONFIG_FILE" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

if command -v ghola >/dev/null 2>&1; then
  HTTP_BIN="ghola"
else
  HTTP_BIN="curl"
fi

for _ in $(seq 1 40); do
  if "$HTTP_BIN" -fsS "${BASE_URL}/api/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

if ! "$HTTP_BIN" -fsS "${BASE_URL}/api/health" >/dev/null 2>&1; then
  echo "Server did not become healthy. Log:"
  cat "$SERVER_LOG"
  exit 1
fi

echo "Running CLI UAT smoke"
BASE_URL="$BASE_URL" bash scripts/run-uat-cli-smoke.sh

echo "Running Web UAT smoke"
BASE_URL="$BASE_URL" bash scripts/run-uat-web-smoke.sh

echo "UAT stack PASSED"
