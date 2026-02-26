# Bug Ledger — v0.8.0 Stabilization

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

## Summary

| Severity | Open | In Progress | Fixed | Verified |
|----------|------|-------------|-------|----------|
| Critical | 0    | 0           | 0     | 0        |
| High     | 0    | 0           | 0     | 0        |
| Medium   | 7    | 0           | 0     | 0        |
| Low      | 3    | 0           | 0     | 0        |

## Entries

| ID | Source | Crate | Tier | Severity | Category | Description | Location | Status |
|----|--------|-------|------|----------|----------|-------------|----------|--------|
| BUG-001 | audit-c4 | docs | -- | Medium | doc drift | Discord channel adapter exists in code (`ironclad-channels/src/discord.rs`) but has no `System_Ext` node in C4 System Context diagram | `docs/architecture/ironclad-c4-system-context.md` line 10-42 | Open |
| BUG-002 | audit-c4 | docs | -- | Medium | doc drift | Signal channel adapter exists in code (`ironclad-channels/src/signal.rs`) but has no `System_Ext` node in C4 System Context diagram | `docs/architecture/ironclad-c4-system-context.md` line 10-42 | Open |
| BUG-003 | audit-c4 | docs | -- | Medium | doc drift | Email channel adapter (IMAP/SMTP) exists in code (`ironclad-channels/src/email.rs`) but has no `System_Ext` node in C4 System Context diagram | `docs/architecture/ironclad-c4-system-context.md` line 10-42 | Open |
| BUG-004 | audit-c4 | docs | -- | Medium | doc drift | Voice channel (STT/TTS via Whisper + OpenAI Audio) exists in code (`ironclad-channels/src/voice.rs`) but has no `System_Ext` node in C4 System Context diagram | `docs/architecture/ironclad-c4-system-context.md` line 10-42 | Open |
| BUG-005 | audit-c4 | docs | -- | Medium | doc drift | Chrome/Chromium browser automation via CDP exists as full crate (`ironclad-browser`) but has no `System_Ext` node in C4 System Context diagram | `docs/architecture/ironclad-c4-system-context.md` line 10-42 | Open |
| BUG-006 | audit-c4 | docs | -- | Medium | doc drift | Google Generative AI (Gemini) has first-class `ApiFormat`, bundled provider config, and dedicated format translation but is lumped into vague "Other LLM Providers" node instead of having its own explicit `System_Ext` | `docs/architecture/ironclad-c4-system-context.md` line 21 | Open |
| BUG-007 | audit-c4 | docs | -- | Medium | doc drift | OpenRouter aggregator is a bundled provider in `bundled_providers.toml` but has no representation in C4 System Context diagram | `docs/architecture/ironclad-c4-system-context.md` line 10-42 | Open |
| BUG-008 | audit-c4 | docs | -- | Low | doc drift | Groq has explicit `System_Ext` node in diagram but is NOT a bundled provider (absent from `bundled_providers.toml`); should be demoted to "Other LLM Providers" or removed | `docs/architecture/ironclad-c4-system-context.md` line 20 | Open |
| BUG-009 | audit-c4 | docs | -- | Low | doc drift | Creator relationship label (line 31) lists only "Telegram / WhatsApp / WebSocket / HTTP API / Dashboard" but omits Discord, Signal, Email, and Voice channels | `docs/architecture/ironclad-c4-system-context.md` line 31 | Open |
| BUG-010 | audit-c4 | docs | -- | Low | doc drift | "Other LLM Providers" label says "Google, Moonshot, etc." but v0.8.0 bundles 11 providers including SGLang, vLLM, Docker Model Runner, llama-cpp, and OpenRouter | `docs/architecture/ironclad-c4-system-context.md` line 21 | Open |
