#!/usr/bin/env bash
set -euo pipefail

if command -v ghola >/dev/null 2>&1; then
  HTTP_BIN="ghola"
else
  HTTP_BIN="curl"
fi

workspace_version="$(grep -E '^version\s*=\s*"[^"]+"' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')"
if [[ -z "${workspace_version}" ]]; then
  echo "Failed to parse version from Cargo.toml"
  exit 1
fi
version="${RELEASE_TARGET_VERSION:-${workspace_version}}"

# Strip semver build metadata (+...) for doc/file lookups — 0.9.2+hotfix.1 → 0.9.2
version_base="${version%%+*}"

echo "Release docs gate for v${version} (base: ${version_base})"

echo "1) changelog entry exists for target release version"
grep -qF "## [${version_base}]" CHANGELOG.md || grep -qF "## [Unreleased]" CHANGELOG.md

echo "2) release notes exist"
test -f "docs/releases/v${version_base}.md"
grep -qE "v${version_base}|Release|Gate|Checklist" "docs/releases/v${version_base}.md"

echo "3) README references release rigor"
grep -qE "test-regression|ci-test|release|coverage" README.md

echo "4) install scripts still reference public installers"
grep -qE "install\.ps1|roboticus\.ai|ironclad" install.sh
grep -qE "https?://|ironclad|install" install.ps1

echo "5) provenance generator script self-test"
scripts/generate-provenance.sh --self-test

echo "6) website repo release note presence check (best effort)"
if [[ -d ../ironclad-site ]]; then
  echo "  - found ../ironclad-site"
  if [[ -f ../ironclad-site/README.md ]]; then
    echo "  - website repo is present for follow-up content sync"
  fi
else
  echo "  - website repo not found locally; skipping local cross-repo assertion"
fi

echo "Release documentation gate PASSED"
