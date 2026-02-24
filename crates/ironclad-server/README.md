# ironclad-server

> **Version 0.5.0**

Autonomous agent runtime built in Rust as a single optimized binary. Top-level assembly point that wires all workspace crates together: HTTP API (axum, 50+ routes), embedded dashboard SPA, CLI (24 commands), WebSocket event push, and application bootstrap.

Part of the [Ironclad](https://github.com/robot-accomplice/ironclad) workspace.

## New in 0.5.0

- **Turns API** -- Browse individual turns with tool call details
- **Feedback API** -- Per-turn user feedback (thumbs up/down, corrections)
- **Efficiency metrics** -- Track and trend agent efficiency over time
- **Recommendations API** -- Proactive suggestions from conversation analysis
- **SSE streaming** -- Server-sent events for real-time streaming responses

## Key Modules

| Module | Description |
|--------|-------------|
| `api/routes/` | REST API route handlers (`build_router()`) |
| `auth` | API key authentication layer |
| `rate_limit` | Global + per-IP rate limiting middleware |
| `dashboard` | Embedded SPA (compile-time `include_dir!` or filesystem fallback) |
| `ws` | WebSocket push via `EventBus` (tokio broadcast) |
| `cli/` | CLI commands (clap): serve, status, sessions, memory, wallet, etc. |
| `daemon` | Daemon install/status/uninstall |
| `migrate/` | Migration engine, skill import/export |
| `plugins` | Plugin registry initialization |

## Installation

```bash
cargo install ironclad-server
```

Windows (PowerShell):

```powershell
irm https://roboticus.ai/install.ps1 | iex
```

## Usage

```bash
# Initialize configuration
ironclad init

# Start the server
ironclad serve

# Check system health
ironclad mechanic
```

## Documentation

- CLI help: `ironclad --help`
- API docs: [docs.rs](https://docs.rs/ironclad-server)
- Full documentation: [github.com/robot-accomplice/ironclad](https://github.com/robot-accomplice/ironclad)

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
