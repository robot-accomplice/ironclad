#!/usr/bin/env python3
"""
One-time migration: move sensitive credentials from SQLite identity table
into the encrypted keystore.

Usage:
    python3 tools/migrate-sensitive-to-keystore.py             # dry-run
    python3 tools/migrate-sensitive-to-keystore.py --commit    # write to keystore, remove from SQLite

Requires: the ironclad binary to be built (uses `ironclad keystore import`).
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path


IRONCLAD_DB = Path.home() / ".ironclad" / "state.db"

SENSITIVE_PREFIXES = [
    "pg:kv:bluesky_credentials",
    "pg:kv:conway_api_key",
    "pg:kv:etherscan_api_key",
    "pg:kv:linkedin_oauth",
    "pg:kv:moltbook_credentials",
    "pg:kv:x_credentials",
    "pg:kv:twitter_duncan",
    "pg:kv:WALLET_ENCRYPTION_KEY",
]


def main():
    parser = argparse.ArgumentParser(
        description="Migrate sensitive identity rows into the encrypted keystore"
    )
    parser.add_argument("--commit", action="store_true")
    parser.add_argument("--db", type=Path, default=IRONCLAD_DB)
    args = parser.parse_args()

    if not args.db.exists():
        print(f"ERROR: {args.db} not found", file=sys.stderr)
        sys.exit(1)

    conn = sqlite3.connect(str(args.db))
    placeholders = ",".join("?" for _ in SENSITIVE_PREFIXES)
    rows = conn.execute(
        f"SELECT key, value FROM identity WHERE key IN ({placeholders})",
        SENSITIVE_PREFIXES,
    ).fetchall()

    if not rows:
        print("No sensitive rows found in identity table.")
        return

    print(f"\nFound {len(rows)} sensitive entries:")
    keystore_entries = {}
    for key, value in rows:
        short_key = key.replace("pg:kv:", "")
        print(f"  {short_key}")
        keystore_entries[short_key] = value

    if not args.commit:
        print(f"\nDRY RUN — would import {len(keystore_entries)} entries to keystore")
        print("         and remove them from the identity table.")
        print("         Run with --commit to proceed.")
        conn.close()
        return

    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
        json.dump(keystore_entries, f)
        tmp_path = f.name

    print(f"\nImporting {len(keystore_entries)} entries into keystore...")
    result = subprocess.run(
        ["cargo", "run", "-p", "ironclad-server", "--", "keystore", "import", tmp_path],
        capture_output=True,
        text=True,
        cwd=str(Path(__file__).parent.parent),
    )

    Path(tmp_path).unlink(missing_ok=True)

    if result.returncode != 0:
        print(f"ERROR: keystore import failed:\n{result.stderr}", file=sys.stderr)
        sys.exit(1)

    print(result.stderr.strip() if result.stderr.strip() else result.stdout.strip())

    print(f"\nRemoving {len(rows)} plaintext entries from identity table...")
    conn.execute(
        f"DELETE FROM identity WHERE key IN ({placeholders})",
        SENSITIVE_PREFIXES,
    )
    conn.commit()
    conn.close()

    print(f"Done. {len(rows)} secrets moved to encrypted keystore.")


if __name__ == "__main__":
    main()
