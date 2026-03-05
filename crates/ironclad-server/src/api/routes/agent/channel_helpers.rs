//! Channel resolution helpers, typing/thinking indicators, and latency estimation.

use super::AppState;

pub(super) fn metadata_str(meta: Option<&serde_json::Value>, ptr: &str) -> Option<String> {
    meta.and_then(|m| m.pointer(ptr)).and_then(|v| {
        v.as_str()
            .map(|s| s.to_string())
            .or_else(|| v.as_i64().map(|n| n.to_string()))
            .or_else(|| v.as_u64().map(|n| n.to_string()))
    })
}

pub(super) fn resolve_channel_chat_id(inbound: &ironclad_channels::InboundMessage) -> String {
    let meta = inbound.metadata.as_ref();
    metadata_str(meta, "/chat_id")
        .or_else(|| metadata_str(meta, "/channel_id"))
        .or_else(|| metadata_str(meta, "/thread_id"))
        .or_else(|| metadata_str(meta, "/conversation_id"))
        .or_else(|| metadata_str(meta, "/group_id"))
        .or_else(|| metadata_str(meta, "/message/chat/id"))
        .or_else(|| metadata_str(meta, "/messages/0/chat/id"))
        .or_else(|| metadata_str(meta, "/messages/0/channel_id"))
        .unwrap_or_else(|| inbound.sender_id.clone())
}

pub(crate) fn channel_chat_id_for_inbound(inbound: &ironclad_channels::InboundMessage) -> String {
    resolve_channel_chat_id(inbound)
}

pub(super) fn resolve_channel_is_group(inbound: &ironclad_channels::InboundMessage) -> bool {
    let meta = inbound.metadata.as_ref();
    if let Some(v) = meta
        .and_then(|m| m.get("is_group"))
        .and_then(|v| v.as_bool())
    {
        return v;
    }
    if let Some(kind) = metadata_str(meta, "/message/chat/type") {
        return matches!(kind.as_str(), "group" | "supergroup");
    }
    false
}

pub(super) fn resolve_channel_scope(
    cfg: &ironclad_core::IroncladConfig,
    inbound: &ironclad_channels::InboundMessage,
    chat_id: &str,
) -> ironclad_db::sessions::SessionScope {
    let mode = cfg.session.scope_mode.as_str();
    let channel = inbound.platform.to_lowercase();
    if mode == "group" && resolve_channel_is_group(inbound) {
        return ironclad_db::sessions::SessionScope::Group {
            group_id: chat_id.to_string(),
            channel,
        };
    }
    if mode == "peer" || mode == "group" {
        return ironclad_db::sessions::SessionScope::Peer {
            peer_id: inbound.sender_id.clone(),
            channel,
        };
    }
    ironclad_db::sessions::SessionScope::Agent
}

pub(super) fn parse_skills_json(skills_json: Option<&str>) -> Vec<String> {
    skills_json
        .and_then(|s| {
            serde_json::from_str::<Vec<String>>(s)
                .inspect_err(|e| tracing::warn!(error = %e, "failed to parse skills JSON"))
                .ok()
        })
        .unwrap_or_default()
}

/// Send a "typing..." indicator on the appropriate chat channel.
/// Best-effort — failures are silently ignored so they never block processing.
pub(super) async fn send_typing_indicator(
    state: &AppState,
    platform: &str,
    chat_id: &str,
    metadata: Option<&serde_json::Value>,
) {
    match platform {
        "telegram" => {
            if let Some(ref tg) = state.telegram {
                tg.send_typing(chat_id).await;
            }
        }
        "whatsapp" => {
            if let Some(ref wa) = state.whatsapp {
                let msg_id = metadata
                    .and_then(|m| m.pointer("/messages/0/id"))
                    .or_else(|| metadata.and_then(|m| m.get("id")))
                    .and_then(|v| v.as_str());
                wa.send_typing(chat_id, msg_id).await;
            }
        }
        "discord" => {
            if let Some(ref dc) = state.discord {
                dc.send_typing(chat_id).await;
            }
        }
        "signal" => {
            if let Some(ref sig) = state.signal {
                sig.send_typing(chat_id).await;
            }
        }
        _ => {}
    }
}

/// Send a thinking indicator on the appropriate chat channel.
/// Used when estimated latency exceeds the configured threshold.
pub(super) async fn send_thinking_indicator(
    state: &AppState,
    platform: &str,
    chat_id: &str,
    metadata: Option<&serde_json::Value>,
) {
    let thinking_text = build_personality_thinking_text(state).await;
    send_typing_indicator(state, platform, chat_id, metadata).await;

    match platform {
        "telegram" => {
            if let Some(ref tg) = state.telegram
                && tg.send_ephemeral(chat_id, &thinking_text).await.is_none()
            {
                tracing::debug!(platform, chat_id, "thinking indicator send failed");
            }
        }
        "whatsapp" => {
            if let Some(ref wa) = state.whatsapp
                && wa.send_ephemeral(chat_id, &thinking_text).await.is_none()
            {
                tracing::debug!(platform, chat_id, "thinking indicator send failed");
            }
        }
        "discord" => {
            if let Some(ref dc) = state.discord
                && dc.send_ephemeral(chat_id, &thinking_text).await.is_none()
            {
                tracing::debug!(platform, chat_id, "thinking indicator send failed");
            }
        }
        "signal" => {
            if let Some(ref sig) = state.signal
                && sig.send_ephemeral(chat_id, &thinking_text).await.is_none()
            {
                tracing::debug!(platform, chat_id, "thinking indicator send failed");
            }
        }
        _ => {}
    }
}

pub(super) async fn build_personality_ack_text(state: &AppState) -> String {
    let cfg = state.config.read().await;
    format!("{} here. On it.", cfg.agent.name)
}

async fn build_personality_thinking_text(state: &AppState) -> String {
    let cfg = state.config.read().await;
    format!("⚔️ {} is working the task…", cfg.agent.name)
}

/// Estimate expected inference latency in seconds based on model tier, input
/// length, and whether the primary provider's circuit breaker is tripped (which
/// means we're falling back to slower alternatives).
pub(super) async fn estimate_inference_latency(
    tier: ironclad_core::ModelTier,
    input_len: usize,
    model: &str,
    primary_model: &str,
    state: &AppState,
) -> u64 {
    use ironclad_core::ModelTier;

    let base: u64 = match tier {
        ModelTier::T1 => 5,
        ModelTier::T2 => 8,
        ModelTier::T3 => 20,
        ModelTier::T4 => 40,
    };

    // Longer inputs take longer to process
    let length_penalty: u64 = match input_len {
        0..=500 => 0,
        501..=2000 => 5,
        2001..=5000 => 15,
        _ => 25,
    };

    // If the primary model's breaker is open, we're falling through the chain
    // which adds latency from failed connection attempts + slower fallbacks
    let primary_prefix = primary_model.split('/').next().unwrap_or("unknown");
    let fallback_penalty: u64 = {
        let llm = state.llm.read().await;
        if model != primary_model && llm.breakers.is_blocked(primary_prefix) {
            15
        } else if model != primary_model {
            5
        } else {
            0
        }
    };

    base + length_penalty + fallback_penalty
}
