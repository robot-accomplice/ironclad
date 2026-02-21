# C4 Level 3: Component Diagram -- ironclad-channels

*Channel adapters for user-facing chat platforms and the zero-trust agent-to-agent (A2A) communication protocol.*

---

## Component Diagram

```mermaid
flowchart TB
    subgraph IroncladChannels ["ironclad-channels"]
        TELEGRAM["telegram.rs<br/>Telegram Bot API"]
        WHATSAPP["whatsapp.rs<br/>WhatsApp Cloud API"]
        WEB["web.rs<br/>WebSocket Interface"]
        A2A["a2a.rs<br/>Agent-to-Agent Protocol"]
    end

    subgraph TelegramDetail ["telegram.rs"]
        TG_POLL["Long-poll getUpdates<br/>or webhook receiver"]
        TG_PARSE["parse_inbound():<br/>extract text, media refs,<br/>chat_id, user info"]
        TG_FORMAT["format_outbound():<br/>Markdown V2 formatting,<br/>message chunking (4096 char limit),<br/>inline keyboard for actions"]
        TG_SEND["send_message():<br/>POST to Bot API"]
    end

    subgraph WhatsAppDetail ["whatsapp.rs"]
        WA_WEBHOOK["Webhook receiver<br/>(verify token, parse payload)"]
        WA_PARSE["parse_inbound():<br/>extract text, media,<br/>phone number, profile"]
        WA_FORMAT["format_outbound():<br/>WhatsApp message templates,<br/>text formatting"]
        WA_SEND["send_message():<br/>POST to Cloud API"]
    end

    subgraph WebDetail ["web.rs"]
        WS_UPGRADE["WebSocket upgrade handler<br/>(axum ws::WebSocketUpgrade)"]
        WS_RECV["recv_message():<br/>parse JSON frame into<br/>InboundMessage"]
        WS_SEND["send_message():<br/>serialize response<br/>to JSON frame"]
        WS_HEARTBEAT["Ping/pong keepalive"]
    end

    subgraph A2ADetail ["a2a.rs - Zero-Trust Protocol"]
        direction TB
        DISCOVERY["Agent Discovery:<br/>query ERC-8004 registry<br/>on Base, cache agent cards<br/>in discovered_agents table"]
        HELLO["Handshake: POST /a2a/hello<br/>DID + nonce (32 bytes) +<br/>timestamp + signature"]
        VERIFY["Mutual Authentication:<br/>verify signature against<br/>on-chain public key<br/>(ERC-8004 registry lookup)"]
        SESSION_KEY["Session Key Derivation:<br/>ECDH (ephemeral keypairs)<br/>-> AES-256-GCM session key<br/>for forward secrecy"]
        ENCRYPT["Message Encryption:<br/>AES-256-GCM per-message<br/>nonce + HMAC auth tag"]
        VALIDATE["Message Validation:<br/>- timestamp freshness (< 60s)<br/>- size < a2a.max_message_size<br/>- rate limit < a2a.rate_limit_per_peer<br/>- injection defense screening"]
        TRUST_TAG["Trust Tagging:<br/>wrap in peer_agent_input<br/>trust_level=X from<br/>relationship_memory"]
    end

    subgraph SharedTrait ["Shared Channel Trait"]
        CHANNEL_TRAIT["trait ChannelAdapter:<br/>async fn recv() -> InboundMessage<br/>async fn send(OutboundMessage)<br/>fn platform_name() -> &str"]
        INBOUND["InboundMessage:<br/>source, text, media,<br/>platform_metadata"]
        OUTBOUND["OutboundMessage:<br/>text, attachments,<br/>reply_to, format_hints"]
    end

    TELEGRAM & WHATSAPP & WEB & A2A -.-> CHANNEL_TRAIT
```

## A2A Handshake Sequence

```mermaid
sequenceDiagram
    participant AgentA as Agent A (initiator)
    participant Registry as ERC-8004 Registry
    participant AgentB as Agent B (responder)
    participant RelMem as relationship_memory

    AgentA->>Registry: lookup Agent B by capability
    Registry-->>AgentA: Agent Card (endpoint, DID, capabilities)
    AgentA->>AgentA: cache in discovered_agents

    AgentA->>AgentB: POST /a2a/hello (DID_A, nonce_A, timestamp, sig_A)
    AgentB->>Registry: verify DID_A -> public key
    Registry-->>AgentB: public key for A
    AgentB->>AgentB: verify sig_A, check timestamp < 60s
    AgentB-->>AgentA: response (DID_B, nonce_B, timestamp, sig_B)
    AgentA->>Registry: verify DID_B -> public key
    Registry-->>AgentA: public key for B
    AgentA->>AgentA: verify sig_B, check timestamp < 60s

    Note over AgentA,AgentB: ECDH key exchange (ephemeral keypairs)
    AgentA->>AgentA: derive session_key = ECDH(ephemeral_A, ephemeral_B)
    AgentB->>AgentB: derive session_key = ECDH(ephemeral_B, ephemeral_A)

    Note over AgentA,AgentB: All subsequent messages encrypted with AES-256-GCM

    AgentA->>AgentB: encrypted task message
    AgentB->>AgentB: decrypt, validate, injection screen
    AgentB->>RelMem: update trust_score for A
    AgentB-->>AgentA: encrypted response
```

## Dependencies

**External crates**: `reqwest` (HTTP for Telegram/WhatsApp APIs), `tokio-tungstenite` (WebSocket), `alloy-rs` (ERC-8004 registry queries), `aes-gcm` (A2A encryption), `x25519-dalek` or `alloy` (ECDH)

**Internal crates**: `ironclad-core` (types, config)

**Depended on by**: `ironclad-server`
