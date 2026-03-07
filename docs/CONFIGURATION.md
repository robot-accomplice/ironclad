# Configuration Reference

Ironclad is configured via a TOML file, typically at `~/.ironclad/ironclad.toml`. Paths starting with `~` are automatically expanded to the user's home directory.

A minimal config requires only the `[agent]`, `[server]`, `[database]`, and `[models]` sections. All other sections have sensible defaults and can be omitted.

## Runtime Reload Workflow

`ironclad.toml` is runtime-reloadable. Ironclad now treats the file as the source of truth for config mutation flows:

1. Validate candidate config (parse + schema + semantic checks).
2. Create a timestamped backup of the current file.
3. Persist the new config atomically.
4. Apply the config to runtime state immediately.

Operationally, this enables the workflow:

- backup -> edit -> lint -> apply/reload

Notes:

- `/api/config` persists updates to `ironclad.toml` and applies them immediately.
- `/api/config/status` exposes last apply result, backup path, and deferred-apply hints.
- Some fields may still be marked deferred for full effect (for example listener bind/port), even though they are persisted immediately.

```toml
[agent]
name = "Roboticus"
id = "roboticus"

[server]
port = 18789

[database]
path = "~/.ironclad/state.db"

[models]
primary = "ollama/qwen3:8b"
```

---

## `[agent]`

Core agent identity.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | `String` | **required** | The agent's display name |
| `id` | `String` | **required** | Unique agent identifier (must be non-empty) |
| `workspace` | `PathBuf` | `~/.ironclad/workspace` | Path to the agent's workspace directory |
| `log_level` | `String` | `"info"` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |
| `delegation_enabled` | `bool` | `true` | Enable decomposition-gate evaluation and delegation policies |
| `delegation_min_complexity` | `f64` | `0.35` | Minimum complexity score before decomposition is considered |
| `delegation_min_utility_margin` | `f64` | `0.15` | Minimum delegation utility margin required to prefer subtask delegation over centralized execution |
| `specialist_creation_requires_approval` | `bool` | `true` | Legacy key name; requires explicit user approval before creating a new subagent when no existing subagent fits |

When subagent creation approval is required, operators can request an explicit configuration preview before approving creation (review-first flow).

---

## `[server]`

HTTP server settings.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `port` | `u16` | `18789` | HTTP server port |
| `bind` | `String` | `"127.0.0.1"` | Bind address. Non-localhost binds require `api_key` |
| `api_key` | `String?` | `None` | API key for request authentication. Required when binding to non-localhost |
| `log_dir` | `PathBuf` | `~/.ironclad/logs` | Directory for structured JSON log files |
| `log_max_days` | `u32` | `7` | Number of days to retain log files |
| `rate_limit_requests` | `u32` | `100` | Maximum requests per rate-limit window |
| `rate_limit_window_secs` | `u64` | `60` | Rate-limit window duration in seconds |
| `per_ip_rate_limit_requests` | `u32` | `300` | Per-client-IP request budget per window |
| `per_actor_rate_limit_requests` | `u32` | `200` | Per-authenticated-actor request budget per window |
| `trusted_proxy_cidrs` | `String[]` | `[]` | CIDR ranges trusted to supply forwarded client IP headers |

---

## `[database]`

SQLite database configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `path` | `PathBuf` | `~/.ironclad/state.db` | Database file path. Use `":memory:"` for ephemeral storage |

---

## `[models]`

LLM model selection and routing.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `primary` | `String` | **required** | Primary model in `provider/model` format (e.g., `ollama/qwen3:8b`) |
| `fallbacks` | `Vec<String>` | `[]` | Ordered list of fallback models |
| `stream_by_default` | `bool` | `false` | Enable streaming responses by default |
| `model_overrides` | `Map<String, ModelOverride>` | `{}` | Per-model tier and cost overrides |

### `[models.routing]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mode` | `String` | `"metascore"` | Routing strategy: `primary` or `metascore` |
| `confidence_threshold` | `f64` | `0.9` | Minimum confidence for local model routing |
| `local_first` | `bool` | `true` | Prefer local models when confidence is sufficient |
| `cost_aware` | `bool` | `false` | Factor cost into routing decisions |
| `estimated_output_tokens` | `u32` | `500` | Estimated output tokens for cost calculations |

Dashboard note:
- The routing profile UI maps `correctness`, `cost`, and `speed` to these fields.
- UI guardrails enforce `correctness + cost + speed <= 1.0` before sending `PUT /api/config`.

### `[models.tiered_inference]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable tiered inference (try cheaper models first) |
| `confidence_floor` | `f64` | `0.6` | Minimum confidence before escalating to a higher tier |
| `escalation_latency_budget_ms` | `u64` | `3000` | Maximum latency budget (ms) for escalation attempts |

### `[models.model_overrides."provider/model"]`

Per-model overrides keyed by the full `provider/model` string.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `tier` | `String?` | `None` | Override model tier (e.g., `"T4"`) |
| `cost_per_input_token` | `f64?` | `None` | Override input token cost |
| `cost_per_output_token` | `f64?` | `None` | Override output token cost |

---

## `[providers.<name>]`

Provider configuration. Ironclad ships with bundled defaults for `ollama`, `openai`, `anthropic`, `google`, `openrouter`, `moonshot`, and `llama-cpp`. User-defined entries override bundled ones.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | `String` | **required** | Provider base URL |
| `tier` | `String` | **required** | Provider tier: `T1` (local), `T2` (mid), `T3` (cloud), `T4` (premium) |
| `format` | `String?` | `None` | API format: `openai`, `anthropic`, `google` |
| `api_key_env` | `String?` | `None` | Environment variable containing the API key |
| `api_key_ref` | `String?` | `None` | Keystore secret name for the API key |
| `chat_path` | `String?` | `None` | Chat completions endpoint path |
| `embedding_path` | `String?` | `None` | Embedding endpoint path |
| `embedding_model` | `String?` | `None` | Default embedding model name |
| `embedding_dimensions` | `usize?` | `None` | Embedding vector dimensions |
| `is_local` | `bool?` | `None` | Whether this is a local provider (affects routing) |
| `cost_per_input_token` | `f64?` | `None` | Cost per input token (USD) |
| `cost_per_output_token` | `f64?` | `None` | Cost per output token (USD) |
| `auth_header` | `String?` | `None` | Custom auth header name (e.g., `x-api-key`). Use `query:<param>` for query-string auth |
| `extra_headers` | `Map<String, String>?` | `None` | Additional HTTP headers to include |
| `tpm_limit` | `u64?` | `None` | Tokens-per-minute rate limit |
| `rpm_limit` | `u64?` | `None` | Requests-per-minute rate limit |
| `auth_mode` | `String?` | `None` | Authentication mode: `api_key` (default), `oauth` |
| `oauth_client_id` | `String?` | `None` | OAuth client ID for this provider |
| `oauth_redirect_uri` | `String?` | `None` | OAuth redirect URI |

---

## `[circuit_breaker]`

Automatic provider failure detection and recovery.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `threshold` | `u32` | `3` | Consecutive failures before tripping the breaker |
| `window_seconds` | `u64` | `60` | Failure counting window in seconds |
| `cooldown_seconds` | `u64` | `60` | Cooldown after a normal failure trip |
| `max_cooldown_seconds` | `u64` | `900` | Maximum cooldown duration (exponential backoff cap) |

---

## `[memory]`

Memory system configuration. Budget percentages **must sum to 100**.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `working_budget_pct` | `f64` | `30.0` | Percentage of memory budget for working memory |
| `episodic_budget_pct` | `f64` | `25.0` | Percentage for episodic memory (event logs) |
| `semantic_budget_pct` | `f64` | `20.0` | Percentage for semantic memory (facts, knowledge) |
| `procedural_budget_pct` | `f64` | `15.0` | Percentage for procedural memory (how-to) |
| `relationship_budget_pct` | `f64` | `10.0` | Percentage for relationship memory (social graph) |
| `embedding_provider` | `String?` | `None` | Provider for memory embeddings (falls back to primary) |
| `embedding_model` | `String?` | `None` | Model for memory embeddings |
| `hybrid_weight` | `f64` | `0.5` | Balance between keyword (0.0) and semantic (1.0) search |
| `ann_index` | `bool` | `false` | Enable approximate nearest-neighbor index for faster search |

---

## `[cache]`

Semantic response caching.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `true` | Enable the semantic cache |
| `exact_match_ttl_seconds` | `u64` | `3600` | Time-to-live for exact cache matches |
| `semantic_threshold` | `f64` | `0.95` | Cosine similarity threshold for semantic cache hits |
| `max_entries` | `usize` | `10000` | Maximum number of cache entries |
| `prompt_compression` | `bool` | `false` | Enable prompt compression for cache keys |
| `compression_target_ratio` | `f64` | `0.5` | Target compression ratio (0.0–1.0) |

---

## `[treasury]`

Financial controls and spending limits.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `per_payment_cap` | `f64` | `100.0` | Maximum single payment amount (USD). Must be positive |
| `hourly_transfer_limit` | `f64` | `500.0` | Maximum hourly transfer volume (USD) |
| `daily_transfer_limit` | `f64` | `2000.0` | Maximum daily transfer volume (USD) |
| `minimum_reserve` | `f64` | `5.0` | Minimum balance to maintain (USD). Must be non-negative |
| `daily_inference_budget` | `f64` | `50.0` | Daily budget for LLM inference costs (USD) |

---

## `[yield]`

DeFi yield management (Aave V3 integration).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable yield management |
| `protocol` | `String` | `"aave"` | Yield protocol |
| `chain` | `String` | `"base"` | Target chain |
| `min_deposit` | `f64` | `50.0` | Minimum deposit amount |
| `withdrawal_threshold` | `f64` | `30.0` | Balance threshold that triggers withdrawal |
| `chain_rpc_url` | `String?` | `None` | RPC URL for yield chain. Unset = mock behavior |
| `pool_address` | `String` | Base Sepolia Aave V3 Pool | Aave V3 Pool contract address |
| `usdc_address` | `String` | Base Sepolia USDC | Underlying asset (USDC) contract address |
| `atoken_address` | `String?` | `None` | aToken address for balance checks |

---

## `[wallet]`

On-chain wallet configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `path` | `PathBuf` | `~/.ironclad/wallet.json` | Encrypted wallet keystore file path |
| `chain_id` | `u64` | `8453` | EVM chain ID (8453 = Base mainnet) |
| `rpc_url` | `String` | `"https://mainnet.base.org"` | JSON-RPC endpoint URL |

---

## `[a2a]`

Agent-to-Agent protocol settings.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `true` | Enable A2A protocol |
| `max_message_size` | `usize` | `65536` | Maximum message size in bytes (64 KB) |
| `rate_limit_per_peer` | `u32` | `10` | Maximum requests per peer per window |
| `session_timeout_seconds` | `u64` | `3600` | Peer session timeout (1 hour) |
| `require_on_chain_identity` | `bool` | `true` | Require on-chain DID for peer authentication |

---

## `[skills]`

Skill execution engine.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `skills_dir` | `PathBuf` | `~/.ironclad/skills` | Directory containing skill definitions |
| `script_timeout_seconds` | `u64` | `30` | Maximum script execution time |
| `script_max_output_bytes` | `usize` | `1048576` | Maximum script output size (1 MB) |
| `allowed_interpreters` | `Vec<String>` | `["bash", "python3", "node"]` | Permitted script interpreters |
| `sandbox_env` | `bool` | `true` | Run scripts in a sandboxed environment |
| `hot_reload` | `bool` | `true` | Watch for skill file changes and reload automatically |

---

## `[security]`

Claim-based RBAC security policy. Controls how inbound messages are resolved to authority levels. See [architecture/security-rbac.md](architecture/security-rbac.md) for full documentation including data flows and sequence diagrams.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `deny_on_empty_allowlist` | `bool` | `true` | When `true`, channels with empty allow-lists reject all messages (secure default). `false` is no longer supported and is migrated by update/mechanic repair. |
| `allowlist_authority` | `InputAuthority` | `Peer` | Authority granted when sender passes a channel's allow-list. Options: `External`, `Peer`, `SelfGenerated`, `Creator` |
| `trusted_authority` | `InputAuthority` | `Creator` | Authority granted when sender is in `channels.trusted_sender_ids`. Options: `External`, `Peer`, `SelfGenerated`, `Creator` |
| `api_authority` | `InputAuthority` | `Creator` | Authority for HTTP API and WebSocket requests. Options: `External`, `Peer`, `SelfGenerated`, `Creator` |
| `threat_caution_ceiling` | `InputAuthority` | `External` | Maximum authority when the threat scanner flags input as Caution. Must be below `Creator`. Options: `External`, `Peer`, `SelfGenerated` |

**Validation rules:**
- `allowlist_authority` must be &le; `trusted_authority`
- `threat_caution_ceiling` must be &lt; `Creator`

**Authority resolution:** `effective = min(max(positive_grants...), min(negative_ceilings...))`

---

## `[channels]`

Communication channel configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `trusted_sender_ids` | `Vec<String>` | `[]` | Sender IDs with trusted authority (default: Creator). Matches against sender_id and chat_id. Empty = no Creator-level users |
| `thinking_threshold_seconds` | `u64` | `30` | Latency threshold before sending a thinking indicator |

### `[channels.telegram]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `true` | Enable Telegram adapter |
| `token_env` | `String` | `""` | Environment variable containing the bot token |
| `token_ref` | `String?` | `None` | Keystore secret name for the bot token |
| `allowed_chat_ids` | `Vec<i64>` | `[]` | Allowed Telegram chat IDs. Empty = allow all |
| `poll_timeout_seconds` | `u64` | `30` | Long-poll timeout for getUpdates |
| `webhook_mode` | `bool` | `false` | Use webhook mode instead of polling |
| `webhook_path` | `String?` | `None` | Custom webhook URL path |
| `webhook_secret` | `String?` | `None` | Secret token for webhook verification |

### `[channels.whatsapp]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable WhatsApp adapter |
| `token_env` | `String` | `""` | Environment variable for the access token |
| `token_ref` | `String?` | `None` | Keystore secret name for the access token |
| `phone_number_id` | `String` | `""` | WhatsApp Business phone number ID |
| `verify_token` | `String` | `""` | Webhook verification token |
| `allowed_numbers` | `Vec<String>` | `[]` | Allowed phone numbers. Empty = allow all |
| `app_secret` | `String?` | `None` | App secret for X-Hub-Signature-256 webhook verification |

### `[channels.discord]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `true` | Enable Discord adapter |
| `token_env` | `String` | `""` | Environment variable for the bot token |
| `token_ref` | `String?` | `None` | Keystore secret name for the bot token |
| `application_id` | `String` | `""` | Discord application ID |
| `allowed_guild_ids` | `Vec<String>` | `[]` | Allowed guild IDs. Empty = allow all |

### `[channels.signal]`

Uses signal-cli's JSON-RPC daemon as a local relay.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable Signal adapter |
| `phone_number` | `String` | `""` | Phone number registered with signal-cli (e.g., `+15551234567`) |
| `daemon_url` | `String` | `"http://127.0.0.1:8080"` | signal-cli JSON-RPC daemon URL |
| `allowed_numbers` | `Vec<String>` | `[]` | Allowed contacts. Empty = allow all |

### `[channels.email]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable email adapter |
| `imap_host` | `String` | `""` | IMAP server hostname |
| `imap_port` | `u16` | `993` | IMAP port (993 = TLS) |
| `smtp_host` | `String` | `""` | SMTP server hostname |
| `smtp_port` | `u16` | `587` | SMTP port (587 = STARTTLS) |
| `username` | `String` | `""` | Email account username |
| `password_env` | `String` | `""` | Environment variable for the email password |
| `from_address` | `String` | `""` | Sender address for outgoing emails |
| `allowed_senders` | `Vec<String>` | `[]` | Allowed sender addresses. Empty = allow all |
| `poll_interval_seconds` | `u64` | `30` | IMAP polling interval |

### `[channels.voice]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable voice channel |
| `stt_model` | `String?` | `None` | Speech-to-text model |
| `tts_model` | `String?` | `None` | Text-to-speech model |
| `tts_voice` | `String?` | `None` | TTS voice identifier |

---

## `[context]`

Context window management.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_tokens` | `usize` | `128000` | Maximum context window size in tokens |
| `soft_trim_ratio` | `f64` | `0.8` | Context usage ratio that triggers soft trimming (oldest turns removed) |
| `hard_clear_ratio` | `f64` | `0.95` | Context usage ratio that triggers hard clear (summary + reset) |
| `preserve_recent` | `usize` | `10` | Number of recent turns to preserve during trimming |
| `checkpoint_enabled` | `bool` | `false` | Enable periodic context checkpointing |
| `checkpoint_interval_turns` | `u32` | `10` | Number of turns between checkpoints |

---

## `[approvals]`

Human-in-the-loop approval gates for tool execution.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable the approval system |
| `gated_tools` | `Vec<String>` | `[]` | Tools requiring explicit approval before execution |
| `blocked_tools` | `Vec<String>` | `[]` | Tools that are always blocked |
| `timeout_seconds` | `u64` | `300` | Approval request timeout (5 minutes) |

---

## `[plugins]`

Plugin system configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `dir` | `PathBuf` | `~/.ironclad/plugins` | Plugin installation directory |
| `allow` | `Vec<String>` | `[]` | Plugin allowlist (empty = allow all not denied) |
| `deny` | `Vec<String>` | `[]` | Plugin denylist |

---

## `[browser]`

Headless browser automation (Chromium/CDP).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable browser automation |
| `executable_path` | `String?` | `None` | Path to Chromium/Chrome binary (auto-detected if unset) |
| `headless` | `bool` | `true` | Run browser in headless mode |
| `profile_dir` | `PathBuf` | `~/.ironclad/browser-profiles` | Browser profile data directory |
| `cdp_port` | `u16` | `9222` | Chrome DevTools Protocol port |

---

## `[daemon]`

Background daemon (launchd/systemd) configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auto_restart` | `bool` | `false` | Automatically restart on crash |
| `pid_file` | `PathBuf` | `~/.ironclad/ironclad.pid` | PID file path |

---

## `[update]`

Auto-update settings.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `check_on_start` | `bool` | `true` | Check for updates on server startup |
| `channel` | `String` | `"stable"` | Update channel: `stable`, `beta`, `dev` |
| `registry_url` | `String` | `"https://roboticus.ai/registry/manifest.json"` | Skills/providers registry manifest URL for catalog and content updates |

---

## `[tier_adapt]`

Tier-aware prompt adaptation. Adjusts prompts based on model tier capabilities.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `t1_strip_system` | `bool` | `false` | Strip system prompt for T1 (local) models |
| `t1_condense_turns` | `bool` | `false` | Condense conversation turns for T1 models |
| `t2_default_preamble` | `String?` | `"Be concise and direct. Focus on accuracy."` | Default system preamble for T2 models |
| `t3_t4_passthrough` | `bool` | `true` | Pass prompts through unmodified for T3/T4 models |

---

## `[personality]`

Personality file configuration. Files are loaded from the agent workspace directory.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `os_file` | `String` | `"OS.toml"` | Filename for the OS (identity/personality) definition |
| `firmware_file` | `String` | `"FIRMWARE.toml"` | Filename for the firmware (capabilities/rules) definition |

---

## `[session]`

Session management.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `ttl_seconds` | `u64` | `86400` | Session time-to-live (24 hours) |
| `scope_mode` | `String` | `"agent"` | Session scoping: `agent` (one per agent), `peer` (one per peer), `group` (shared) |
| `reset_schedule` | `String?` | `None` | Cron expression for periodic session reset (e.g., `"0 0 * * *"` or `"CRON_TZ=UTC+02:00 0 9 * * *"`) |

Notes:
- `scope_mode` is applied by web and channel message handlers when auto-creating sessions.
- `ttl_seconds` is enforced by the heartbeat `SessionGovernor` task.
- When `reset_schedule` is set, the governor evaluates the cron expression directly each heartbeat tick and rotates agent-scope sessions once per matching slot.

---

## `[digest]`

Conversation digest and summarization.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `true` | Enable conversation digests |
| `max_tokens` | `usize` | `512` | Maximum digest length in tokens |
| `decay_half_life_days` | `u32` | `7` | Importance decay half-life in days |

---

## `[multimodal]`

Multimodal (vision, audio) capabilities.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable multimodal processing |
| `media_dir` | `PathBuf?` | `None` | Directory for storing media files |
| `max_image_size_bytes` | `usize` | `10485760` | Maximum image size (10 MB) |
| `max_audio_size_bytes` | `usize` | `26214400` | Maximum audio size (25 MB) |
| `max_video_size_bytes` | `usize` | `52428800` | Maximum video size (50 MB) |
| `vision_model` | `String?` | `None` | Model to use for vision tasks |
| `transcription_model` | `String?` | `None` | Model to use for audio transcription |
| `auto_transcribe_audio` | `bool` | `false` | Automatically transcribe audio attachments |
| `auto_describe_images` | `bool` | `false` | Add image-attachment descriptions to prompt context |

Security note:
- Remote attachment downloads only allow `http`/`https` URLs and reject localhost/private-IP targets to reduce SSRF risk.

---

## `[knowledge]`

External knowledge sources.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `sources` | `Vec<KnowledgeSource>` | `[]` | List of knowledge source entries |

### Knowledge source entry

```toml
[[knowledge.sources]]
name = "docs"
source_type = "directory"
path = "~/Documents/reference"
max_chunks = 10
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | `String` | **required** | Display name for this knowledge source |
| `source_type` | `String` | **required** | Source type (e.g., `directory`, `url`) |
| `path` | `PathBuf?` | `None` | Local path (for directory sources) |
| `url` | `String?` | `None` | URL (for remote sources) |
| `max_chunks` | `usize` | `10` | Maximum chunks to retrieve per query |

---

## `[workspace_config]`

Workspace behavior settings.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `soul_versioning` | `bool` | `false` | Enable git-style versioning for personality files |
| `index_on_start` | `bool` | `false` | Index workspace files on server startup |
| `watch_for_changes` | `bool` | `false` | Watch workspace for file changes and re-index |

---

## `[mcp]`

Model Context Protocol server and client configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `server_enabled` | `bool` | `false` | Expose Ironclad as an MCP server |
| `server_port` | `u16` | `3001` | MCP server port |
| `clients` | `Vec<McpClient>` | `[]` | List of MCP servers to connect to |

### MCP client entry

```toml
[[mcp.clients]]
name = "github"
url = "http://localhost:4000"
transport = "Sse"
auth_token_env = "GITHUB_MCP_TOKEN"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | `String` | **required** | Client display name |
| `url` | `String` | **required** | MCP server URL |
| `transport` | `McpTransport` | `Sse` | Transport: `Sse`, `Stdio`, `Http`, `WebSocket` |
| `auth_token_env` | `String?` | `None` | Environment variable for auth token |

---

## `[devices]`

Multi-device sync and pairing.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable device management |
| `identity_path` | `PathBuf?` | `None` | Path to device identity file |
| `sync_enabled` | `bool` | `false` | Enable cross-device state sync |
| `max_paired_devices` | `usize` | `5` | Maximum number of paired devices |

---

## `[discovery]`

Network service discovery (mDNS/DNS-SD).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable service discovery |
| `dns_sd` | `bool` | `false` | Enable DNS-SD discovery |
| `mdns` | `bool` | `false` | Enable mDNS discovery |
| `advertise` | `bool` | `false` | Advertise this agent on the local network |
| `service_name` | `String` | `"_ironclad._tcp"` | DNS-SD service name |

---

## `[obsidian]`

Obsidian vault integration for knowledge management.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable Obsidian integration |
| `vault_path` | `PathBuf?` | `None` | Path to the Obsidian vault |
| `auto_detect` | `bool` | `false` | Auto-detect vault location |
| `auto_detect_paths` | `Vec<PathBuf>` | `[]` | Paths to search for vaults |
| `index_on_start` | `bool` | `true` | Index vault contents on startup |
| `watch_for_changes` | `bool` | `false` | Watch vault for file changes |
| `ignored_folders` | `Vec<String>` | `[".obsidian", ".trash", ".git"]` | Folders to exclude from indexing |
| `template_folder` | `String` | `"templates"` | Obsidian templates folder name |
| `default_folder` | `String` | `"ironclad"` | Default folder for agent-created notes |
| `preferred_destination` | `bool` | `true` | Prefer Obsidian as the destination for agent notes |
| `tag_boost` | `f64` | `0.2` | Relevance boost for tag matches during search |
