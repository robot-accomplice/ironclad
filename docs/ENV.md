# Environment Variables

Ironclad reads environment variables for API keys, CLI configuration, and runtime behavior. This document covers every environment variable recognized by the system.

---

## CLI Configuration

These variables are read by the `ironclad` CLI binary and can be used instead of command-line flags.

| Variable | CLI Flag | Default | Description |
|----------|----------|---------|-------------|
| `IRONCLAD_URL` | `--url` | `http://127.0.0.1:18789` | Gateway URL for CLI management commands |
| `IRONCLAD_CONFIG` | `-c, --config` | — | Path to config file |
| `IRONCLAD_PROFILE` | `--profile` | — | Profile name for state isolation |
| `IRONCLAD_THEME` | `--theme` | `crt-green` | Color theme: `crt-green`, `crt-orange`, `terminal` |
| `IRONCLAD_NERDMODE` | `--nerdmode` | — | Enable retro CRT mode |

---

## API Keys

Provider API keys are resolved from environment variables specified in the provider config's `api_key_env` field. These are the defaults from the bundled provider configurations.

| Variable | Description | Required |
|----------|-------------|----------|
| `OPENAI_API_KEY` | OpenAI API key | If using OpenAI models |
| `ANTHROPIC_API_KEY` | Anthropic API key | If using Claude models |
| `GOOGLE_API_KEY` | Google AI (Gemini) API key | If using Google models |
| `OPENROUTER_API_KEY` | OpenRouter API key | If using OpenRouter |
| `MOONSHOT_API_KEY` | Moonshot API key | If using Moonshot models |

Custom providers can reference any environment variable via `api_key_env`:

```toml
[providers.custom]
url = "https://api.custom.ai"
tier = "T3"
api_key_env = "CUSTOM_API_KEY"
```

---

## Channel Tokens

Channel adapter tokens are resolved from the `token_env` field in each channel's config section.

| Variable | Config Field | Description |
|----------|-------------|-------------|
| `TELEGRAM_BOT_TOKEN` | `channels.telegram.token_env` | Telegram Bot API token |
| `WHATSAPP_TOKEN` | `channels.whatsapp.token_env` | WhatsApp Business API access token |
| `DISCORD_BOT_TOKEN` | `channels.discord.token_env` | Discord bot token |

The actual variable name depends on what you set in `token_env`. For example:

```toml
[channels.telegram]
enabled = true
token_env = "MY_TG_TOKEN"  # reads $MY_TG_TOKEN at runtime
```

Alternatively, use `token_ref` to read from the encrypted keystore instead of an environment variable.

### Email

| Variable | Config Field | Description |
|----------|-------------|-------------|
| *(user-defined)* | `channels.email.password_env` | Email account password |

---

## MCP Client Authentication

MCP client auth tokens are resolved from `auth_token_env` per client entry:

```toml
[[mcp.clients]]
name = "github"
url = "http://localhost:4000"
auth_token_env = "GITHUB_MCP_TOKEN"
```

| Variable | Description |
|----------|-------------|
| *(user-defined)* | MCP server authentication token, as specified in `auth_token_env` |

---

## OAuth

| Variable | Description |
|----------|-------------|
| `IRONCLAD_OAUTH_CLIENT_ID` | Fallback OAuth client ID when not set in provider config or CLI flag |

OAuth tokens are stored in `~/.ironclad/oauth_tokens.json` after `ironclad auth login`.

---

## Wallet

| Variable | Description |
|----------|-------------|
| `IRONCLAD_WALLET_PASSPHRASE` | Passphrase for encrypting/decrypting the wallet keystore file. If unset, uses machine-derived key |

---

## Update System

| Variable | CLI Flag | Description |
|----------|----------|-------------|
| `IRONCLAD_REGISTRY_URL` | `--registry-url` | Override the content pack registry URL for `ironclad update` commands |

---

## Display and Formatting

| Variable | Description |
|----------|-------------|
| `NO_COLOR` | When set to any non-empty value, disables color output (per [no-color.org](https://no-color.org)) |

---

## System Variables

These standard system variables are read by Ironclad for path resolution, identity derivation, and sandboxing.

| Variable | Used For |
|----------|----------|
| `HOME` | Home directory for `~` expansion, config/data paths, daemon setup |
| `USER` | Username for machine-derived keystore passphrase and wallet identity |
| `USERNAME` | Fallback for `USER` (Windows) |
| `HOSTNAME` | Hostname for machine-derived keystore passphrase |
| `HOST` | Fallback for `HOSTNAME` |
| `PATH` | Inherited by sandboxed skill scripts and used for binary discovery |
| `LANG` | Inherited by sandboxed skill scripts |
| `TERM` | Inherited by sandboxed skill scripts |
| `TMPDIR` | Inherited by sandboxed skill scripts |

---

## Skill Script Sandbox

When `[skills] sandbox_env = true` (default), skill scripts run in a sanitized environment that only inherits these variables from the parent process:

- `PATH`
- `HOME`
- `USER`
- `LANG`
- `TERM`
- `TMPDIR`

All other environment variables (including API keys) are **not** passed to skill scripts.

---

## Build-Time Variables

These are used during compilation and are not runtime-configurable:

| Variable | Description |
|----------|-------------|
| `CARGO_PKG_VERSION` | Injected by Cargo; used for version display and health endpoint |
| `CARGO_BIN_EXE_ironclad` | Used in integration tests to locate the built binary |

---

## Precedence

For settings that can be configured via both environment variables and config file:

1. **CLI flags** take highest precedence
2. **Environment variables** override config file values (for CLI flags with `env =` in clap)
3. **Config file** (`ironclad.toml`) values
4. **Built-in defaults**

For API keys and tokens specifically:

1. **Keystore** (`api_key_ref` / `token_ref`) — encrypted, preferred
2. **Environment variable** (`api_key_env` / `token_env`) — plaintext in process environment
3. **Inline config** (`api_key` in `[server]`) — plaintext in file, least preferred
