# ironclad-channels

> **Version 0.5.0**

Channel adapters for user-facing chat platforms and the zero-trust agent-to-agent (A2A) communication protocol for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime.

## Supported Channels

| Channel | Module | Status |
|---------|--------|--------|
| Telegram | `telegram` | Full (Bot API, long-poll + webhook, Markdown V2) |
| WhatsApp | `whatsapp` | Full (Cloud API, webhook, templates) |
| Discord | `discord` | Full (Gateway, slash commands, rich embeds) |
| Signal | `signal` | Full (signal-cli daemon, JSON-RPC) |
| WebSocket | `web` | Full (axum ws, JSON frames, heartbeat) |
| Voice | `voice` | WebRTC + STT/TTS pipeline |
| Email | `email` | IMAP listener + SMTP sender |
| A2A | `a2a` | Zero-trust protocol (ECDH, AES-256-GCM) |

## Key Types & Traits

| Type | Module | Description |
|------|--------|-------------|
| `ChannelAdapter` | `lib` | Trait: `recv()`, `send()`, `platform_name()` |
| `InboundMessage` | `lib` | Normalized inbound message from any platform |
| `OutboundMessage` | `lib` | Normalized outbound message |
| `ChannelRouter` | `router` | Multi-channel message routing and dispatch |
| `A2aProtocol` | `a2a` | ECDH handshake, AES-256-GCM encryption, DID verification |

## Additional Modules

- `delivery` -- Outbound delivery queue with retry logic
- `filter` -- Addressability filter (per-channel routing rules, keyword triggers, mention-only)

## Usage

```toml
[dependencies]
ironclad-channels = "0.5"
```

```rust
use ironclad_channels::{ChannelAdapter, InboundMessage, OutboundMessage};

// All channel adapters implement the ChannelAdapter trait
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-channels).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
