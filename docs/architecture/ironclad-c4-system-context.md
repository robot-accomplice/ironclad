# C4 Level 1: System Context — Ironclad Platform

*Describes the Ironclad autonomous agent runtime and its external boundaries. Aligned with actual codebase (lib.rs, main.rs).*

---

## System Context Diagram

```mermaid
C4Context
    title Ironclad System Context

    Person(creator, "Creator", "Human operator who interacts<br/>via chat channels and dashboard")

    System(ironclad, "Ironclad Platform", "Single-binary Rust autonomous agent runtime:<br/>unified SQLite DB, heuristic model routing,<br/>in-memory semantic cache, 5-tier memory + FTS,<br/>zero-trust A2A, multi-layer injection defense,<br/>policy engine (6 rules), wallet & treasury")

    System_Ext(anthropic, "Anthropic Claude", "LLM — api.anthropic.com")
    System_Ext(openai, "OpenAI", "LLM — api.openai.com")
    System_Ext(ollama, "Ollama", "Local LLM — HTTP (e.g. 127.0.0.1:11434)")
    System_Ext(google, "Google Generative AI (Gemini)", "LLM — generativelanguage.googleapis.com")
    System_Ext(openrouter, "OpenRouter", "LLM aggregator — openrouter.ai/api")
    System_Ext(otherLlms, "Other LLM Providers", "DeepSeek, Groq, Moonshot, SGLang, vLLM,<br/>Docker Model Runner, llama-cpp — configurable")

    System_Ext(telegram, "Telegram", "Chat channel via Bot API")
    System_Ext(whatsapp, "WhatsApp", "Chat channel via Cloud API")
    System_Ext(discord, "Discord", "Chat channel via Gateway + REST API")
    System_Ext(signal, "Signal", "Chat channel via signal-cli daemon")
    System_Ext(email, "Email", "Chat channel via IMAP + SMTP")
    System_Ext(voice, "Voice", "Chat channel via STT/TTS (Whisper + OpenAI Audio)")
    System_Ext(web, "Web / HTTP", "Dashboard, REST API, WebSocket")
    System_Ext(chromeCdp, "Chrome / Chromium", "Browser automation via CDP")

    System_Ext(baseChain, "Base Sepolia / Base", "Ethereum L2: USDC, Aave V3 (yield),<br/>wallet interaction via alloy-rs")

    System_Ext(peerAgents, "Peer Agents", "Other Ironclad or A2A-compatible agents")

    Rel(creator, ironclad, "Telegram / WhatsApp / Discord / Signal /<br/>Email / Voice / WebSocket / HTTP API / Dashboard")
    Rel(ironclad, anthropic, "HTTPS (Messages API)")
    Rel(ironclad, openai, "HTTPS (Completions API)")
    Rel(ironclad, ollama, "HTTP (Ollama API)")
    Rel(ironclad, google, "HTTPS (Generative Language API)")
    Rel(ironclad, openrouter, "HTTPS (OpenRouter API)")
    Rel(ironclad, otherLlms, "HTTPS (configurable providers)")
    Rel(ironclad, telegram, "HTTPS (Bot API polling/webhook)")
    Rel(ironclad, whatsapp, "HTTPS (Cloud API webhook)")
    Rel(ironclad, discord, "WSS (Gateway) + HTTPS (REST API)")
    Rel(ironclad, signal, "JSON-RPC (Unix socket to signal-cli)")
    Rel(ironclad, email, "IMAP (poll) + SMTP (send)")
    Rel(ironclad, voice, "WebRTC / HTTPS (STT/TTS)")
    Rel(ironclad, web, "HTTP (axum server)")
    Rel(ironclad, chromeCdp, "CDP WebSocket (localhost)")
    Rel(ironclad, baseChain, "JSON-RPC via alloy-rs (wallet, Aave V3)")
    Rel(ironclad, peerAgents, "HTTPS (A2A protocol)")
```

## External Systems Summary

| System | Protocol | Purpose | Auth |
|--------|----------|---------|------|
| Anthropic Claude | HTTPS | LLM inference | API key (env) |
| OpenAI | HTTPS | LLM inference | API key (env) |
| Ollama | HTTP | Local LLM inference | None (localhost/LAN) |
| Google Gemini | HTTPS | LLM inference | API key (env) |
| OpenRouter | HTTPS | LLM aggregator | API key (env) |
| Other LLM Providers | HTTPS | LLM inference (DeepSeek, Groq, Moonshot, SGLang, vLLM, Docker Model Runner, llama-cpp) | API key (env) or none (local) |
| Telegram | HTTPS | User chat channel | Bot token (env) |
| WhatsApp | HTTPS | User chat channel | Cloud API token (env) |
| Discord | WSS + HTTPS | User chat channel | Bot token (env) |
| Signal | JSON-RPC | User chat channel | signal-cli daemon |
| Email | IMAP + SMTP | User chat channel | IMAP/SMTP credentials (env) |
| Voice | WebRTC / HTTPS | User chat channel (STT/TTS) | API key (env) |
| Web / Dashboard | HTTP | REST API, WebSocket, UI | Optional API key |
| Chrome / Chromium | CDP WebSocket | Browser automation | None (localhost) |
| Base (Sepolia/Mainnet) | JSON-RPC | Wallet, USDC, Aave V3 yield | Wallet key (file) |
| Peer Agents | HTTPS | A2A task delegation | A2A identity / challenge-response |

## Key Boundaries

- **Single process**: Ironclad is one OS process. All internal communication is in-process (no IPC).
- **Network boundary**: External systems are reached over HTTP/HTTPS or JSON-RPC.
- **Trust boundary**: Creator input has full authority; peer/external input is constrained by the policy engine (AuthorityRule, CommandSafetyRule, etc.) and 4-layer injection defense.
- **Financial boundary**: On-chain operations (USDC, yield) are guarded by treasury policy and wallet service (ironclad-wallet).

## References

- Entry and bootstrap: `crates/ironclad-server/src/main.rs`, `crates/ironclad-server/src/lib.rs`
- Channels: `ironclad-channels` (Telegram, WhatsApp, Discord, Signal, Email, Voice, WebSocket, A2A)
- Browser: `ironclad-browser` (Chrome/Chromium via CDP)
- Wallet / Base: `ironclad-wallet` (alloy-rs, Aave V3 on Base Sepolia)
