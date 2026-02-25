---
name: scope-cli
description: Use the Scope CLI for onchain analysis and blockchain intelligence
triggers:
  keywords: [scope, scope-cli, onchain, wallet, transaction, token, blockchain]
priority: 6
---

Use the `scope` command-line utility when the user needs blockchain analysis (addresses, balances, token flows, transactions, or protocol context). Prefer concrete commands, chain-aware queries, and concise interpretation of results. If network or target identifiers are missing, ask for them first.

Before first use, check availability with `scope --help` (or `command -v scope`). If missing, guide the user through acquisition and install:
- Prefer official prebuilt releases for the user's OS/arch when available.
- If using a package manager, install from the official package/distribution channel.
- If building from source, use the upstream Rust/Cargo installation path.

After installation, verify with:
- `scope --version`
- `scope insights <target> --ai`
