//! HTTP handler for the non-streaming agent message endpoint.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use ironclad_core::InputAuthority;

use super::core;
use super::decomposition::{
    DecompositionOutcome, DelegationProvenance, apply_decomposition_decision,
    evaluate_decomposition_gate,
};
use super::resolve_web_scope;
use super::{AgentMessageRequest, AppState};

pub async fn agent_message(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<AgentMessageRequest>,
) -> impl IntoResponse {
    tracing::info!(channel = "api", session_id = ?body.session_id, "Processing agent message");
    let config = state.config.read().await;

    if body.content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "message content cannot be empty"})),
        ));
    }
    if body.content.len() > 32_768 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(json!({"error": "message content exceeds maximum length (32768 bytes)"})),
        ));
    }

    // Injection defense: block (>0.7), sanitize (0.3-0.7), or pass (<0.3)
    let threat = ironclad_agent::injection::check_injection(&body.content);
    let reduced_authority = threat.is_caution();
    if threat.is_blocked() {
        return Err((
            StatusCode::FORBIDDEN,
            axum::Json(json!({
                "error": "message_blocked",
                "reason": "prompt injection detected",
                "threat_score": threat.value(),
            })),
        ));
    }
    let user_content = if reduced_authority {
        tracing::info!(score = threat.value(), "Sanitizing caution-level input");
        ironclad_agent::injection::sanitize(&body.content)
    } else {
        body.content.clone()
    };

    // In-flight deduplication
    let dedup_fp = ironclad_llm::DedupTracker::fingerprint(
        "",
        &[ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: user_content.clone(),
            parts: None,
        }],
    );
    {
        let mut llm = state.llm.write().await;
        if !llm.dedup.check_and_track(&dedup_fp) {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(json!({
                    "error": "duplicate_request",
                    "reason": "identical request already in flight",
                })),
            ));
        }
    }

    let agent_id = config.agent.id.clone();
    let session_id = match &body.session_id {
        Some(sid) => match ironclad_db::sessions::get_session(&state.db, sid) {
            Ok(Some(session)) if session.agent_id == agent_id => sid.clone(),
            Ok(Some(_)) => {
                let mut llm = state.llm.write().await;
                llm.dedup.release(&dedup_fp);
                drop(llm);
                return Err((
                    StatusCode::FORBIDDEN,
                    axum::Json(json!({"error": "session does not belong to this agent"})),
                ));
            }
            Ok(None) => {
                let mut llm = state.llm.write().await;
                llm.dedup.release(&dedup_fp);
                drop(llm);
                return Err((
                    StatusCode::NOT_FOUND,
                    axum::Json(json!({"error": "session not found"})),
                ));
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to retrieve session");
                let mut llm = state.llm.write().await;
                llm.dedup.release(&dedup_fp);
                drop(llm);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(json!({"error": "internal server error"})),
                ));
            }
        },
        None => {
            let scope = match resolve_web_scope(&config, &body) {
                Ok(scope) => scope,
                Err(msg) => {
                    let mut llm = state.llm.write().await;
                    llm.dedup.release(&dedup_fp);
                    drop(llm);
                    return Err((StatusCode::BAD_REQUEST, axum::Json(json!({"error": msg}))));
                }
            };
            match ironclad_db::sessions::find_or_create(&state.db, &agent_id, Some(&scope)) {
                Ok(sid) => sid,
                Err(e) => {
                    tracing::error!(error = %e, "failed to create session");
                    let mut llm = state.llm.write().await;
                    llm.dedup.release(&dedup_fp);
                    drop(llm);
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        axum::Json(json!({"error": "internal server error"})),
                    ));
                }
            }
        }
    };

    // Set nickname on first message in a session
    let session_nickname = match ironclad_db::sessions::get_session(&state.db, &session_id) {
        Ok(Some(s)) if s.nickname.is_none() => {
            let nick = ironclad_db::sessions::derive_nickname(&body.content);
            ironclad_db::sessions::update_nickname(&state.db, &session_id, &nick)
                .inspect_err(|e| tracing::warn!(error = %e, session_id = %session_id, "failed to set session nickname"))
                .ok();
            Some(nick)
        }
        Ok(Some(s)) => s.nickname,
        _ => None,
    };

    // Store user message
    let user_msg_id = match ironclad_db::sessions::append_message(
        &state.db,
        &session_id,
        "user",
        &body.content,
    ) {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "failed to store user message");
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({"error": "internal server error"})),
            ));
        }
    };

    // Create a turn ID early so model-selection audit can be tied to this task.
    let turn_id = uuid::Uuid::new_v4().to_string();
    // Use the ModelRouter to select a model based on complexity
    let features = ironclad_llm::extract_features(&user_content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);

    // Decomposition gate: evaluate whether this task should be delegated
    let gate_decision = evaluate_decomposition_gate(&state, &user_content, complexity).await;
    let outcome = apply_decomposition_decision(&state, &gate_decision, &session_id, "api").await;
    let delegation_workflow_note = match outcome {
        DecompositionOutcome::SpecialistProposalPending { prompt } => {
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            drop(config);
            return Ok(axum::Json(json!({
                "session_id": session_id,
                "content": prompt,
                "decomposition": "requires_specialist_creation",
            })));
        }
        DecompositionOutcome::Centralized => None,
        DecompositionOutcome::Delegated { workflow_note } => Some(workflow_note),
    };

    // ── Prepare inference via core ───────────────────────────────────
    let config = state.config.read().await;
    let agent_name = config.agent.name.clone();
    let primary_model = config.models.primary.clone();
    let tier_adapt = config.tier_adapt.clone();
    drop(config);
    let personality = state.personality.read().await;
    let soul_text = personality.soul_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);

    let input = core::InferenceInput {
        state: &state,
        session_id: &session_id,
        user_content: &user_content,
        turn_id: &turn_id,
        channel_label: "api",
        agent_name,
        agent_id: agent_id.clone(),
        soul_text,
        firmware_text,
        primary_model,
        tier_adapt,
        delegation_workflow_note,
        inject_diagnostics: true,
        gate_system_note: None,
        delegated_execution_note: None,
    };

    let prepared = match core::prepare_inference(&input).await {
        Ok(p) => p,
        Err(msg) => {
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({"error": msg})),
            ));
        }
    };

    // ── Cache check ──────────────────────────────────────────────────
    let cached_response = core::check_cache(
        &state,
        &user_content,
        &prepared.cache_hash,
        prepared.query_embedding.as_deref(),
    )
    .await;

    if let Some(cached) = cached_response {
        let asst_id = match ironclad_db::sessions::append_message(
            &state.db,
            &session_id,
            "assistant",
            &cached.content,
        ) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = %e, "failed to store cached response");
                let mut llm = state.llm.write().await;
                llm.dedup.release(&dedup_fp);
                drop(llm);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(json!({"error": "internal server error"})),
                ));
            }
        };

        core::record_cost(
            &state,
            &cached.model,
            &prepared.provider_prefix,
            0,
            0,
            0.0,
            Some("cached"),
            true,
        );

        {
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
        }

        return Ok(axum::Json(json!({
            "session_id": session_id,
            "nickname": session_nickname,
            "user_message_id": user_msg_id,
            "assistant_message_id": asst_id,
            "content": cached.content,
            "model": cached.model,
            "cached": true,
            "tokens_saved": cached.tokens_saved,
        })));
    }

    // ── Inference + ReAct loop ───────────────────────────────────────
    let authority = if reduced_authority {
        InputAuthority::External
    } else {
        InputAuthority::Creator
    };
    let mut delegation_provenance = DelegationProvenance::default();
    let inference = core::run_inference_and_react(
        &state,
        &prepared,
        &turn_id,
        authority,
        Some("api"),
        &mut delegation_provenance,
    )
    .await;
    let assistant_content = inference.content;

    // ── Post-turn: store, cost, ingest, cache ────────────────────────
    let asst_id = match ironclad_db::sessions::append_message(
        &state.db,
        &session_id,
        "assistant",
        &assistant_content,
    ) {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "failed to store assistant response");
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({"error": "internal server error"})),
            ));
        }
    };

    core::record_cost(
        &state,
        &inference.model,
        &prepared.provider_prefix,
        inference.tokens_in,
        inference.tokens_out,
        inference.cost,
        None,
        false,
    );

    core::post_turn_ingest(&state, &session_id, &user_content, &assistant_content);

    core::store_in_cache(
        &state,
        &prepared.cache_hash,
        &user_content,
        &assistant_content,
        &inference.model,
        inference.tokens_out,
    )
    .await;

    // Release dedup tracking so subsequent identical requests are allowed
    {
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
    }

    // Background nickname refinement after 4+ messages
    if let Ok(count) = ironclad_db::sessions::message_count(&state.db, &session_id)
        && count >= 4
    {
        let refine_db = state.db.clone();
        let refine_llm = Arc::clone(&state.llm);
        let refine_sid = session_id.clone();
        let refine_oauth = state.oauth.clone();
        let refine_keystore = state.keystore.clone();
        tokio::spawn(async move {
            if let Err(e) = refine_session_nickname(
                &refine_db,
                &refine_llm,
                &refine_sid,
                &refine_oauth,
                &refine_keystore,
            )
            .await
            {
                tracing::debug!(error = %e, session = %refine_sid, "nickname refinement skipped");
            }
        });
    }

    Ok(axum::Json(json!({
        "session_id": session_id,
        "nickname": session_nickname,
        "user_message_id": user_msg_id,
        "assistant_message_id": asst_id,
        "content": assistant_content,
        "model": inference.model,
        "cached": false,
        "tokens_in": inference.tokens_in,
        "tokens_out": inference.tokens_out,
        "cost": inference.cost,
        "threat_score": threat.value(),
        "reduced_authority": reduced_authority,
        "react_turns": inference.react_turns,
    })))
}

/// Refine a session's nickname using the LLM to summarize conversation topics.
pub(super) async fn refine_session_nickname(
    db: &ironclad_db::Database,
    llm: &Arc<tokio::sync::RwLock<ironclad_llm::LlmService>>,
    session_id: &str,
    oauth: &ironclad_llm::oauth::OAuthManager,
    keystore: &ironclad_core::keystore::Keystore,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let messages = ironclad_db::sessions::list_messages(db, session_id, Some(8))?;
    if messages.len() < 4 {
        return Ok(());
    }

    let mut conversation = String::with_capacity(1024);
    for m in &messages {
        let prefix = if m.role == "user" {
            "User"
        } else {
            "Assistant"
        };
        let snippet: String = m.content.chars().take(200).collect();
        conversation.push_str(&format!("{prefix}: {snippet}\n"));
    }

    let prompt = format!(
        "Summarize this conversation topic in 3-6 words as a short title. \
         Only output the title, nothing else.\n\n{conversation}"
    );

    let llm_read = llm.read().await;
    let model_id = llm_read.router.select_model().to_string();
    let model_for_api = model_id
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model_id)
        .to_string();

    let provider = llm_read.providers.get_by_model(&model_id);
    let (url, api_key, auth_header, format, extra_headers) = match provider {
        Some(p) => {
            let key = super::super::admin::resolve_provider_key(
                &p.name,
                p.is_local,
                &p.auth_mode,
                p.api_key_ref.as_deref(),
                &p.api_key_env,
                oauth,
                keystore,
            )
            .await
            .unwrap_or_else(|| {
                if !p.is_local {
                    tracing::warn!(provider = %p.name, "API key resolved to None for non-local provider");
                }
                String::new()
            });
            (
                format!("{}{}", p.url, p.chat_path),
                key,
                p.auth_header.clone(),
                p.format,
                p.extra_headers.clone(),
            )
        }
        None => return Ok(()),
    };

    let req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages: vec![ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: prompt,
            parts: None,
        }],
        max_tokens: Some(30),
        temperature: Some(0.3),
        system: None,
        quality_target: None,
    };

    let body = ironclad_llm::format::translate_request(&req, format)?;
    let resp = llm_read
        .client
        .forward_with_provider(&url, &api_key, body, &auth_header, &extra_headers)
        .await?;
    drop(llm_read);

    let unified = ironclad_llm::format::translate_response(&resp, format)?;
    let nickname = unified.content.trim().trim_matches('"').to_string();

    if !nickname.is_empty() && nickname.len() <= 60 {
        ironclad_db::sessions::update_nickname(db, session_id, &nickname)?;
        tracing::info!(
            session = %session_id,
            nickname = %nickname,
            "Refined session nickname via LLM"
        );
    }
    Ok(())
}
