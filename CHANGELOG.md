# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.2] - 2026-02-23

### Fixed

- `ironclad daemon start` now verifies the service is actually running after `launchctl load` — previously reported "Daemon started" even when the service crashed immediately
- `ironclad daemon install` resolves the config path to absolute before embedding in the plist — previously used the relative path which launchd couldn't resolve
- Captures launchctl stderr and checks `LastExitStatus` / PID to give actionable error messages on daemon start failure

## [0.4.1] - 2026-02-23

### Added

- `ironclad daemon start|stop|restart` subcommands for full daemon lifecycle management
- Interactive prompt after `ironclad daemon install` asking whether to start immediately
- `--start` flag on `ironclad daemon install` for non-interactive use
- Dashboard keystore management: save/remove provider API keys from the settings page
- Session nicknames in dashboard sessions table with click-to-copy session ID

### Fixed

- Replaced stale `[providers.local]` (localhost:8080) with `[providers.moonshot]` in bundled and registry provider configs
- Added `moonshot/kimi-k2.5` to dashboard known-models list for settings autocomplete
- `ironclad daemon install` now actually offers to load the service (previously only wrote the plist/unit file)
- `ironclad daemon uninstall` now stops the running service before removing the file
- `ironclad daemon status` distinguishes between "not installed" and "installed but not running"
- Registry URL restored to correct `roboticus.ai/registry` path (not subdomain)
- Empty env vars no longer falsely reported as "configured" in key status checks

### Security

- `delete_provider_key` endpoint now validates provider exists before allowing keystore deletion
- Unified key resolution via `KeySource` enum eliminates 3 duplicated cascade implementations
- `resolve_provider_key` returns `Option<String>` instead of silently sending empty auth headers
- Replace secret-looking test placeholders to prevent false GitGuardian alerts

## [0.4.0] - 2026-02-23

### Added

- Signal channel adapter backed by signal-cli JSON-RPC daemon (`ironclad-channels::signal`)
- Unified thinking indicator (🤖🧠…) for all chat channels (Telegram, WhatsApp, Discord, Signal)
- Configurable `thinking_threshold_seconds` on `[channels]` — estimated latency gate for thinking indicator (default: 30s)
- `send_typing` and `send_ephemeral` on WhatsApp and Discord adapters
- Latency estimator based on model tier, input length, and circuit-breaker state
- LLM fallback chain: `infer_with_fallback` helper retries across configured providers on transient errors
- Permanent error detection in delivery queue — 403/401/400 and "bot blocked" errors dead-letter immediately
- Config auto-discovery: `ironclad start` checks `~/.ironclad/ironclad.toml` when no `--config` flag is given
- Obsidian vault integration module with read, search, and write tools
- GitHub Actions release workflow for cross-platform binaries and crates.io publishing

### Changed

- `thinking_threshold_seconds` moved from per-channel (`TelegramConfig`) to `ChannelsConfig` level
- Channel message processing is now platform-agnostic via `send_typing_indicator` / `send_thinking_indicator` helpers
- Delivery queue `mark_failed` checks for permanent errors before scheduling retries
- Channel router `send_to` and `drain_retry_queue` skip retry enqueue for permanent errors
- Circuit breaker test updated to reflect fallback-first behavior

### Fixed

- LLM inference no longer returns a static error when the primary provider is down — falls through to configured fallbacks
- Telegram bot no longer retries messages to chats it was removed from (permanent error dead-lettering)

## [0.3.0] - 2026-02-23

### Security

- Plugin sandbox: validate tool names against allowlist; reject path-traversal payloads; add `shutdown_all` for graceful teardown
- Browser restrictions: block `file://`, `javascript:`, `data:` URI schemes in CDP navigation; harden Chrome launch flags
- Session role validation: reject messages with roles outside `{user, assistant, system, tool}`
- Channel message authority: trusted sender IDs config for elevated `ChannelAuthority`
- WhatsApp webhook signature verification via HMAC-SHA256
- Docker: run as non-root `ironclad` user
- Wallet: encrypt private keys with machine-derived passphrase; never store plaintext
- API key `#[serde(skip_serializing)]` prevents accidental serialization leakage

### Fixed

- Telegram adapter now processes all updates in a batch, not just the first
- Cron worker dispatches jobs instead of unconditionally marking success
- Cron expressions use the `cron` crate for full syntax support (ranges, lists, steps)
- Per-IP rate-limit HashMap evicted on window reset, preventing unbounded growth
- Interview sessions capped at 100 with 1-hour TTL; expired sessions evicted
- `Cargo.lock` committed; CI builds use `--locked` for reproducible builds
- Graceful shutdown handler (SIGINT + SIGTERM) via `with_graceful_shutdown()`
- Duplicate migration version numbers renumbered to unique sequential IDs
- Migrations wrapped in transactions for atomicity
- SQL `LIKE` patterns escape user-supplied wildcards
- Memory query endpoints clamp limit to 1000

### Changed

- Deduplicated `Optional<T>` trait across 5 DB modules; use `rusqlite::OptionalExtension`
- `SessionStatus` and `MessageRole` enums added for future type-safe migration
- Regex allocation in `decode_common_encodings` hoisted to static `LazyLock`
- Silent `.ok()` calls in `ingest_turn()` replaced with `tracing::warn!` logging
- Reusable `reqwest::Client` stored in `Wallet` for connection pooling
- A2A sessions made private with TTL eviction and 256-session cap
- Plugin registry releases lock before tool execution (`Arc<Mutex<Box<dyn Plugin>>>`)
- `CdpSession::set_timeout` now functional (was a documented no-op)
- Daemon logs written to `~/.ironclad/logs/` instead of world-readable `/tmp/`
- Deduplicated `collect_string_values` across policy rules

### Added

- Pre-commit hook for fast format checks (`hooks/pre-commit`)

## [0.2.0] - 2026-02-21

Initial release with core agent runtime, memory tiers, wallet integration,
channel adapters, browser automation, plugin SDK, and scheduling.
