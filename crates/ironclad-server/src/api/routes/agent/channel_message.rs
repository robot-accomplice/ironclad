//! Channel message processing for Telegram, Discord, Signal, Email, and webhooks.

use super::AppState;
use super::channel_helpers::{
    build_personality_ack_text, estimate_inference_latency, resolve_channel_chat_id,
    resolve_channel_scope, send_thinking_indicator, send_typing_indicator,
};
use super::core;
use super::decomposition::{
    DecompositionDecision, DecompositionOutcome, DelegationProvenance,
    apply_decomposition_decision, build_gate_system_note, evaluate_decomposition_gate,
    maybe_handle_specialist_creation_controls,
};
use super::intents::{
    requests_cron, requests_current_events, requests_delegation, requests_execution,
    requests_file_distribution, requests_introspection,
};
use super::strip_internal_delegation_metadata;
use ironclad_core::InputAuthority;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn strip_numeric_bracket_citations(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '[' {
            let mut j = i + 1;
            let mut has_digit = false;
            while j < chars.len() && chars[j].is_ascii_digit() {
                has_digit = true;
                j += 1;
            }
            if has_digit && j < chars.len() && chars[j] == ']' {
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn normalize_telegram_text(content: &str) -> String {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        let mut normalized = if in_fence {
            line.to_string()
        } else {
            line.trim_start_matches('#').trim_start().to_string()
        };
        normalized = normalized
            .replace("**", "")
            .replace("__", "")
            .replace('`', "");
        normalized = strip_numeric_bracket_citations(&normalized);
        out.push(normalized);
    }
    out.join("\n").trim().to_string()
}

fn is_short_followup_for_previous_reply(user_content: &str) -> bool {
    let lower = user_content.trim().to_ascii_lowercase();
    if lower.len() > 80 {
        return false;
    }
    let markers = [
        "what's that from",
        "what is that from",
        "where is that from",
        "no, your quote",
        "your quote",
        "what quote",
        "source?",
    ];
    markers.iter().any(|m| lower.contains(m))
}

fn is_short_reactive_sarcasm(user_content: &str) -> bool {
    let lower = user_content.trim().to_ascii_lowercase();
    if lower.len() > 32 {
        return false;
    }
    let markers = [
        "wow",
        "great",
        "fantastic",
        "amazing",
        "incredible",
        "brilliant",
        "sure",
        "right",
    ];
    markers
        .iter()
        .any(|m| lower == *m || lower == format!("{m}.") || lower == format!("{m}..."))
}

fn is_short_contradiction_followup(user_content: &str) -> bool {
    let lower = user_content.trim().to_ascii_lowercase();
    if lower.len() > 48 {
        return false;
    }
    let markers = [
        "that's not true",
        "that is not true",
        "not true",
        "that's wrong",
        "that is wrong",
        "incorrect",
    ];
    markers
        .iter()
        .any(|m| lower == *m || lower == format!("{m}.") || lower.contains(m))
}

async fn contextualize_short_followup(
    state: &AppState,
    session_id: &str,
    user_content: &str,
) -> String {
    if !is_short_followup_for_previous_reply(user_content)
        && !is_short_reactive_sarcasm(user_content)
        && !is_short_contradiction_followup(user_content)
    {
        return user_content.to_string();
    }
    let Ok(history) = ironclad_db::sessions::list_messages(&state.db, session_id, Some(20)) else {
        return user_content.to_string();
    };
    let previous_assistant = history
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.trim())
        .filter(|s| !s.is_empty());
    let Some(previous_assistant) = previous_assistant else {
        return user_content.to_string();
    };
    if is_short_reactive_sarcasm(user_content) {
        return format!(
            "User likely reacted with sarcasm/frustration to your previous reply. Acknowledge the miss directly, do not treat it as praise, and correct course.\nPrevious assistant reply excerpt:\n\"{}\"\n\nUser reaction:\n{}",
            previous_assistant.chars().take(240).collect::<String>(),
            user_content
        );
    }
    if is_short_contradiction_followup(user_content) {
        return format!(
            "User directly disputed your previous reply as incorrect. Acknowledge the error and provide a corrected answer grounded in available tools/delegation.\nPrevious assistant reply excerpt:\n\"{}\"\n\nUser follow-up:\n{}",
            previous_assistant.chars().take(240).collect::<String>(),
            user_content
        );
    }
    let quote = previous_assistant.chars().take(360).collect::<String>();
    format!(
        "User follow-up references your immediately previous reply. Answer specifically what that prior reply/quote is from.\nPrevious assistant reply excerpt:\n\"{}\"\n\nUser question:\n{}",
        quote, user_content
    )
}

pub(super) fn format_channel_reply_for_delivery(platform: &str, content: &str) -> String {
    let cleaned = strip_internal_delegation_metadata(content);
    if platform.eq_ignore_ascii_case("telegram") {
        return normalize_telegram_text(&cleaned);
    }
    cleaned
}

pub async fn process_channel_message(
    state: &AppState,
    inbound: ironclad_channels::InboundMessage,
) -> Result<(), String> {
    tracing::info!(channel = %inbound.platform, peer = %inbound.sender_id, "Processing channel message");
    let chat_id = resolve_channel_chat_id(&inbound);
    let platform = inbound.platform.clone();

    // ── Multimodal enrichment ───────────────────────────────────────────
    // Extract structured attachments from metadata, download media, and
    // enrich content with transcription / vision descriptions.
    let mut inbound = inbound;
    if state.media_service.is_some() {
        enrich_multimodal(state, &mut inbound).await;
    }

    if inbound.content.trim().is_empty() {
        return Ok(());
    }
    if inbound.content.len() > 32_768 {
        state
            .channel_router
            .send_reply(
                &platform,
                &chat_id,
                "Message is too long (max 32768 bytes). Please shorten and try again.".into(),
            )
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "failed to send oversize message reply"))
            .ok();
        return Ok(());
    }

    // Addressability filter: in group chats, only respond when explicitly addressed
    {
        let config = state.config.read().await;
        let agent_name = &config.agent.name;
        let chain = ironclad_channels::filter::default_addressability_chain(agent_name);
        if !chain.accepts(&inbound) {
            tracing::debug!(chat_id = %chat_id, "addressability filter: not addressed, skipping");
            return Ok(());
        }
    }

    if inbound.content.starts_with('/')
        && let Some(reply) =
            super::handle_bot_command(state, &inbound.content, Some(&inbound)).await
    {
        let reply = format_channel_reply_for_delivery(&platform, &reply);
        state
            .channel_router
            .send_reply(&platform, &chat_id, reply)
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "failed to send bot command reply"))
            .ok();
        return Ok(());
    }

    // Injection defense: block (>0.7), sanitize (0.3-0.7), or pass (<0.3)
    let threat = ironclad_agent::injection::check_injection(&inbound.content);
    if threat.is_blocked() {
        state
            .channel_router
            .send_reply(
                &platform,
                &chat_id,
                "I can't process that message — it was flagged by my safety filters.".into(),
            )
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "failed to send injection block reply"))
            .ok();
        return Ok(());
    }
    let mut user_content = if threat.is_caution() {
        tracing::info!(score = threat.value(), platform = %platform, "Sanitizing caution-level channel input");
        ironclad_agent::injection::sanitize(&inbound.content)
    } else {
        inbound.content.clone()
    };

    // Show "typing..." indicator while processing (all chat channels)
    send_typing_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;

    // In-flight deduplication for channel messages
    let dedup_scope = format!("{}:{}", platform, chat_id);
    let dedup_fp = ironclad_llm::DedupTracker::fingerprint(
        &dedup_scope,
        &[ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: user_content.clone(),
            parts: None,
        }],
    );
    {
        let mut llm = state.llm.write().await;
        if !llm.dedup.check_and_track(&dedup_fp) {
            tracing::debug!("dropping duplicate channel message");
            return Ok(());
        }
    }

    let config = state.config.read().await;
    let agent_id = config.agent.id.clone();
    let scope = resolve_channel_scope(&config, &inbound, &chat_id);
    drop(config);
    let session_id = match ironclad_db::sessions::find_or_create(&state.db, &agent_id, Some(&scope))
    {
        Ok(id) => id,
        Err(e) => {
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err(e.to_string());
        }
    };
    if let Err(e) =
        ironclad_db::sessions::append_message(&state.db, &session_id, "user", &inbound.content)
    {
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err(e.to_string());
    }
    user_content = contextualize_short_followup(state, &session_id, &user_content).await;
    if let Some(reply) =
        maybe_handle_specialist_creation_controls(state, &session_id, &user_content).await
    {
        let reply = format_channel_reply_for_delivery(&platform, &reply);
        state
            .channel_router
            .send_reply(&platform, &chat_id, reply)
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "failed to send specialist control reply"))
            .ok();
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Ok(());
    }

    let channel_turn_id = uuid::Uuid::new_v4().to_string();

    // Pre-create the turn record so that any tool calls executed during
    // delegation (before the main inference pipeline) can reference it
    // without violating the tool_calls.turn_id FK constraint.
    if let Err(e) = ironclad_db::sessions::create_turn_with_id(
        &state.db,
        &channel_turn_id,
        &session_id,
        None,
        None,
        None,
        None,
    ) {
        tracing::warn!(error = %e, "failed to pre-create turn record for channel message");
    }

    let features = ironclad_llm::extract_features(&user_content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let gate_decision = evaluate_decomposition_gate(state, &user_content, complexity).await;
    let outcome = apply_decomposition_decision(state, &gate_decision, &session_id, "channel").await;
    let delegation_workflow_note = match outcome {
        DecompositionOutcome::SpecialistProposalPending { prompt } => {
            state
                .channel_router
                .send_reply(&platform, &chat_id, prompt)
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "failed to send specialist proposal"))
                .ok();
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Ok(());
        }
        DecompositionOutcome::Centralized => None,
        DecompositionOutcome::Delegated { workflow_note } => Some(workflow_note),
    };
    let gate_system_note =
        build_gate_system_note(&gate_decision, delegation_workflow_note.as_deref());

    // ── Concrete delegation execution (before inference) ──────────
    let config = state.config.read().await;
    let agent_name = config.agent.name.clone();
    let agent_id = config.agent.id.clone();
    let primary_model = config.models.primary.clone();
    let tier_adapt = config.tier_adapt.clone();
    let thinking_threshold = config.channels.thinking_threshold_seconds;
    let trusted = config.channels.trusted_sender_ids.clone();
    let security_config = config.security.clone();

    // Derive allow-list membership from per-channel config for claim resolution.
    let (sender_in_allowlist, allowlist_configured) = match platform.as_str() {
        "telegram" => {
            if let Some(ref tg) = config.channels.telegram {
                let in_list = tg
                    .allowed_chat_ids
                    .iter()
                    .any(|id| id.to_string() == chat_id);
                (
                    !tg.allowed_chat_ids.is_empty() && in_list,
                    !tg.allowed_chat_ids.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "whatsapp" => {
            if let Some(ref wa) = config.channels.whatsapp {
                let in_list = wa.allowed_numbers.iter().any(|n| n == &inbound.sender_id);
                (
                    !wa.allowed_numbers.is_empty() && in_list,
                    !wa.allowed_numbers.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "discord" => {
            if let Some(ref dc) = config.channels.discord {
                let in_list = dc.allowed_guild_ids.iter().any(|g| g == &chat_id);
                (
                    !dc.allowed_guild_ids.is_empty() && in_list,
                    !dc.allowed_guild_ids.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "signal" => {
            if let Some(ref sig) = config.channels.signal {
                let in_list = sig.allowed_numbers.iter().any(|n| n == &inbound.sender_id);
                (
                    !sig.allowed_numbers.is_empty() && in_list,
                    !sig.allowed_numbers.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "email" => {
            let in_list = config
                .channels
                .email
                .allowed_senders
                .iter()
                .any(|s| s.eq_ignore_ascii_case(&inbound.sender_id));
            (
                !config.channels.email.allowed_senders.is_empty() && in_list,
                !config.channels.email.allowed_senders.is_empty(),
            )
        }
        _ => (false, false),
    };
    drop(config);
    let personality = state.personality.read().await;
    let soul_text = personality.soul_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);

    // Resolve authority via the unified claim-based RBAC system.
    let security_claim = ironclad_core::security::resolve_channel_claim(
        &ironclad_core::security::ChannelContext {
            sender_id: &inbound.sender_id,
            chat_id: &chat_id,
            channel: &platform,
            sender_in_allowlist,
            allowlist_configured,
            threat_is_caution: threat.is_caution(),
            trusted_sender_ids: &trusted,
        },
        &security_config,
    );
    let channel_authority = security_claim.authority;
    if security_claim.threat_downgraded {
        tracing::info!(
            sender = %inbound.sender_id,
            channel = %platform,
            effective_authority = ?channel_authority,
            sources = ?security_claim.sources,
            "Threat-score ceiling applied to channel message"
        );
    }

    let mut precomputed_delegation_provenance = DelegationProvenance::default();
    let delegated_execution_note = if let DecompositionDecision::Delegated(plan) = &gate_decision {
        let delegated_params = serde_json::json!({
            "task": user_content,
            "subtasks": plan.subtasks,
        });
        match super::execute_tool_call(
            state,
            "orchestrate-subagents",
            &delegated_params,
            &channel_turn_id,
            channel_authority,
            Some(&platform),
        )
        .await
        {
            Ok(output) => {
                let output = strip_internal_delegation_metadata(&output);
                precomputed_delegation_provenance.subagent_task_started = true;
                precomputed_delegation_provenance.subagent_task_completed = true;
                precomputed_delegation_provenance.subagent_result_attached =
                    !output.trim().is_empty();
                Some(format!(
                    "Delegated subagent execution completed this turn. Verified output:\n{}",
                    output
                ))
            }
            Err(err) => {
                precomputed_delegation_provenance.subagent_task_started = true;
                Some(format!(
                    "Delegation was attempted this turn but failed: {err}"
                ))
            }
        }
    } else {
        None
    };

    if channel_authority == InputAuthority::Creator
        && let Some(skill_reply) = try_skill_first_fulfillment(
            state,
            &user_content,
            &channel_turn_id,
            channel_authority,
            &platform,
        )
        .await
    {
        let skill_reply = format_channel_reply_for_delivery(&platform, &skill_reply);
        state
            .channel_router
            .send_reply(&platform, &chat_id, skill_reply)
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "failed to send skill-first reply"))
            .ok();
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Ok(());
    }

    // ── Prepare inference via core ───────────────────────────────────
    let input = core::InferenceInput {
        state,
        session_id: &session_id,
        user_content: &user_content,
        turn_id: &channel_turn_id,
        channel_label: &platform,
        agent_name,
        agent_id: agent_id.clone(),
        soul_text,
        firmware_text,
        primary_model: primary_model.clone(),
        tier_adapt,
        delegation_workflow_note,
        inject_diagnostics: false,
        gate_system_note: Some(gate_system_note),
        delegated_execution_note,
    };

    let mut prepared = match core::prepare_inference(&input).await {
        Ok(p) => p,
        Err(msg) => {
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err(msg);
        }
    };

    // Model switch for complex delegated tasks
    let mut model_switch_notice: Option<String> = None;
    if matches!(gate_decision, DecompositionDecision::Delegated(_))
        && complexity > 0.8
        && prepared.model != primary_model
    {
        model_switch_notice = Some(format!(
            "Model suitability update: switching delegated execution from `{}` to `{}` for this task.",
            prepared.model, primary_model
        ));
        let new_model_for_api = primary_model
            .split_once('/')
            .map(|(_, m)| m)
            .unwrap_or(&primary_model)
            .to_string();
        prepared.model = primary_model.clone();
        prepared.model_for_api = new_model_for_api.clone();
        prepared.request.model = new_model_for_api;
    }

    // ── Thinking/typing indicator ────────────────────────────────────
    let thinking_sent = Arc::new(AtomicBool::new(false));
    let typing_keepalive_stop = Arc::new(AtomicBool::new(false));
    {
        // Start visual feedback immediately for every inbound message so the
        // user sees responsive behavior even when we avoid textual pre-acks.
        send_typing_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;
        let keepalive_state = state.clone();
        let keepalive_platform = platform.clone();
        let keepalive_chat_id = chat_id.clone();
        let keepalive_metadata = inbound.metadata.clone();
        let keepalive_stop = Arc::clone(&typing_keepalive_stop);
        tokio::spawn(async move {
            // Keep signaling liveness for long-running turns until completion.
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if keepalive_stop.load(Ordering::Acquire) {
                    break;
                }
                send_typing_indicator(
                    &keepalive_state,
                    &keepalive_platform,
                    &keepalive_chat_id,
                    keepalive_metadata.as_ref(),
                )
                .await;
            }
        });

        let ack_text = build_personality_ack_text(state).await;
        if let Some(notice) = model_switch_notice.as_ref() {
            state
                .channel_router
                .send_reply(&platform, &chat_id, notice.clone())
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "failed to send model switch notice"))
                .ok();
        }
        let should_pre_ack = platform == "telegram"
            && (requests_execution(&user_content)
                || requests_current_events(&user_content)
                || requests_delegation(&user_content)
                || requests_introspection(&user_content)
                || requests_file_distribution(&user_content)
                || requests_cron(&user_content));
        if should_pre_ack {
            state
                .channel_router
                .send_reply(&platform, &chat_id, ack_text.clone())
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "failed to send pre-acknowledgment"))
                .ok();
            send_thinking_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;
            thinking_sent.store(true, Ordering::Release);
        }

        let estimated_latency = estimate_inference_latency(
            prepared.tier,
            user_content.len(),
            &prepared.model,
            &primary_model,
            state,
        )
        .await;

        if !should_pre_ack && estimated_latency >= thinking_threshold {
            send_thinking_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;
            thinking_sent.store(true, Ordering::Release);
        } else if !should_pre_ack {
            let delayed_state = state.clone();
            let delayed_platform = platform.clone();
            let delayed_chat_id = chat_id.clone();
            let delayed_metadata = inbound.metadata.clone();
            let delayed_guard = Arc::clone(&thinking_sent);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(thinking_threshold)).await;
                if delayed_guard.load(Ordering::Acquire) {
                    return;
                }
                send_thinking_indicator(
                    &delayed_state,
                    &delayed_platform,
                    &delayed_chat_id,
                    delayed_metadata.as_ref(),
                )
                .await;
                delayed_guard.store(true, Ordering::Release);
            });
        }
    }

    // ── Unified inference pipeline (cache → inference → post-turn) ──
    let mut delegation_provenance = precomputed_delegation_provenance;
    let result = match core::execute_inference_pipeline(
        state,
        &prepared,
        &session_id,
        &user_content,
        &channel_turn_id,
        channel_authority,
        Some(&platform),
        &mut delegation_provenance,
    )
    .await
    {
        Ok(r) => r,
        Err(msg) => {
            typing_keepalive_stop.store(true, Ordering::Release);
            thinking_sent.store(true, Ordering::Release);
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err(msg);
        }
    };

    // Send reply to channel
    let outbound = format_channel_reply_for_delivery(&platform, &result.content);
    if let Err(e) = state
        .channel_router
        .send_reply(&platform, &chat_id, outbound)
        .await
    {
        typing_keepalive_stop.store(true, Ordering::Release);
        thinking_sent.store(true, Ordering::Release);
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err(e.to_string());
    }
    typing_keepalive_stop.store(true, Ordering::Release);
    thinking_sent.store(true, Ordering::Release);

    // Release dedup tracking
    {
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
    }

    Ok(())
}

fn user_keyword_tokens(input: &str) -> std::collections::HashSet<String> {
    input
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s.len() >= 3)
        .collect()
}

async fn try_skill_first_fulfillment(
    state: &AppState,
    user_content: &str,
    turn_id: &str,
    authority: InputAuthority,
    channel: &str,
) -> Option<String> {
    let skills = match ironclad_db::skills::list_skills(&state.db) {
        Ok(rows) => rows.into_iter().filter(|s| s.enabled).collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(error = %e, "skill-first lookup failed");
            return None;
        }
    };
    if skills.is_empty() {
        return None;
    }

    let tokens = user_keyword_tokens(user_content);
    if tokens.is_empty() {
        return None;
    }

    let mut best: Option<(usize, String, String)> = None;
    for skill in skills {
        let Some(script_path) = skill.script_path.clone() else {
            continue;
        };
        let Some(triggers_raw) = skill.triggers_json.as_deref() else {
            continue;
        };
        let Ok(triggers) = serde_json::from_str::<ironclad_core::SkillTrigger>(triggers_raw) else {
            continue;
        };
        let score = triggers
            .keywords
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .filter(|k| tokens.contains(k))
            .count();
        if score == 0 {
            continue;
        }
        match best {
            Some((best_score, _, _)) if best_score >= score => {}
            _ => best = Some((score, skill.name.clone(), script_path)),
        }
    }

    let (_score, skill_name, script_path) = best?;

    let params = serde_json::json!({
        "path": script_path,
        "args": [user_content],
    });
    match super::execute_tool_call(
        state,
        "run_script",
        &params,
        turn_id,
        authority,
        Some(channel),
    )
    .await
    {
        Ok(output) => {
            tracing::info!(skill = %skill_name, "skill-first execution succeeded");
            Some(output)
        }
        Err(e) => {
            tracing::warn!(skill = %skill_name, error = %e, "skill-first execution failed; falling back to LLM pipeline");
            None
        }
    }
}

// ── Multimodal enrichment ───────────────────────────────────────────────

/// Map a MIME content-type to the voice pipeline's `AudioFormat`.
fn audio_format_from_content_type(ct: &str) -> ironclad_channels::voice::AudioFormat {
    let ct_lower = ct.to_ascii_lowercase();
    if ct_lower.contains("ogg") || ct_lower.contains("opus") {
        ironclad_channels::voice::AudioFormat::Ogg
    } else if ct_lower.contains("mp3") || ct_lower.contains("mpeg") {
        ironclad_channels::voice::AudioFormat::Mp3
    } else if ct_lower.contains("wav") {
        ironclad_channels::voice::AudioFormat::Wav
    } else if ct_lower.contains("pcm") || ct_lower.contains("raw") {
        ironclad_channels::voice::AudioFormat::Pcm
    } else {
        // Default to Ogg — WhatsApp voice notes use audio/ogg; codecs=opus
        ironclad_channels::voice::AudioFormat::Ogg
    }
}

/// Extract `MediaAttachment` entries from inbound metadata, download media
/// via [`ironclad_channels::media::MediaService`], and prepend
/// transcription/vision descriptions to
/// `inbound.content`. Runs inline (not spawned) so content is enriched
/// before the message reaches the LLM pipeline.
async fn enrich_multimodal(state: &AppState, inbound: &mut ironclad_channels::InboundMessage) {
    let media_svc = match &state.media_service {
        Some(svc) => svc,
        None => return,
    };

    // Parse attachments from metadata
    let attachments: Vec<ironclad_channels::MediaAttachment> = inbound
        .metadata
        .as_ref()
        .and_then(|m| m.get("attachments"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    if attachments.is_empty() {
        return;
    }

    let config = state.config.read().await;
    let auto_transcribe = config.multimodal.auto_transcribe_audio;
    let auto_describe = config.multimodal.auto_describe_images;
    drop(config);

    let mut enrichments: Vec<String> = Vec::new();

    for att in &attachments {
        // Download media if source URL is available
        let local_path = if let Some(ref url) = att.source_url {
            if url.starts_with("http://") || url.starts_with("https://") {
                match media_svc
                    .download_and_store(url, &att.media_type, att.filename.as_deref())
                    .await
                {
                    Ok(path) => Some(path),
                    Err(e) => {
                        tracing::warn!(
                            url = %url,
                            error = %e,
                            "failed to download media attachment"
                        );
                        None
                    }
                }
            } else if url.starts_with("whatsapp://media/") {
                // WhatsApp media requires a two-step download (resolve media ID → URL)
                let media_id = url.trim_start_matches("whatsapp://media/");
                if let Some(ref wa) = state.whatsapp {
                    match media_svc
                        .download_whatsapp_media(
                            media_id,
                            &wa.token,
                            &att.media_type,
                            att.filename.as_deref(),
                        )
                        .await
                    {
                        Ok(path) => Some(path),
                        Err(e) => {
                            tracing::warn!(
                                media_id = %media_id,
                                error = %e,
                                "failed to download WhatsApp media"
                            );
                            None
                        }
                    }
                } else {
                    tracing::debug!("WhatsApp adapter not configured, cannot download media");
                    None
                }
            } else {
                att.local_path.clone()
            }
        } else {
            att.local_path.clone()
        };

        // Auto-transcribe audio attachments
        if auto_transcribe
            && att.media_type == ironclad_channels::MediaType::Audio
            && let Some(ref path) = local_path
            && let Some(ref voice_lock) = state.voice
        {
            // Read audio bytes from downloaded file
            match tokio::fs::read(path).await {
                Ok(audio_data) => {
                    // Infer audio format from content-type
                    let format = audio_format_from_content_type(&att.content_type);
                    let mut voice = voice_lock.write().await;
                    match voice.transcribe(&audio_data, format).await {
                        Ok(result) if !result.text.trim().is_empty() => {
                            tracing::info!(
                                path = %path.display(),
                                chars = result.text.len(),
                                "audio transcription complete"
                            );
                            enrichments.push(format!("[Transcription: {}]", result.text.trim()));
                        }
                        Ok(_) => {
                            tracing::debug!("audio transcription returned empty text");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "audio transcription failed");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to read audio file for transcription"
                    );
                }
            }
        }

        // Auto-describe images via vision model
        if auto_describe
            && att.media_type == ironclad_channels::MediaType::Image
            && local_path.is_some()
        {
            // Vision description is deferred to a future PR — for now, add a
            // placeholder noting the attachment exists
            let desc = att.filename.as_deref().unwrap_or("image");
            enrichments.push(format!("[Image attached: {desc}]"));
        }
    }

    // Prepend enrichments to content
    if !enrichments.is_empty() {
        let prefix = enrichments.join(" ");
        if inbound.content.is_empty() {
            inbound.content = prefix;
        } else {
            inbound.content = format!("{prefix}\n{}", inbound.content);
        }
    }
}
