# Ironclad Security Audit Report

**Date:** 2026-02-21  
**Scope:** Full codebase audit — authentication, injection, crypto, data handling, network, financials.

---

## Findings by Severity

### CRITICAL

#### 1. HMAC trust boundary never used in production
- **File:line:** `crates/ironclad-agent/src/prompt.rs` (inject_hmac_boundary, verify_hmac_boundary); call sites
- **Description:** `inject_hmac_boundary` and `verify_hmac_boundary` exist and are tested in `ironclad-tests` and in `prompt.rs` unit tests, but **neither is called anywhere in production**. The agent builds the system prompt in `agent.rs` (e.g. `soul_text`) and sends it to the LLM without HMAC-tagging. User content is not verified against tampering via HMAC boundaries before being trusted.
- **Impact:** System prompt and user content cannot be cryptographically verified as unmodified. An attacker who can influence data in transit or at rest could inject or alter instructions without detection.
- **Recommendation:** Either (a) wire `build_system_prompt` + `inject_hmac_boundary` into the prompt path and `verify_hmac_boundary` on any content that is re-injected (e.g. from memory/cache), or (b) remove the dead code and document that HMAC boundaries are not in use so the design is explicit.

#### 2. Model output (L4) never scanned before reaching users
- **File:line:** `crates/ironclad-agent/src/injection.rs` (`scan_output`); `crates/ironclad-server/src/api/routes/agent.rs` (assistant content path)
- **Description:** `scan_output()` (L4) is implemented and tested but **never called in production**. Assistant content from the LLM is stored and returned to clients (e.g. `assistant_content` in `agent_message` and `process_channel_message`) without scanning for prompt-injection or exfil patterns.
- **Impact:** If the model is coerced into outputting instructions or sensitive patterns, that output is relayed to users and could be used in follow-up requests or downstream systems without filtering.
- **Recommendation:** Call `scan_output(assistant_content)` before persisting and before returning in all agent response paths (HTTP agent message, channel message handler). On positive result, either redact, replace with a safe message, or return a generic error and do not persist the raw output.

#### 3. Dashboard and /ws exempt from API key; dashboard embeds API key in HTML
- **File:line:** `crates/ironclad-server/src/auth.rs` (is_exempt: `/`, `/ws`); `crates/ironclad-server/src/dashboard.rs` (build_dashboard_html injects API_KEY)
- **Description:** Routes `/` and `/ws` are explicitly exempt from API key checks. The dashboard handler serves HTML that injects the server’s API key into the page (`var API_KEY = '...'`) so the SPA can call the API. No separate authentication protects the dashboard.
- **Impact:** Anyone who can reach the server (e.g. same network, or public bind) can load `/` and obtain the API key from the page source or dev tools, then call all protected API routes. WebSocket at `/ws` is also unauthenticated.
- **Recommendation:** Do not embed the API key in the dashboard HTML. Use one of: (a) require API key (or session) to access `/` (e.g. redirect to login or require `x-api-key`/Bearer for the dashboard HTML), (b) use a short-lived token or cookie after login for the SPA, or (c) serve the dashboard only on a separate port/bind with strict access control and do not inject the main API key. Remove `/ws` from exempt list if it should be protected, or document that it is intentionally public and limit what it can do.

#### 4. Telegram webhook accepted without auth when secret not set
- **File:line:** `crates/ironclad-server/src/api/routes/channels.rs` (webhook_telegram, ~15–34)
- **Description:** Telegram webhook handler only verifies `X-Telegram-Bot-Api-Secret-Token` when `adapter.webhook_secret` is `Some`. If the adapter exists but `webhook_secret` is `None`, the secret check is skipped and the body is processed.
- **Impact:** Any client can POST to `/api/webhooks/telegram` and inject fake “Telegram” updates (e.g. fake messages, chat IDs), causing the agent to process arbitrary content and potentially send replies to attacker-controlled chats.
- **Recommendation:** Require webhook secret when Telegram is enabled: reject (401) with a clear error if `webhook_secret` is `None` and the adapter is configured, or disable webhook processing until a secret is set.

#### 5. WhatsApp webhook accepted without auth when app_secret not set
- **File:line:** `crates/ironclad-server/src/api/routes/channels.rs` (webhook_whatsapp, ~95–121)
- **Description:** WhatsApp signature verification (`X-Hub-Signature-256`) is only performed when `adapter.app_secret` is `Some`. If the adapter is configured but `app_secret` is `None`, the webhook body is processed without verification.
- **Impact:** Same as Telegram: fake webhook payloads can be sent to `/api/webhooks/whatsapp`, leading to unauthorized message processing and possible reply abuse.
- **Recommendation:** Require `app_secret` when WhatsApp webhook is enabled; return 401 if secret is missing and do not process the webhook.

---

### HIGH

#### 6. PolicyEngine only applied to plugin tool HTTP API, not other tool execution
- **File:line:** `crates/ironclad-server/src/api/routes/admin.rs` (execute_plugin_tool, ~303–346); `crates/ironclad-agent/src/tools.rs` (Tool::execute); agent loop / channel flow
- **Description:** `PolicyEngine::evaluate_all` is only called in `execute_plugin_tool` (POST `/api/plugins/{name}/execute/{tool}`). Built-in tools (e.g. EchoTool) and any future agent-loop tool execution do not go through the policy engine.
- **Impact:** Plugin tools are gated by authority, command safety, etc., but other tool paths are not. If the agent or another path invokes tools without policy checks, high-risk or forbidden operations could be allowed.
- **Recommendation:** Ensure every tool execution path (plugin HTTP, agent loop, scripts, etc.) runs through the same PolicyEngine (or a documented subset) before calling `execute`. Centralize tool dispatch so policy cannot be bypassed.

#### 7. FTS5 query injection in hybrid_search (embeddings)
- **File:line:** `crates/ironclad-db/src/embeddings.rs` (~125–131); `crates/ironclad-db/src/memory.rs` (sanitize_fts_query used in fts_search)
- **Description:** `memory::fts_search` sanitizes the query with `sanitize_fts_query` (phrase wrap, alphanumeric/whitespace, quote escape). `embeddings::hybrid_search` passes `query_text` directly into `memory_fts MATCH ?1` without sanitization.
- **Impact:** If `hybrid_search` is ever called with user-controlled or attacker-controlled `query_text`, FTS5 operators (e.g. AND, OR, NOT, quoted phrases) can be injected, potentially causing errors, bypassing intended search, or affecting availability. Currently `hybrid_search` does not appear to be called from other crates; risk is latent.
- **Recommendation:** Use the same FTS sanitization in `hybrid_search` (e.g. call `memory::sanitize_fts_query` or move it to a shared helper and use it in both `memory` and `embeddings`). If `hybrid_search` is not used, add a comment or remove it to avoid future misuse.

#### 8. Wallet private key stored unencrypted on disk
- **File:line:** `crates/ironclad-wallet/src/wallet.rs` (WalletFile with private_key_hex, load_or_generate)
- **Description:** Private key is stored in a JSON file as hex. On Unix, file permissions are set to `0o600` after creation; there is no encryption at rest.
- **Impact:** Anyone with filesystem access (or backup access) can read the key and control the wallet. Malware or compromised host can exfiltrate keys.
- **Recommendation:** Document that keys are stored in plaintext and recommend filesystem encryption (e.g. encrypted volume) and strict file permissions. Optionally add optional encryption-at-rest (e.g. passphrase-derived key) for high-security deployments.

---

### MEDIUM

#### 9. WebSocket endpoint unauthenticated and exempt from API key
- **File:line:** `crates/ironclad-server/src/auth.rs` (is_exempt includes `/ws`); `crates/ironclad-server/src/ws.rs`
- **Description:** `/ws` is exempt from API key middleware. The WebSocket handler echoes client messages back and forwards EventBus events; it does not appear to drive agent actions or expose secrets directly.
- **Impact:** Unauthenticated clients can connect and send messages; current logic only echoes. If future code uses WebSocket input for agent commands or sensitive operations, that would be unprotected.
- **Recommendation:** If WebSocket will carry sensitive or command data, require authentication (e.g. API key in query or first message) and remove `/ws` from exempt list. If it is intentionally public (e.g. read-only status), document that and keep the handler minimal.

#### 10. No global API rate limiting
- **File:line:** `crates/ironclad-server/src/lib.rs` (router layers); A2A has per-peer rate limit in `ironclad-channels/src/a2a.rs`
- **Description:** A2A has per-peer rate limiting. There is no application-level rate limiting on general API routes (e.g. `/api/agent/message`, `/api/sessions`, etc.).
- **Impact:** DoS or abuse via high request rate on expensive endpoints (e.g. LLM calls) or session/memory endpoints.
- **Recommendation:** Add a rate-limiting layer (e.g. per-IP or per-API-key) for sensitive routes, with configurable limits and possibly stricter limits for agent and plugin endpoints.

#### 11. CORS allows all origins when API key is not set
- **File:line:** `crates/ironclad-server/src/lib.rs` (~253–268)
- **Description:** When `config.server.api_key.is_none()`, CORS is configured with `allow_origin(Any)`. When API key is set, CORS is restricted to a single origin derived from `bind:port`.
- **Impact:** Without API key, any website can make cross-origin requests to the server, increasing CSRF and abuse surface. With API key, origin is limited to one (which may still be too broad depending on deployment).
- **Recommendation:** Make CORS configurable (e.g. list of allowed origins in config). When API key is not set, consider not allowing arbitrary origins or document that the server is intended for development/local use only.

#### 12. Money arithmetic unchecked; possible overflow
- **File:line:** `crates/ironclad-wallet/src/money.rs` (Add, Sub for Money(i64))
- **Description:** `Money` uses `i64` (cents). `Add` and `Sub` use `self.0 + rhs.0` and `self.0 - rhs.0` with no overflow checks. Treasury policy checks use `Money::from_dollars` and comparisons; negative amounts are rejected at policy level.
- **Impact:** Extreme values could overflow (e.g. large positive + large positive). In practice, treasury caps and validation may bound inputs; unchecked arithmetic remains a risk for bugs or future code paths.
- **Recommendation:** Use `checked_add`/`checked_sub` (or `saturating_*`) and handle or reject overflow. Consider a small type (e.g. `Money` constructors that validate range) to keep all money in a safe range.

#### 13. Request body size limits not uniform
- **File:line:** WhatsApp webhook uses `WEBHOOK_BODY_LIMIT: 1MB` in `channels.rs`; other JSON routes use Axum default (e.g. 2MB) or unspecified.
- **Description:** Only the WhatsApp webhook explicitly enforces a body limit. Other endpoints (e.g. agent message, plugin execute, config update) rely on framework defaults.
- **Impact:** Very large payloads could cause memory pressure or DoS. Default limits may be acceptable but are not documented or consistently set.
- **Recommendation:** Document or set explicit body size limits for all JSON/body-accepting routes (e.g. agent message, plugin execute, config, A2A) and consider stricter limits for sensitive endpoints.

---

### LOW

#### 14. verify_hmac_boundary and inject_hmac_boundary only in tests
- **File:line:** `crates/ironclad-agent/src/prompt.rs`; `crates/ironclad-tests/src/injection_defense.rs`
- **Description:** HMAC boundary helpers are only referenced in tests, not in production prompt or verification paths.
- **Impact:** Same as CRITICAL #1 (dead code / missing control); listed again at LOW as a “missing use” reminder if CRITICAL #1 is addressed by removing the feature.
- **Recommendation:** Resolve consistently with CRITICAL #1 (either wire into production or remove and document).

#### 15. Script runner path and interpreter whitelist
- **File:line:** `crates/ironclad-agent/src/script_runner.rs` (execute, check_interpreter)
- **Description:** Script path and args come from the caller; interpreter is validated against a whitelist and extension/shebang. No explicit path canonicalization or “escape from allowed dir” check was found in the caller chain.
- **Impact:** If any caller ever passes a user-controlled path (e.g. from API or message), path traversal (e.g. `../../../etc/passwd`) could lead to execution of unintended scripts. Current usage appears to be from fixed skill paths.
- **Recommendation:** Ensure all callers of `ScriptRunner::execute` use paths derived from config or trusted sources only. If any path can be user-influenced, canonicalize and restrict to an allowed directory.

#### 16. Discord has no webhook route
- **File:line:** `crates/ironclad-server/src/api/routes/mod.rs` (routes); `crates/ironclad-channels/src/discord.rs`
- **Description:** Only `/api/webhooks/telegram` and `/api/webhooks/whatsapp` exist. Discord adapter exists but there is no `/api/webhooks/discord`. Discord appears outbound-only (bot token).
- **Impact:** No finding; if Discord webhooks are added later, they should require equivalent authentication (e.g. signature or secret) and injection checks.
- **Recommendation:** When/if adding Discord webhooks, enforce verification (e.g. signature) and apply the same injection and auth patterns as Telegram/WhatsApp.

---

### INFO

#### 17. L1 injection check applied on HTTP and channel message content
- **File:line:** `crates/ironclad-server/src/api/routes/agent.rs` (body.content); `crates/ironclad-server/src/api/routes/agent.rs` (inbound.content in process_channel_message)
- **Description:** `check_injection` is called for `/api/agent/message` and for channel messages (Telegram/WhatsApp) before processing. Blocked messages are rejected or answered with a safe message.
- **Impact:** Positive: user and channel input is L1-checked on these paths.

#### 18. Memory search API uses sanitized FTS
- **File:line:** `crates/ironclad-server/src/api/routes/memory.rs` (memory_search); `crates/ironclad-db/src/memory.rs` (fts_search, sanitize_fts_query)
- **Description:** `/api/memory/search?q=...` uses `memory::fts_search`, which applies `sanitize_fts_query` before FTS5 MATCH.
- **Impact:** Positive: memory search is protected against FTS5 operator injection.

#### 19. SQL uses parameterized queries
- **File:line:** `crates/ironclad-db` (sessions, memory, cron, skills, metrics, etc.)
- **Description:** Queries use `?1`, `?2`, and `rusqlite::params![]` (or equivalent). No raw string interpolation of user input into SQL was found.
- **Impact:** Positive: SQL injection risk is mitigated.

#### 20. Treasury policy validates negative and zero amounts
- **File:line:** `crates/ironclad-wallet/src/treasury.rs` (check_per_payment, check_hourly_limit, check_daily_limit, check_minimum_reserve)
- **Description:** All checked flows require positive amounts (`amt <= Money::zero()` returns an error). Tests cover negative and zero rejection.
- **Impact:** Positive: financial entry points covered by treasury checks reject non-positive amounts.

#### 21. A2A crypto: X25519 + HKDF + AES-256-GCM
- **File:line:** `crates/ironclad-channels/src/a2a.rs`
- **Description:** Key agreement uses X25519 ephemeral keys, HKDF-SHA256 for session key derivation, and AES-256-GCM for encryption. Nonce is generated with `OsRng`. Message size and timestamp drift are validated.
- **Impact:** Positive: A2A crypto design is sound.

#### 22. WhatsApp webhook verifies X-Hub-Signature-256 when secret is set
- **File:line:** `crates/ironclad-server/src/api/routes/channels.rs` (webhook_whatsapp)
- **Description:** When `app_secret` is set, HMAC-SHA256 of the raw body is computed and compared to the header. Body is read with a size limit before parsing.
- **Impact:** Positive: when configured, WhatsApp webhook authentication is correct.

#### 23. Config API strips api_key before returning
- **File:line:** `crates/ironclad-server/src/api/routes/admin.rs` (~55)
- **Description:** When returning config to the client, `api_key` is removed from the payload.
- **Impact:** Positive: API key is not leaked via config endpoint.

---

## Security Strengths

- **Injection (L1):** `check_injection` is applied to HTTP agent message and channel (Telegram/WhatsApp) message content; blocked messages are rejected or answered safely.
- **FTS (memory):** `/api/memory/search` uses `fts_search` with `sanitize_fts_query`; no raw user input in FTS5 MATCH.
- **SQL:** Consistent use of parameterized queries in `ironclad-db`; no string interpolation into SQL.
- **Treasury:** Negative/zero amount validation at policy level; multiple limits (per-payment, hourly, daily, reserve, inference budget).
- **A2A:** Strong crypto (X25519, HKDF, AES-256-GCM); message size and timestamp checks; per-peer rate limiting.
- **WhatsApp auth:** When `app_secret` is set, webhook signature verification is correctly implemented.
- **Wallet:** Unix file permissions `0o600` on wallet file; key generation with `OsRng`.
- **Config:** API key removed from config in API responses.

---

## Overall Security Grade

**C+ (Good foundations, critical gaps)**

- Strong points: parameterized SQL, FTS sanitization for memory search, L1 injection on main input paths, treasury amount checks, A2A crypto, WhatsApp HMAC when configured.
- Critical gaps: HMAC boundary and L4 output scanning not used in production; dashboard and WebSocket exempt from auth with API key embedded in HTML; Telegram/WhatsApp webhooks accepted without auth when secrets are not set; PolicyEngine only on plugin tool HTTP path.
- Addressing the five CRITICAL and the HIGH findings (policy coverage, FTS in hybrid_search, wallet key handling) would materially improve the security posture and justify a higher grade.

---

## Essential Files for Security Context

| Topic | Files |
|------|--------|
| Auth / API key / exemptions | `crates/ironclad-server/src/auth.rs`, `crates/ironclad-server/src/lib.rs` |
| Webhooks | `crates/ironclad-server/src/api/routes/channels.rs`, `crates/ironclad-channels/src/telegram.rs`, `crates/ironclad-channels/src/whatsapp.rs` |
| Injection L1/L4 | `crates/ironclad-agent/src/injection.rs`, `crates/ironclad-server/src/api/routes/agent.rs` |
| HMAC boundary | `crates/ironclad-agent/src/prompt.rs` |
| Policy engine | `crates/ironclad-agent/src/policy.rs`, `crates/ironclad-server/src/api/routes/admin.rs` (execute_plugin_tool) |
| FTS / memory | `crates/ironclad-db/src/memory.rs` (fts_search, sanitize_fts_query), `crates/ironclad-db/src/embeddings.rs` (hybrid_search) |
| Crypto A2A | `crates/ironclad-channels/src/a2a.rs` |
| Wallet / money | `crates/ironclad-wallet/src/wallet.rs`, `crates/ironclad-wallet/src/money.rs`, `crates/ironclad-wallet/src/treasury.rs` |
| Dashboard | `crates/ironclad-server/src/dashboard.rs` |
