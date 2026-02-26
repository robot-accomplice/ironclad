# CLI Reference

Ironclad provides a comprehensive CLI for managing the agent runtime. All commands support these global options:

**Global Options:**

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--url <url>` | `IRONCLAD_URL` | `http://127.0.0.1:18789` | Gateway URL for management commands |
| `--profile <name>` | `IRONCLAD_PROFILE` | — | Profile name for state isolation |
| `-c, --config <path>` | `IRONCLAD_CONFIG` | — | Path to configuration file |
| `--color <mode>` | — | `auto` | Color output: `auto`, `always`, `never` |
| `--theme <name>` | `IRONCLAD_THEME` | `crt-green` | Color theme: `crt-green`, `crt-orange`, `terminal` |
| `--no-draw` | — | — | Disable CRT typewriter draw effect |
| `--nerdmode` | `IRONCLAD_NERDMODE` | — | Retro mode: CRT green tint, ASCII symbols, typewriter draw |
| `-q, --quiet` | — | — | Suppress informational output (errors only) |
| `--json` | — | — | Output structured JSON instead of formatted text |

---

## Lifecycle

### `ironclad serve`

Boot the Ironclad runtime. Aliases: `start`, `run`.

```bash
ironclad serve [OPTIONS]
```

**Options:**

| Flag | Description |
|------|-------------|
| `-p, --port <port>` | Override bind port |
| `-b, --bind <address>` | Override bind address |

If no config file is specified, Ironclad looks for `~/.ironclad/ironclad.toml`. Falls back to built-in defaults with an in-memory database if no config is found.

Refuses to start on a non-localhost bind address without `[server] api_key` set.

On startup, Ironclad `0.7.x` auto-migrates legacy provider URLs that still point to `http://127.0.0.1:8788/<provider>` to canonical direct provider base URLs (for example Anthropic/Google), writes the updated `ironclad.toml`, and keeps a one-time backup at `ironclad.toml.bak`.

Legacy loopback provider URLs are deprecated in `0.7.x` and removed in `0.8.0+`. In `0.8.0+`, startup fails fast if any `providers.*.url` still uses `127.0.0.1:8788/<provider>`, and operators must configure direct upstream provider base URLs.

### `ironclad init`

Initialize a new workspace with a starter config and skills directory.

```bash
ironclad init [PATH]
```

**Arguments:**

| Argument | Default | Description |
|----------|---------|-------------|
| `PATH` | `.` | Directory to initialize |

Creates `ironclad.toml` and a `skills/` directory with starter skill definitions.

### `ironclad setup`

Interactive setup wizard. Alias: `onboard`. Walks through provider selection, API key configuration, model testing, and workspace setup.

For local hosts, the wizard is Apertus-aware and SGLang-first:

- Recommends `sglang` as the default local provider for Apertus when no existing local model framework/model cache is detected.
- Offers `vllm`, `docker-model-runner`, and `ollama` as fallback local hosts.
- Detects RAM and only offers resource-appropriate Apertus variants (`8B` always preferred; `70B` only on high-memory systems).
- Keeps all local-host setup/download steps optional and non-fatal.

```bash
ironclad setup
```

### `ironclad check`

Validate configuration file syntax and semantics.

```bash
ironclad check [-c CONFIG]
```

Checks TOML syntax, field validation, memory budget sums, treasury constraints, provider reachability, and skills directory existence.

### `ironclad version`

Display firmware version, Rust edition, target architecture, and OS.

```bash
ironclad version
```

### `ironclad update`

Check for and install updates. Has several subcommands:

#### `ironclad update check`

Show available updates without installing.

```bash
ironclad update check [OPTIONS]
```

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--channel <ch>` | — | `stable` | Update channel: `stable`, `beta`, `dev` |
| `--registry-url <url>` | `IRONCLAD_REGISTRY_URL` | — | Override registry URL for content packs |

#### `ironclad update all`

Update everything (binary + content packs).

```bash
ironclad update all [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--channel <ch>` | Update channel (default: `stable`) |
| `--yes` | Auto-accept unmodified files (still prompts for conflicts) |
| `--no-restart` | Don't restart daemon after update |
| `--registry-url <url>` | Override registry URL |

#### `ironclad update binary`

Update the Ironclad binary.

By default, Ironclad now downloads the platform-specific release archive from GitHub Releases, verifies it against the published SHA256 fingerprint in `SHA256SUMS.txt`, then replaces the binary.

On Windows, replacement is staged and finalized just after the current process exits.

Use `--method build` if you prefer to rebuild via `cargo install`.

`--method build` requires a local native build toolchain with a C compiler available on `PATH`.

> On Windows, `--method build` is not supported in-process because running executables are file-locked by the OS.
> Use a fresh PowerShell session instead:
>
> ```powershell
> ironclad daemon stop
> cargo install ironclad-server --force
> ironclad daemon start
> ```

```bash
ironclad update binary [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--channel <ch>` | Update channel (default: `stable`) |
| `--yes` | Auto-accept if newer version is available |
| `--method <download&#124;build>` | Update strategy (default: `download`) |

#### `ironclad update providers`

Update bundled provider configurations.

```bash
ironclad update providers [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--yes` | Auto-accept unmodified files |
| `--registry-url <url>` | Override registry URL |

#### `ironclad update skills`

Update blessed skill pack.

```bash
ironclad update skills [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--yes` | Auto-accept unmodified files |
| `--registry-url <url>` | Override registry URL |

---

## Operations

### `ironclad status`

Display system status including agent state, model info, memory usage, and channel status.

```bash
ironclad status
```

### `ironclad mechanic`

Run diagnostics and self-repair. Alias: `doctor`.
The mechanic also performs signature-based incident analysis from recent runtime logs and channel telemetry (for example, detecting repeated Telegram API 404 loops and surfacing targeted repair commands), and flags delegation-integrity risk when subagents are enabled but not running.

```bash
ironclad mechanic [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-r, --repair` | Attempt to auto-repair issues |
| `--json` | Emit machine-readable findings (severity, confidence, repair plans) |
| `--allow-job <name[,name...]>` | In `--repair` mode, re-enable allowlisted `paused_unknown_action` cron jobs |

### `ironclad logs`

View and tail server logs.

```bash
ironclad logs [OPTIONS]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-n, --lines <n>` | `50` | Number of lines to show |
| `-f, --follow` | — | Follow log output (stream) |
| `-l, --level <level>` | `info` | Minimum log level: `trace`, `debug`, `info`, `warn`, `error` |

### `ironclad circuit`

Inspect and manage circuit breaker state.

#### `ironclad circuit status`

Show circuit breaker status for all providers.

#### `ironclad circuit reset`

Reset all tripped circuit breakers.

---

## Data

### `ironclad sessions`

Manage conversation sessions.

#### `ironclad sessions list`

List all sessions with IDs, agents, nicknames, and timestamps.

#### `ironclad sessions show <ID>`

Show session details and message history.

#### `ironclad sessions create <AGENT_ID>`

Create a new session for the specified agent.

#### `ironclad sessions export <ID>`

Export a session to a file.

| Flag | Default | Description |
|------|---------|-------------|
| `-f, --format <fmt>` | `json` | Output format: `json`, `html`, `markdown` |
| `-o, --output <path>` | stdout | Output file path |

#### `ironclad sessions backfill-nicknames`

Generate nicknames for all sessions that are missing one.

### `ironclad memory`

Browse and search memory banks.

#### `ironclad memory list <TIER>`

List entries in a memory tier (`working`, `episodic`, `semantic`).

| Flag | Description |
|------|-------------|
| `-s, --session <id>` | Session ID (required for `working` tier) |
| `-l, --limit <n>` | Limit results |

#### `ironclad memory search <QUERY>`

Search across all memory tiers.

| Flag | Description |
|------|-------------|
| `-l, --limit <n>` | Limit results |

### `ironclad skills`

Manage agent skills.

#### `ironclad skills list`

List all registered skills.

#### `ironclad skills show <ID>`

Show skill details including description, kind, and parameters.

#### `ironclad skills reload`

Reload skills from disk.

#### `ironclad skills catalog-list [--query <q>]`

List (or search) catalog skill entries from built-ins + configured registry.

#### `ironclad skills catalog-install <SKILL...> [--activate]`

Install one or more skills from the catalog with checksum verification and rollback-on-failure semantics.

#### `ironclad skills catalog-activate [SKILL...]`

Activate installed skills by reloading the runtime registry.

The dashboard `Skills` page also exposes this flow with catalog checkboxes and actions for `Install selected`, `Activate selected`, and `Install + Activate`.

#### `ironclad skills import <SOURCE>`

Import skills from an OpenClaw workspace or `.tar.gz` archive.

| Flag | Description |
|------|-------------|
| `--no-safety-check` | Skip safety checks (dangerous) |
| `--accept-warnings` | Auto-accept warnings (still blocks on critical findings) |

#### `ironclad skills export`

Export skills to a portable archive.

| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output <path>` | `ironclad-skills-export.tar.gz` | Output archive path |
| `IDS...` | all | Specific skill IDs to export |

### `ironclad schedule`

View scheduled tasks. Alias: `cron`.

#### `ironclad schedule list`

List all scheduled cron jobs.

### `ironclad metrics`

View metrics and cost telemetry.

#### `ironclad metrics costs`

Show inference cost breakdown by model and provider.

#### `ironclad metrics transactions`

Show transaction history.

| Flag | Description |
|------|-------------|
| `-H, --hours <n>` | Time window in hours |

#### `ironclad metrics cache`

Show semantic cache hit/miss statistics.

### `ironclad wallet`

Inspect wallet and treasury.

#### `ironclad wallet show`

Show wallet overview including balance, chain, and treasury policy.

#### `ironclad wallet address`

Display the wallet's on-chain address.

#### `ironclad wallet balance`

Check wallet balance.

---

## Authentication

### `ironclad auth`

Manage OAuth authentication for providers.

#### `ironclad auth login`

Log in to a provider via OAuth (PKCE flow).

| Flag | Description |
|------|-------------|
| `--provider <name>` | Provider name (e.g., `anthropic`) |
| `--client-id <id>` | OAuth client ID (overrides config) |

Opens a browser for authorization and listens for the callback on a local port.

#### `ironclad auth status`

Show OAuth token status for all authenticated providers.

#### `ironclad auth logout`

Remove stored OAuth tokens for a provider.

| Flag | Description |
|------|-------------|
| `--provider <name>` | Provider name |

---

## Configuration

### `ironclad config`

Read and write configuration values.

#### `ironclad config show`

Show the running configuration (from the gateway).

#### `ironclad config get <PATH>`

Get a config value by TOML dotted path (e.g., `models.primary`).

#### `ironclad config set <PATH> <VALUE>`

Set a config value in the config file, then immediately apply it to the running runtime (if reachable).

| Flag | Default | Description |
|------|---------|-------------|
| `-f, --file <path>` | `ironclad.toml` | Config file to modify |
| `--no-apply` | `false` | Skip immediate runtime apply via `/api/config` |

#### `ironclad config unset <PATH>`

Remove a config key from the config file, then immediately apply it to the running runtime (if reachable).

| Flag | Default | Description |
|------|---------|-------------|
| `-f, --file <path>` | `ironclad.toml` | Config file to modify |
| `--no-apply` | `false` | Skip immediate runtime apply via `/api/config` |

#### `ironclad config lint`

Parse and validate a config file (schema + semantic validation) without applying it.

| Flag | Default | Description |
|------|---------|-------------|
| `-f, --file <path>` | `ironclad.toml` | Config file to validate |

#### `ironclad config backup`

Create a timestamped backup of a config file.

| Flag | Default | Description |
|------|---------|-------------|
| `-f, --file <path>` | `ironclad.toml` | Config file to back up |

### `ironclad models`

Discover and manage models.

#### `ironclad models list`

List all configured models with their providers and tiers.

#### `ironclad models scan`

Scan providers for available models.

Model discovery mode is provider-aware:

- Ollama-like providers use `/api/tags`.
- OpenAI-compatible providers (including SGLang, vLLM, and Docker Model Runner frontends) use `/v1/models`.

| Argument | Description |
|----------|-------------|
| `PROVIDER` | Optional provider to scan (e.g., `ollama`, `openai`). Omit to scan all |

### `ironclad plugins`

Manage plugins.

#### `ironclad plugins list`

List installed plugins with status and version.

#### `ironclad plugins info <NAME>`

Show plugin details including tools and configuration.

#### `ironclad plugins install <SOURCE>`

Install a plugin from a directory.

#### `ironclad plugins uninstall <NAME>`

Uninstall a plugin.

#### `ironclad plugins enable <NAME>`

Enable a disabled plugin.

#### `ironclad plugins disable <NAME>`

Disable a plugin.

### `ironclad agents`

Manage agents in a multi-agent setup.

#### `ironclad agents list`

List all agents with status.

#### `ironclad agents start <ID>`

Start an agent.

#### `ironclad agents stop <ID>`

Stop an agent.

### `ironclad channels`

Inspect channel adapters.

Channel delivery retries are durable in v0.8.0 flows:

- Retryable outbound deliveries are persisted to the local database.
- Pending/in-flight delivery items are recovered on restart before retry draining resumes.
- Permanent failures move to dead-letter state for operator inspection/replay workflows.

#### `ironclad channels list`

List channel adapters and their status.

#### `ironclad channels dead-letter [--limit <n>]`

List dead-letter outbound deliveries with channel, attempt counts, and last error.

#### `ironclad channels replay <ID>`

Replay a dead-letter item by id (moves it back to pending retry state).

### `ironclad security`

Security audit and hardening.

#### `ironclad security audit`

Run a security audit on configuration and file permissions.

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config <path>` | `ironclad.toml` | Config file to audit |

Checks: API key presence, file permissions, wallet encryption, skill sandbox settings, bind address security, and more.

Rate-limit/operator notes:

- Configure per-window quotas with `server.rate_limit_requests`, `server.per_ip_rate_limit_requests`, and `server.per_actor_rate_limit_requests`.
- Configure trusted proxy hops with `server.trusted_proxy_cidrs` before relying on forwarded client IP headers.
- Watch for `429 rate_limit_exceeded` responses to identify hot actors or abusive IPs.

---

## Credentials

### `ironclad keystore`

Manage the encrypted credential store. By default, uses a machine-derived key (based on hostname + username). A custom passphrase can be provided with `--password`.
All successful keystore mutations are also appended to `~/.ironclad/keystore.audit.log` as JSON lines (operation, key name, timestamp, process metadata). Secret values are never logged.

#### `ironclad keystore set <KEY> [VALUE]`

Store a secret. Omit `VALUE` for an interactive secure prompt.

| Flag | Description |
|------|-------------|
| `--password <pass>` | Custom passphrase |

#### `ironclad keystore get <KEY>`

Retrieve and print a secret value to stdout.

| Flag | Description |
|------|-------------|
| `--password <pass>` | Custom passphrase |

#### `ironclad keystore list`

List all stored secret names (not values).

| Flag | Description |
|------|-------------|
| `--password <pass>` | Custom passphrase |

#### `ironclad keystore remove <KEY>`

Remove a secret.

| Flag | Description |
|------|-------------|
| `--password <pass>` | Custom passphrase |

#### `ironclad keystore import <PATH>`

Import secrets from a JSON file with `{"key": "value", ...}` format.

| Flag | Description |
|------|-------------|
| `--password <pass>` | Custom passphrase |

#### `ironclad keystore rekey`

Change the keystore passphrase. Prompts interactively for the new passphrase.

| Flag | Description |
|------|-------------|
| `--password <pass>` | Current passphrase |

---

## Migration

### `ironclad migrate`

Migrate data between OpenClaw and Ironclad formats.

#### `ironclad migrate import <SOURCE>`

Import data from an OpenClaw workspace into Ironclad.

| Flag | Description |
|------|-------------|
| `-a, --areas <list>` | Comma-separated areas to import (default: all) |
| `--yes` | Skip confirmation prompts |
| `--no-safety-check` | Skip safety checks on skill scripts |

#### `ironclad migrate export <TARGET>`

Export Ironclad data to OpenClaw format.

| Flag | Description |
|------|-------------|
| `-a, --areas <list>` | Comma-separated areas to export (default: all) |

---

## System

### `ironclad daemon`

Manage the background daemon service.

#### `ironclad daemon install`

Install the daemon as a LaunchAgent (macOS), systemd user service (Linux), or managed detached user process (Windows).

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config <path>` | `ironclad.toml` | Config file path |
| `--start` | — | Start immediately without prompting |

#### `ironclad daemon start`

Start the daemon.

#### `ironclad daemon stop`

Stop the daemon.

#### `ironclad daemon restart`

Restart the daemon.

#### `ironclad daemon status`

Show daemon status (installed, running, PID when available).

#### `ironclad daemon uninstall`

Uninstall the daemon service.

### `ironclad web`

Open the web dashboard in the default browser.

```bash
ironclad web
```

### `ironclad reset`

Reset state to factory defaults.

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompt |

### `ironclad uninstall`

Uninstall the Ironclad daemon and optionally remove all data.

| Flag | Description |
|------|-------------|
| `--purge` | Also remove `~/.ironclad/` data directory |

### `ironclad completion`

Generate shell completions.

```bash
ironclad completion <SHELL>
```

| Argument | Description |
|----------|-------------|
| `SHELL` | Target shell: `bash`, `zsh`, `fish` |

**Usage example:**

```bash
# Bash
ironclad completion bash > ~/.bash_completion.d/ironclad

# Zsh
ironclad completion zsh > ~/.zfunc/_ironclad

# Fish
ironclad completion fish > ~/.config/fish/completions/ironclad.fish
```
