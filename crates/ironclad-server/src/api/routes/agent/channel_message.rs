//! Channel message processing for Telegram, Discord, Signal, Email, and webhooks.

use ironclad_core::InputAuthority;

use super::AppState;
use super::channel_helpers::{
    estimate_inference_latency, resolve_channel_chat_id, resolve_channel_scope,
    send_thinking_indicator, send_typing_indicator,
};
use super::core;
use super::decomposition::{
    DecompositionDecision, DecompositionOutcome, DelegationProvenance,
    apply_decomposition_decision, build_gate_system_note, evaluate_decomposition_gate,
    maybe_handle_specialist_creation_controls,
};

pub async fn process_channel_message(
    state: &AppState,
    inbound: ironclad_channels::InboundMessage,
) -> Result<(), String> {
    tracing::info!(channel = %inbound.platform, peer = %inbound.sender_id, "Processing channel message");
    let chat_id = resolve_channel_chat_id(&inbound);
    let platform = inbound.platform.clone();

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
    let user_content = if threat.is_caution() {
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
    if let Some(reply) =
        maybe_handle_specialist_creation_controls(state, &session_id, &user_content).await
    {
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
    drop(config);
    let personality = state.personality.read().await;
    let soul_text = personality.soul_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);

    let channel_authority = {
        let sender_trusted = !trusted.is_empty()
            && (trusted.iter().any(|id| id == &chat_id)
                || trusted.iter().any(|id| id == &inbound.sender_id));
        if threat.is_caution() || !sender_trusted {
            InputAuthority::External
        } else {
            InputAuthority::Creator
        }
    };

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
    {
        if let Some(notice) = model_switch_notice.as_ref() {
            state
                .channel_router
                .send_reply(&platform, &chat_id, notice.clone())
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "failed to send model switch notice"))
                .ok();
        }
        let estimated_latency = estimate_inference_latency(
            prepared.tier,
            user_content.len(),
            &prepared.model,
            &primary_model,
            state,
        )
        .await;

        if estimated_latency >= thinking_threshold {
            send_thinking_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;
        } else {
            send_typing_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;
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
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err(msg);
        }
    };

    // Send reply to channel
    if let Err(e) = state
        .channel_router
        .send_reply(&platform, &chat_id, result.content.clone())
        .await
    {
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err(e.to_string());
    }

    // Release dedup tracking
    {
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
    }

    Ok(())
}
