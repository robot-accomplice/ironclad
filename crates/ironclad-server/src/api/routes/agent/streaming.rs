//! SSE streaming endpoint for agent message inference.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::StreamExt;
use serde_json::json;

use super::core;
use super::guards::DedupGuard;
use super::routing::{fallback_candidates, resolve_inference_provider};
use super::{AgentMessageRequest, AppState};

/// Streaming version of `agent_message`. Returns an SSE stream of `StreamChunk`
/// events as tokens arrive from the LLM provider. The accumulated response is
/// stored in the session and published to the EventBus after the stream ends.
pub async fn agent_message_stream(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<AgentMessageRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>,
    (StatusCode, axum::Json<serde_json::Value>),
> {
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

    let threat = ironclad_agent::injection::check_injection(&body.content);
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
    let user_content = if threat.is_caution() {
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
            let scope = match super::resolve_web_scope(&config, &body) {
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

    match ironclad_db::sessions::append_message(&state.db, &session_id, "user", &body.content) {
        Ok(_) => {}
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
    }

    let turn_id = uuid::Uuid::new_v4().to_string();

    // Read config values needed for InferenceInput before dropping the lock.
    let tier_adapt = config.tier_adapt.clone();
    let agent_name = config.agent.name.clone();
    let agent_id_for_input = config.agent.id.clone();
    let primary_model = config.models.primary.clone();
    let personality = state.personality.read().await;
    let soul_text = personality.soul_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);
    drop(config);

    let input = core::InferenceInput {
        state: &state,
        session_id: &session_id,
        user_content: &user_content,
        turn_id: &turn_id,
        channel_label: "api-stream",
        agent_name,
        agent_id: agent_id_for_input,
        soul_text,
        firmware_text,
        primary_model,
        tier_adapt,
        delegation_workflow_note: None,
        inject_diagnostics: true,
        gate_system_note: None,
        delegated_execution_note: None,
    };
    let prepared = match core::prepare_inference(&input).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "streaming prepare_inference failed");
            let mut llm = state.llm.write().await;
            llm.dedup.release(&dedup_fp);
            drop(llm);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({"error": e})),
            ));
        }
    };

    let model = prepared.model.clone();
    let unified_req = prepared.request;

    // Use the same fallback surface as non-stream inference.
    let candidates = {
        let cfg = state.config.read().await;
        fallback_candidates(&cfg, &model)
    };
    let mut selected_model = model.clone();
    let mut provider_prefix = model.split('/').next().unwrap_or("unknown").to_string();
    let mut cost_in = 0.0_f64;
    let mut cost_out = 0.0_f64;
    let mut last_error = String::new();
    let mut chunk_stream_opt = None;

    for candidate in candidates {
        let candidate_prefix = candidate.split('/').next().unwrap_or("unknown").to_string();
        {
            let llm = state.llm.read().await;
            if llm.breakers.is_blocked(&candidate_prefix) {
                last_error = format!("{candidate_prefix} circuit breaker open");
                continue;
            }
        }

        let Some(resolved) = resolve_inference_provider(&state, &candidate).await else {
            last_error = format!("no provider configured for {candidate}");
            continue;
        };

        if !resolved.is_local && resolved.api_key.is_empty() {
            last_error = format!("no API key for {}", resolved.provider_prefix);
            continue;
        }

        let mut req_clone = unified_req.clone();
        req_clone.model = candidate
            .split('/')
            .nth(1)
            .unwrap_or(&candidate)
            .to_string();
        let llm_body = match ironclad_llm::format::translate_request(&req_clone, resolved.format) {
            Ok(body) => body,
            Err(e) => {
                tracing::error!(error = %e, "failed to translate streaming LLM request");
                let mut llm = state.llm.write().await;
                llm.dedup.release(&dedup_fp);
                drop(llm);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(json!({"error": "internal server error"})),
                ));
            }
        };

        // Clone the HTTP client so we can release the RwLock before the
        // potentially long-running network call (SA-HIGH-1).
        let llm_client = state.llm.read().await.client.clone();
        let mut llm_body_stream = llm_body;
        llm_body_stream["stream"] = serde_json::json!(true);
        let result = llm_client
            .forward_stream(
                &resolved.url,
                &resolved.api_key,
                llm_body_stream,
                &resolved.auth_header,
                &resolved.extra_headers,
            )
            .await
            .map(|raw| ironclad_llm::SseChunkStream::new(raw, resolved.format));

        match result {
            Ok(stream) => {
                let mut llm = state.llm.write().await;
                llm.breakers.record_success(&resolved.provider_prefix);
                drop(llm);
                selected_model = candidate.clone();
                provider_prefix = resolved.provider_prefix;
                cost_in = resolved.cost_in;
                cost_out = resolved.cost_out;
                chunk_stream_opt = Some(stream);
                break;
            }
            Err(e) => {
                let is_credit = e.is_credit_error();
                let mut llm = state.llm.write().await;
                if is_credit {
                    llm.breakers.record_credit_error(&resolved.provider_prefix);
                } else {
                    llm.breakers.record_failure(&resolved.provider_prefix);
                }
                drop(llm);
                last_error = e.to_string();
            }
        }
    }

    let Some(chunk_stream) = chunk_stream_opt else {
        tracing::error!(error = %last_error, "all streaming fallback candidates failed");
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err((
            StatusCode::BAD_GATEWAY,
            axum::Json(json!({"error": "upstream provider error"})),
        ));
    };

    // Send initial metadata event, then stream chunks, then send a final summary
    let session_id_clone = session_id.clone();
    let turn_id_clone = turn_id.clone();
    let model_clone = selected_model.clone();
    // BUG-027: Capture actual agent_id for WebSocket events instead of hardcoding.
    let agent_id_clone = {
        let config = state.config.read().await;
        config.agent.id.clone()
    };
    let event_bus = state.event_bus.clone();
    let db = state.db.clone();
    let cache_hash = prepared.cache_hash;
    let llm_arc = Arc::clone(&state.llm);
    let hmac_secret_clone = state.hmac_secret.clone();
    let user_content_clone = user_content.clone();
    let state_clone = state.clone();

    // DedupGuard ensures the fingerprint is released even if the client disconnects
    // mid-stream and the generator is dropped before reaching the explicit release.
    let dedup_guard = DedupGuard {
        llm: Arc::clone(&state.llm),
        fingerprint: dedup_fp,
    };

    let sse_stream = async_stream::stream! {
        // Move the guard into the generator so it drops with the stream
        let _dedup_guard = dedup_guard;

        // Opening event with session metadata
        let open = json!({
            "type": "stream_start",
            "session_id": session_id_clone,
            "turn_id": turn_id_clone,
            "model": model_clone,
        });
        yield Ok(Event::default().data(open.to_string()));
        event_bus.publish(
            json!({
                "type": "agent_working",
                "agent_id": agent_id_clone,
                "workstation": "llm",
                "activity": "inference",
                "session_id": session_id_clone,
                "model": model_clone,
            })
            .to_string(),
        );

        let mut accumulator = ironclad_llm::format::StreamAccumulator::default();
        let stream_start = std::time::Instant::now();
        let mut stream = std::pin::pin!(chunk_stream);

        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    accumulator.push(&chunk);

                    let chunk_event = json!({
                        "type": "stream_chunk",
                        "delta": chunk.delta,
                        "done": false,
                        "session_id": session_id_clone,
                    });
                    event_bus.publish(chunk_event.to_string());

                    let sse_data = json!({
                        "type": "chunk",
                        "delta": chunk.delta,
                        "model": chunk.model,
                        "finish_reason": chunk.finish_reason,
                    });
                    yield Ok(Event::default().data(sse_data.to_string()));
                }
                Err(e) => {
                    tracing::error!(error = %e, "streaming chunk error from provider");
                    {
                        let mut llm = llm_arc.write().await;
                        llm.breakers.record_failure(&provider_prefix);
                        llm.breakers.set_capacity_pressure(&provider_prefix, false);
                    }
                    let err_data = json!({"type": "error", "error": "upstream provider error"});
                    yield Ok(Event::default().data(err_data.to_string()));
                    break;
                }
            }
        }

        let unified_resp = accumulator.finalize();

        // HMAC boundary + L4 output scan via core helper
        let assistant_content = core::sanitize_model_output(
            unified_resp.content.clone(),
            hmac_secret_clone.as_ref(),
        );

        // Streaming-specific: emit SSE event if L4 scan blocks
        let content_blocked = ironclad_agent::injection::scan_output(&assistant_content);
        let assistant_content = if content_blocked {
            tracing::warn!("L4 output scan flagged streaming response");
            let blocked_event = json!({
                "type": "stream_blocked",
                "reason": "output safety filter triggered",
                "session_id": session_id_clone,
            });
            yield Ok(Event::default().data(blocked_event.to_string()));
            "[Response blocked by output safety filter]".to_string()
        } else {
            assistant_content
        };

        // Post-stream: store assistant response (scanned content)
        if let Err(e) = ironclad_db::sessions::append_message(
            &db,
            &session_id_clone,
            "assistant",
            &assistant_content,
        ) {
            tracing::error!(error = %e, session_id = %session_id_clone, "failed to persist assistant response after streaming inference");
        }

        // Record inference cost via core
        let cost = unified_resp.tokens_in as f64 * cost_in + unified_resp.tokens_out as f64 * cost_out;
        if let Err(e) = ironclad_db::sessions::create_turn_with_id(
            &db,
            &turn_id_clone,
            &session_id_clone,
            Some(&model_clone),
            Some(unified_resp.tokens_in as i64),
            Some(unified_resp.tokens_out as i64),
            Some(cost),
        ) {
            tracing::warn!(error = %e, turn_id = %turn_id_clone, "failed to persist streaming turn");
        }
        let stream_latency_ms = stream_start.elapsed().as_millis() as i64;
        core::record_cost(
            &state_clone,
            &model_clone,
            &provider_prefix,
            unified_resp.tokens_in as i64,
            unified_resp.tokens_out as i64,
            cost,
            None,
            false,
            Some(stream_latency_ms),
            None, // quality_score not computed for streaming
            false,
        );

        // Capacity + breaker tracking (streaming-specific -- needs raw LLM access)
        {
            let mut llm = llm_arc.write().await;
            llm.breakers.record_success(&provider_prefix);
            let total_tokens = unified_resp.tokens_in + unified_resp.tokens_out;
            llm.capacity.record(&provider_prefix, total_tokens as u64);
            let pressured = llm.capacity.is_sustained_hot(&provider_prefix);
            llm.breakers.set_capacity_pressure(&provider_prefix, pressured);
        }

        // Cache write-through via core
        core::store_in_cache(
            &state_clone,
            &cache_hash,
            &user_content_clone,
            &assistant_content,
            &model_clone,
            unified_resp.tokens_out as i64,
        ).await;

        // Background memory ingestion.
        // Streaming currently does not execute a ReAct tool loop, but it may still emit
        // tool-call intents in provider output; persist those intents so episodic memory
        // is not blind on the streaming path.
        let streamed_tool_results: Vec<(String, String)> = super::tools::parse_tool_calls(&assistant_content)
            .into_iter()
            .map(|(name, params)| (name, format!("unexecuted_streaming_tool_call: {params}")))
            .collect();
        core::post_turn_ingest(
            &state_clone,
            &session_id_clone,
            &user_content_clone,
            &assistant_content,
            &streamed_tool_results,
        );

        let done_event = json!({
            "type": "stream_chunk",
            "delta": "",
            "done": true,
            "session_id": session_id_clone,
        });
        event_bus.publish(done_event.to_string());
        event_bus.publish(
            json!({
                "type": "agent_idle",
                "agent_id": agent_id_clone,
                "workstation": "llm",
                "session_id": session_id_clone,
            })
            .to_string(),
        );

        let final_event = json!({
            "type": "stream_end",
            "session_id": session_id_clone,
            "turn_id": turn_id_clone,
            "model": unified_resp.model,
            "tokens_in": unified_resp.tokens_in,
            "tokens_out": unified_resp.tokens_out,
            "content_length": assistant_content.len(),
            "content_blocked": content_blocked,
        });
        yield Ok(Event::default().data(final_event.to_string()));

        // Guard drops here on normal completion, releasing the fingerprint
    };

    Ok(Sse::new(sse_stream).keep_alive(KeepAlive::default()))
}
