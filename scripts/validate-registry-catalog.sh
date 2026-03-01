#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT_DIR/registry/manifest.json"
SKILLS_DIR="$ROOT_DIR/registry/skills"
BUILTINS_FILE="$ROOT_DIR/registry/builtin-skills.json"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required"
  exit 1
fi

if [[ ! -f "$MANIFEST" ]]; then
  echo "Missing manifest: $MANIFEST"
  exit 1
fi

if [[ ! -f "$BUILTINS_FILE" ]]; then
  echo "Missing builtins file: $BUILTINS_FILE"
  exit 1
fi

expected_builtins_sha="$(jq -r '.packs.builtins.sha256 // empty' "$MANIFEST")"
expected_builtins_path="$(jq -r '.packs.builtins.path // empty' "$MANIFEST")"
if [[ -z "$expected_builtins_sha" || -z "$expected_builtins_path" ]]; then
  echo "Manifest must define packs.builtins.sha256 and packs.builtins.path"
  exit 1
fi

actual_builtins_sha="$(shasum -a 256 "$ROOT_DIR/registry/$expected_builtins_path" | awk '{print $1}')"
if [[ "$expected_builtins_sha" != "$actual_builtins_sha" ]]; then
  echo "Builtins checksum mismatch:"
  echo "  expected: $expected_builtins_sha"
  echo "  actual:   $actual_builtins_sha"
  exit 1
fi

manifest_skill_files="$(jq -r '.packs.skills.files | keys[]' "$MANIFEST" | sort)"
disk_skill_files="$(cd "$SKILLS_DIR" && ls -1 *.md | sort)"

if [[ "$manifest_skill_files" != "$disk_skill_files" ]]; then
  echo "Manifest skill file list does not match registry/skills/*.md"
  diff <(echo "$manifest_skill_files") <(echo "$disk_skill_files") || true
  exit 1
fi

while IFS= read -r skill_file; do
  [[ -z "$skill_file" ]] && continue
  expected_sha="$(jq -r --arg f "$skill_file" '.packs.skills.files[$f]' "$MANIFEST")"
  actual_sha="$(shasum -a 256 "$SKILLS_DIR/$skill_file" | awk '{print $1}')"
  if [[ "$expected_sha" != "$actual_sha" ]]; then
    echo "Skill checksum mismatch for $skill_file:"
    echo "  expected: $expected_sha"
    echo "  actual:   $actual_sha"
    exit 1
  fi
done <<<"$manifest_skill_files"

# Guardrail: no duplicate names between built-in skills and downloadable skills.
builtin_names="$(jq -r '.[].name | ascii_downcase' "$BUILTINS_FILE" | sort -u)"
downloadable_names="$(for f in "$SKILLS_DIR"/*.md; do awk -F': ' '/^name:/{print tolower($2); exit}' "$f"; done | sort -u)"
collisions="$(comm -12 <(echo "$builtin_names") <(echo "$downloadable_names"))"
if [[ -n "$collisions" ]]; then
  echo "Built-in/downloadable name collisions detected:"
  echo "$collisions"
  exit 1
fi

echo "Registry catalog validation passed."
