# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
