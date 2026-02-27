#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/generate-provenance.sh --artifact-dir <dir> --output <file>
  scripts/generate-provenance.sh --self-test
EOF
}

build_manifest() {
  local artifact_dir="$1"
  local output_file="$2"

  if [[ ! -d "$artifact_dir" ]]; then
    echo "artifact directory not found: $artifact_dir" >&2
    return 1
  fi

  local tmp
  tmp="$(mktemp)"
  printf '[]' >"$tmp"

  shopt -s nullglob
  local file
  for file in "$artifact_dir"/ironclad-*.tar.gz "$artifact_dir"/ironclad-*.zip; do
    local sha_file="${file}.sha256"
    if [[ ! -f "$sha_file" ]]; then
      echo "missing checksum sidecar for $(basename "$file")" >&2
      return 1
    fi
    local sha
    sha="$(tr -d '[:space:]' <"$sha_file")"
    if [[ ! "$sha" =~ ^[0-9a-f]{64}$ ]]; then
      echo "invalid sha256 in $(basename "$sha_file")" >&2
      return 1
    fi
    local size
    size="$(wc -c <"$file" | tr -d '[:space:]')"
    jq \
      --arg name "$(basename "$file")" \
      --arg sha256 "$sha" \
      --arg size_bytes "$size" \
      '. + [{name: $name, sha256: $sha256, size_bytes: ($size_bytes | tonumber)}]' \
      "$tmp" >"${tmp}.next"
    mv "${tmp}.next" "$tmp"
  done
  shopt -u nullglob

  local generated_at
  generated_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

  jq -n \
    --arg generated_at "$generated_at" \
    --arg git_sha "${GITHUB_SHA:-unknown}" \
    --arg ref "${GITHUB_REF:-unknown}" \
    --arg run_id "${GITHUB_RUN_ID:-local}" \
    --arg run_attempt "${GITHUB_RUN_ATTEMPT:-0}" \
    --arg actor "${GITHUB_ACTOR:-local}" \
    --slurpfile artifacts "$tmp" \
    '{
      generated_at: $generated_at,
      source: {
        git_sha: $git_sha,
        git_ref: $ref,
        run_id: $run_id,
        run_attempt: $run_attempt,
        actor: $actor
      },
      artifacts: ($artifacts[0] | sort_by(.name))
    }' >"$output_file"

  rm -f "$tmp"
}

self_test() {
  local test_dir out_file
  test_dir="$(mktemp -d)"
  out_file="$test_dir/provenance.json"
  trap 'rm -rf "$test_dir"' RETURN

  printf 'abc' >"$test_dir/ironclad-0.0.0-x86_64-linux.tar.gz"
  printf 'def' >"$test_dir/ironclad-0.0.0-x86_64-windows.zip"
  printf 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad' >"$test_dir/ironclad-0.0.0-x86_64-linux.tar.gz.sha256"
  printf 'cb8379ac2098aa165029e3938a51da0bcecfc008fd6795f401178647f96c5b34' >"$test_dir/ironclad-0.0.0-x86_64-windows.zip.sha256"

  build_manifest "$test_dir" "$out_file"
  jq -e '.artifacts | length == 2' "$out_file" >/dev/null
  jq -e '.artifacts[0].sha256 | test("^[0-9a-f]{64}$")' "$out_file" >/dev/null
  jq -e '.source.run_id != null' "$out_file" >/dev/null
  echo "provenance self-test passed"
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

artifact_dir=""
output_file=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-dir)
      artifact_dir="${2:-}"
      shift 2
      ;;
    --output)
      output_file="${2:-}"
      shift 2
      ;;
    *)
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$artifact_dir" || -z "$output_file" ]]; then
  usage
  exit 1
fi

build_manifest "$artifact_dir" "$output_file"
echo "wrote provenance manifest to $output_file"
