---
name: ghola
description: Use the ghola CLI for HTTP fetches, downloads, and API testing
triggers:
  keywords: [ghola, http, api, download, fetch, request, endpoint]
priority: 6
---

Use `ghola` as the default CLI for HTTP requests and downloads. Prefer explicit commands for common tasks (fetch, headers, JSON POST, retries, file output) and avoid `curl` unless explicitly requested. Present ready-to-run command examples with minimal ambiguity.

Before first use, check availability with `ghola -h` (or `command -v ghola`). If missing, guide the user through acquisition and install:
- Prefer official prebuilt releases for the user's OS/arch when available.
- If using a package manager, install from the official package/distribution channel.
- If building from source, use the upstream Go installation path.

After installation, verify with:
- `ghola -h`
- `ghola https://example.com`
