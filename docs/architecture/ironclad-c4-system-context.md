# C4 Level 1: System Context -- Ironclad Platform

*Generated 2026-02-20. Describes the Ironclad autonomous agent runtime and its external dependencies.*

---

## System Context Diagram

```mermaid
C4Context
    title Ironclad System Context

    Person(creator, "Creator", "Human operator who interacts<br/>via chat channels and dashboard")

    System(ironclad, "Ironclad Platform", "Single-binary Rust autonomous agent runtime:<br/>unified SQLite DB, ML model routing,<br/>semantic cache, 5-tier memory, zero-trust A2A,<br/>multi-layer injection defense, yield engine")

    System_Ext(anthropic, "Anthropic Claude", "T3 LLM -- api.anthropic.com")
    System_Ext(google, "Google Gemini", "T2 LLM -- generativelanguage.googleapis.com")
    System_Ext(moonshot, "Moonshot Kimi", "T2 LLM -- api.moonshot.ai")
    System_Ext(openaiCodex, "OpenAI Codex", "Primary LLM -- api.openai.com")
    System(ollamaLocal, "Ollama (Mac)", "T1 self-hosted -- 127.0.0.1:11434")
    System(ollamaGpu, "Ollama (Windows GPU)", "T1 self-hosted -- 192.168.50.253:11434")

    System_Ext(telegram, "Telegram", "Chat channel via Bot API")
    System_Ext(whatsapp, "WhatsApp", "Chat channel via Cloud API")

    System_Ext(baseChain, "Ethereum Base L2", "USDC transfers, ERC-8004 agent registry,<br/>EIP-3009 signed authorizations")

    System_Ext(defi, "Aave / Compound", "DeFi yield protocols on Base<br/>for idle USDC treasury growth")

    System_Ext(peerAgents, "Peer Agents", "Other Ironclad or A2A-compatible agents<br/>discovered via ERC-8004 registry,<br/>authenticated via challenge-response")

    System_Ext(creditsApi, "Credits API", "HTTP endpoint for credit purchases<br/>via x402 payment protocol")

    Rel(creator, ironclad, "Telegram / WhatsApp / WebSocket / Dashboard")
    Rel(ironclad, anthropic, "HTTPS (Messages API)")
    Rel(ironclad, google, "HTTPS (Generative AI API)")
    Rel(ironclad, moonshot, "HTTPS (Chat API)")
    Rel(ironclad, openaiCodex, "HTTPS (Completions + Responses API)")
    Rel(ironclad, ollamaLocal, "HTTP LAN (Ollama API)")
    Rel(ironclad, ollamaGpu, "HTTP LAN (Ollama API)")
    Rel(ironclad, telegram, "HTTPS (Bot API polling/webhook)")
    Rel(ironclad, whatsapp, "HTTPS (Cloud API webhook)")
    Rel(ironclad, baseChain, "JSON-RPC via alloy-rs")
    Rel(ironclad, defi, "On-chain contract calls via alloy-rs")
    Rel(ironclad, peerAgents, "HTTPS (A2A protocol, AES-256-GCM encrypted)")
    Rel(ironclad, creditsApi, "HTTPS + x402 payment headers")
```

## External Systems Summary

| System | Protocol | Purpose | Auth |
|--------|----------|---------|------|
| Anthropic Claude | HTTPS | T3 LLM inference | API key (env var) |
| Google Gemini | HTTPS | T2 LLM inference | API key (env var) |
| Moonshot Kimi | HTTPS | T2 LLM inference | API key (env var) |
| OpenAI Codex | HTTPS | Primary LLM inference | API key (env var) |
| Ollama (Mac) | HTTP | T1 local inference | None (localhost) |
| Ollama (GPU) | HTTP | T1 local inference | None (LAN) |
| Telegram | HTTPS | User chat channel | Bot token (env var) |
| WhatsApp | HTTPS | User chat channel | Cloud API token (env var) |
| Ethereum Base | JSON-RPC | USDC, ERC-8004, yield | Wallet private key (file) |
| Aave / Compound | On-chain | Yield on idle USDC | Wallet private key (file) |
| Peer Agents | HTTPS | A2A task delegation | ECDH session keys + ERC-8004 identity |
| Credits API | HTTPS | x402 credit purchases | EIP-3009 signed authorization |

## Key Boundaries

- **Single process boundary**: Ironclad is one OS process. All internal communication is in-process function calls -- no IPC, no serialization boundaries.
- **Network boundary**: All external systems are accessed over HTTP/HTTPS or JSON-RPC. The only local-network connections are to Ollama instances.
- **Trust boundary**: Creator messages have full authority. Peer agent messages are wrapped in trust-tagged boundaries and processed with reduced authority. All external input passes through the 4-layer injection defense pipeline.
- **Financial boundary**: On-chain operations (USDC transfers, yield deposits/withdrawals) are guarded by the treasury policy engine with per-payment, hourly, and daily limits.
