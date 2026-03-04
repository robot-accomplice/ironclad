//! Shared inference core used by all three entry points (API, streaming, channel).
//!
//! `prepare_inference` builds a `PreparedInference` from an `InferenceInput`, handling:
//! model selection, embedding, RAG retrieval, history, system prompt, HMAC, context building.
//!
//! `run_react_loop` drives the Think→Act→Observe→Finish cycle on top of a prepared request.
//!
//! `post_turn_ingest` spawns background memory + embedding work.

use std::sync::Arc;

use ironclad_agent::agent_loop::{AgentLoop, ReactAction, ReactState};
use ironclad_core::config::TierAdaptConfig;
use ironclad_core::{InputAuthority, ModelTier};

use super::AppState;
use super::decomposition::DelegationProvenance;
use super::diagnostics::{collect_runtime_diagnostics, diagnostics_system_note};
use super::guards::{
    enforce_execution_truth_guard, enforce_model_identity_truth_guard, enforce_non_repetition,
    enforce_subagent_claim_guard,
};
use super::routing::{
    infer_with_fallback, persist_model_selection_audit, select_routed_model_with_audit,
};
use super::tools::{execute_tool_call, parse_tool_call, parse_tool_calls};

/// Caller-supplied context that differs across the three entry points.
pub(super) struct InferenceInput<'a> {
    pub state: &'a AppState,
    pub session_id: &'a str,
    pub user_content: &'a str,
    pub turn_id: &'a str,
    /// Label for model audit trail ("api", "api-stream", "telegram", etc.)
    pub channel_label: &'a str,
    /// System prompt fragments from caller
    pub agent_name: String,
    pub agent_id: String,
    pub soul_text: String,
    pub firmware_text: String,
    pub primary_model: String,
    pub tier_adapt: TierAdaptConfig,
    /// Optional delegation workflow note injected into system prompt
    pub delegation_workflow_note: Option<String>,
    /// Whether to inject runtime diagnostics (API yes, channels no)
    pub inject_diagnostics: bool,
    /// Optional gate system note for channels
    pub gate_system_note: Option<String>,
    /// Optional delegated execution note for channels
    pub delegated_execution_note: Option<String>,
}

/// Result of `prepare_inference` — everything needed to call the LLM.
pub(super) struct PreparedInference {
    pub model: String,
    pub model_for_api: String,
    pub tier: ModelTier,
    pub request: ironclad_llm::format::UnifiedRequest,
    pub previous_assistant: Option<String>,
    pub query_embedding: Option<Vec<f32>>,
    pub cache_hash: String,
}

/// Result of a completed (non-streaming) inference cycle.
pub(super) struct InferenceOutput {
    pub content: String,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: f64,
    pub react_turns: usize,
    pub latency_ms: u64,
    pub quality_score: f64,
    pub escalated: bool,
    /// Tool calls executed during the ReAct loop: (tool_name, result_text).
    pub tool_results: Vec<(String, String)>,
}

/// Build a `PreparedInference` from the caller's `InferenceInput`.
///
/// Handles: model routing, embedding, RAG retrieval, history, system prompt,
/// HMAC injection, context assembly, and tier adaptation.
pub(super) async fn prepare_inference(
    input: &InferenceInput<'_>,
) -> Result<PreparedInference, String> {
    let state = input.state;

    // Model selection + audit
    let features = ironclad_llm::extract_features(input.user_content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let model_audit = select_routed_model_with_audit(state, input.user_content).await;
    let model = model_audit.selected_model.clone();
    let complexity_label = format!("{complexity:?}");
    persist_model_selection_audit(
        state,
        input.turn_id,
        input.session_id,
        input.channel_label,
        Some(&complexity_label),
        input.user_content,
        &model_audit,
    )
    .await;
    let _ = ironclad_db::sessions::update_model(&state.db, input.session_id, &model);

    // Tier resolution
    let tier = {
        let llm = state.llm.read().await;
        llm.providers
            .get_by_model(&model)
            .map(|p| p.tier)
            .unwrap_or_else(|| ironclad_llm::tier::classify(&model))
    };

    // Embedding for RAG + cache L2
    let query_embedding = {
        let llm = state.llm.read().await;
        llm.embedding
            .embed_single(input.user_content)
            .await
            .inspect_err(|e| {
                tracing::warn!(error = %e, "embedding generation failed, RAG retrieval will be skipped")
            })
            .ok()
    };

    // Cache lookup
    let cache_hash = ironclad_llm::SemanticCache::compute_hash("", "", input.user_content);

    // Memory retrieval
    let complexity_level = ironclad_agent::context::determine_level(complexity);
    let ann_ref = if state.ann_index.is_built() {
        Some(&state.ann_index)
    } else {
        None
    };
    let memories = state.retriever.retrieve_with_ann(
        &state.db,
        input.session_id,
        input.user_content,
        query_embedding.as_deref(),
        complexity_level,
        ann_ref,
    );

    // History
    let history_messages =
        ironclad_db::sessions::list_messages(&state.db, input.session_id, Some(50))
            .map_err(|e| format!("failed to load conversation history: {e}"))?;
    let previous_assistant = history_messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.clone());
    let history: Vec<ironclad_llm::format::UnifiedMessage> = history_messages
        .iter()
        .rev()
        .skip(1) // skip the user message just appended by caller
        .rev()
        .map(|m| ironclad_llm::format::UnifiedMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            parts: None,
        })
        .collect();

    // System prompt
    let model_for_api = model
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model)
        .to_string();
    let system_prompt = if input.soul_text.is_empty() {
        format!(
            "You are {name}, an autonomous AI agent (id: {id}). \
             When asked who you are, always identify as {name}. \
             Never reveal the underlying model name or claim to be a generic assistant.",
            name = input.agent_name,
            id = input.agent_id,
        )
    } else {
        let mut prompt = input.soul_text.clone();
        if !input.firmware_text.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&input.firmware_text);
        }
        prompt
    };
    let system_prompt = if let Some(ref wf_note) = input.delegation_workflow_note {
        format!("{system_prompt}\nWorkflow: {wf_note}")
    } else {
        system_prompt
    };
    // Build tool definitions early so we can embed a text-based tool summary in the
    // system prompt. This ensures models without native function-calling support can
    // still discover and invoke tools via the text-embedded JSON format.
    let tools = super::decomposition::build_all_tool_definitions(&state.tools);
    let tool_summary: Vec<(String, String)> = tools
        .iter()
        .map(|t| (t.name.clone(), t.description.clone()))
        .collect();
    let system_prompt = format!(
        "{system_prompt}{}{}",
        ironclad_agent::prompt::runtime_metadata_block(
            env!("CARGO_PKG_VERSION"),
            &input.primary_model,
            &model,
        ),
        ironclad_agent::prompt::tool_use_instructions(&tool_summary),
    );
    let system_prompt =
        ironclad_agent::prompt::inject_hmac_boundary(&system_prompt, state.hmac_secret.as_ref());
    if !ironclad_agent::prompt::verify_hmac_boundary(&system_prompt, state.hmac_secret.as_ref()) {
        tracing::error!("HMAC boundary verification failed immediately after injection");
        return Err("internal HMAC verification failure".into());
    }

    // Context assembly
    let mut messages = ironclad_agent::context::build_context(
        complexity_level,
        &system_prompt,
        &memories,
        &history,
    );

    // Session checkpoint restore: inject most recent checkpoint context on resume.
    match ironclad_db::checkpoint::load_checkpoint(&state.db, input.session_id) {
        Ok(Some(cp)) => {
            let mut checkpoint_note = format!(
                "Session checkpoint restore (turn_count={}): {}",
                cp.turn_count, cp.memory_summary
            );
            if let Some(active_tasks) = cp.active_tasks
                && !active_tasks.trim().is_empty()
            {
                checkpoint_note.push_str("\nActive tasks: ");
                checkpoint_note.push_str(&active_tasks);
            }
            if let Some(digest) = cp.conversation_digest
                && !digest.trim().is_empty()
            {
                checkpoint_note.push_str("\nConversation digest: ");
                checkpoint_note.push_str(&digest);
            }
            messages.push(ironclad_llm::format::UnifiedMessage {
                role: "system".into(),
                content: checkpoint_note,
                parts: None,
            });
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "failed to load context checkpoint"),
    }

    // Hippocampus context: compact table summary for ambient storage awareness
    match ironclad_db::hippocampus::compact_summary(&state.db) {
        Ok(summary) if !summary.is_empty() => {
            messages.push(ironclad_llm::format::UnifiedMessage {
                role: "system".into(),
                content: summary,
                parts: None,
            });
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to generate hippocampus summary");
        }
        _ => {}
    }

    // Optional: runtime diagnostics (API paths inject; channels deliberately skip)
    if input.inject_diagnostics {
        let runtime_diag = collect_runtime_diagnostics(state).await;
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: diagnostics_system_note(&runtime_diag),
            parts: None,
        });
    }

    // Optional: gate system note (channels inject decomposition decision)
    if let Some(ref note) = input.gate_system_note {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: note.clone(),
            parts: None,
        });
    }
    if let Some(ref note) = input.delegated_execution_note {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: note.clone(),
            parts: None,
        });
    }

    // Ensure user message is last
    if messages
        .last()
        .is_none_or(|m| m.content != input.user_content)
    {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: input.user_content.to_string(),
            parts: None,
        });
    }
    // Prompt compression gate — only when enabled in config
    {
        let cfg = input.state.config.read().await;
        if cfg.cache.prompt_compression {
            ironclad_agent::context::compress_context(
                &mut messages,
                cfg.cache.compression_target_ratio,
            );
        }
    }

    ironclad_llm::tier::adapt_for_tier(tier, &mut messages, &input.tier_adapt);

    let request = ironclad_llm::format::UnifiedRequest {
        model: model_for_api.clone(),
        messages,
        max_tokens: Some(2048),
        temperature: None,
        system: None,
        quality_target: None,
        tools,
    };

    Ok(PreparedInference {
        model,
        model_for_api,
        tier,
        request,
        previous_assistant,
        query_embedding,
        cache_hash,
    })
}

/// Strip forged HMAC boundaries + L4 output scan on a single piece of content.
pub(super) fn sanitize_model_output(content: String, hmac_secret: &[u8]) -> String {
    let content = if content.contains("<<<TRUST_BOUNDARY:") {
        if !ironclad_agent::prompt::verify_hmac_boundary(&content, hmac_secret) {
            tracing::warn!("HMAC boundary tampered in model output, stripping");
            ironclad_agent::prompt::strip_hmac_boundaries(&content)
        } else {
            content
        }
    } else {
        content
    };
    if ironclad_agent::injection::scan_output(&content) {
        tracing::warn!("L4 output scan flagged model response, blocking");
        "[Response blocked by output safety filter]".to_string()
    } else {
        content
    }
}

/// Run the non-streaming inference + ReAct loop. Returns the final assistant content
/// along with token/cost totals.
pub(super) async fn run_inference_and_react(
    state: &AppState,
    prepared: &PreparedInference,
    turn_id: &str,
    authority: InputAuthority,
    channel_label: Option<&str>,
    delegation_provenance: &mut DelegationProvenance,
) -> InferenceOutput {
    // Initial inference
    let mut resolved_model = prepared.model.clone();
    let (
        initial_content,
        mut total_in,
        mut total_out,
        mut total_cost,
        latency_ms,
        quality_score,
        escalated,
    ) = match infer_with_fallback(state, &prepared.request, &prepared.model).await {
        Ok(result) => {
            resolved_model = result.model.clone();
            (
                result.content,
                result.tokens_in,
                result.tokens_out,
                result.cost,
                result.latency_ms,
                result.quality_score,
                result.escalated,
            )
        }
        Err(last_error) => (
            super::tools::provider_failure_user_message(&last_error.to_string(), true),
            0,
            0,
            0.0,
            0,
            0.0,
            false,
        ),
    };

    let initial_content = sanitize_model_output(initial_content, state.hmac_secret.as_ref());

    // ReAct loop — supports multiple tool calls per LLM turn
    let mut react_loop = AgentLoop::new(10);
    let mut final_content = initial_content.clone();
    let mut tool_results_acc: Vec<(String, String)> = Vec::new();

    let mut pending_calls = parse_tool_calls(&initial_content);
    // Fall back to single-parse for edge cases (e.g. embedded JSON)
    if pending_calls.is_empty()
        && let Some(single) = parse_tool_call(&initial_content)
    {
        pending_calls.push(single);
    }

    if !pending_calls.is_empty() {
        react_loop.transition(ReactAction::Think);
        let mut react_messages = prepared.request.messages.clone();
        react_messages.push(ironclad_llm::format::UnifiedMessage {
            role: "assistant".into(),
            content: initial_content,
            parts: None,
        });

        while !pending_calls.is_empty() {
            let mut observations = Vec::new();
            let mut batch_aborted = false;

            for (tn, tp) in &pending_calls {
                // Loop detection: break if the same tool+params repeats consecutively
                if react_loop.is_looping(tn, &tp.to_string()) {
                    tracing::warn!(
                        tool = tn.as_str(),
                        "ReAct loop detected — same tool+params repeated"
                    );
                    batch_aborted = true;
                    break;
                }

                // Track delegation provenance for channel claim guard
                if tn.to_ascii_lowercase().contains("subagent")
                    || tn.to_ascii_lowercase().contains("delegate")
                {
                    delegation_provenance.subagent_task_started = true;
                }

                react_loop.transition(ReactAction::Act {
                    tool_name: tn.clone(),
                    params: tp.to_string(),
                });
                if react_loop.state == ReactState::Done {
                    batch_aborted = true;
                    break;
                }

                let tool_result =
                    execute_tool_call(state, tn, tp, turn_id, authority, channel_label).await;
                let observation = match tool_result {
                    Ok(ref out) => {
                        if tn.to_ascii_lowercase().contains("subagent")
                            || tn.to_ascii_lowercase().contains("delegate")
                        {
                            delegation_provenance.subagent_task_completed = true;
                            delegation_provenance.subagent_result_attached = !out.trim().is_empty();
                        }
                        format!("[Tool {tn} succeeded]: {out}")
                    }
                    Err(ref err) => format!("[Tool {tn} failed]: {err}"),
                };
                // Accumulate tool results for memory ingestion
                let result_text = match &tool_result {
                    Ok(out) => out.clone(),
                    Err(err) => format!("error: {err}"),
                };
                tool_results_acc.push((tn.clone(), result_text));

                let observation = if ironclad_agent::injection::scan_output(&observation) {
                    tracing::warn!(
                        tool = tn.as_str(),
                        "tool result flagged by output scan, sanitizing"
                    );
                    format!("[Tool {tn} result blocked by safety filter]")
                } else {
                    observation
                };

                observations.push(observation);
            }

            if batch_aborted && observations.is_empty() {
                final_content = "I stopped tool execution because the same tool call kept repeating without progress. Please rephrase or provide a more specific command.".to_string();
                break;
            }

            react_loop.transition(ReactAction::Observe);
            let combined_observation = observations.join("\n\n");
            react_messages.push(ironclad_llm::format::UnifiedMessage {
                role: "user".into(),
                content: combined_observation,
                parts: None,
            });

            if react_loop.state == ReactState::Done {
                break;
            }

            let follow_req = ironclad_llm::format::UnifiedRequest {
                model: prepared.request.model.clone(),
                messages: react_messages.clone(),
                max_tokens: Some(2048),
                temperature: None,
                system: None,
                quality_target: None,
                tools: prepared.request.tools.clone(),
            };

            let follow_content =
                match infer_with_fallback(state, &follow_req, &prepared.model).await {
                    Ok(result) => {
                        resolved_model = result.model.clone();
                        total_in += result.tokens_in;
                        total_out += result.tokens_out;
                        total_cost += result.cost;
                        result.content
                    }
                    Err(e) => format!("LLM follow-up error: {e}"),
                };

            react_messages.push(ironclad_llm::format::UnifiedMessage {
                role: "assistant".into(),
                content: follow_content.clone(),
                parts: None,
            });

            let follow_content = sanitize_model_output(follow_content, state.hmac_secret.as_ref());

            pending_calls = parse_tool_calls(&follow_content);
            if pending_calls.is_empty()
                && let Some(single) = parse_tool_call(&follow_content)
            {
                pending_calls.push(single);
            }
            if pending_calls.is_empty() {
                react_loop.transition(ReactAction::Finish);
                final_content = follow_content;
            }
        }

        if !pending_calls.is_empty()
            && (final_content.trim().is_empty() || final_content.contains("\"tool_call\""))
        {
            final_content = "I could not complete the requested tool workflow this turn. Please retry with a narrower command.".to_string();
        }
    }

    // Post-ReAct guards
    let final_content = enforce_subagent_claim_guard(final_content, delegation_provenance);
    let final_content = enforce_execution_truth_guard(
        prepared
            .request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or_default(),
        final_content,
        &tool_results_acc,
    );
    let final_content = enforce_model_identity_truth_guard(
        prepared
            .request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or_default(),
        final_content,
        &resolved_model,
    );
    let final_content =
        enforce_non_repetition(final_content, prepared.previous_assistant.as_deref());

    InferenceOutput {
        content: final_content,
        model: resolved_model,
        tokens_in: total_in,
        tokens_out: total_out,
        cost: total_cost,
        react_turns: react_loop.turn_count,
        latency_ms,
        quality_score,
        escalated,
        tool_results: tool_results_acc,
    }
}

/// Check the semantic cache. Returns `Some(CachedResponse)` on hit.
pub(super) async fn check_cache(
    state: &AppState,
    user_content: &str,
    cache_hash: &str,
    query_embedding: Option<&[f32]>,
) -> Option<ironclad_llm::CachedResponse> {
    let _ = user_content;
    let _ = query_embedding;
    let mut llm = state.llm.write().await;
    // High-integrity default: only exact/tool-TTL cache hits.
    // Semantic near-match cache reuse can fabricate wrong instruction-bound outputs.
    llm.cache.lookup_strict(cache_hash)
}

/// Store a response in the semantic cache.
pub(super) async fn store_in_cache(
    state: &AppState,
    cache_hash: &str,
    user_content: &str,
    content: &str,
    model: &str,
    tokens_out: i64,
) {
    if tokens_out > 0 {
        let entry = ironclad_llm::CachedResponse {
            content: content.to_string(),
            model: model.to_string(),
            tokens_saved: tokens_out as u32,
            created_at: std::time::Instant::now(),
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3600),
            hits: 0,
            involved_tools: false,
            embedding: None,
        };
        let mut llm = state.llm.write().await;
        llm.cache
            .store_with_embedding(cache_hash, user_content, entry);
    }
}

/// Spawn background memory ingestion + embedding generation for a completed turn.
pub(super) fn post_turn_ingest(
    state: &AppState,
    session_id: &str,
    user_content: &str,
    assistant_content: &str,
    tool_results: &[(String, String)],
) {
    let db = state.db.clone();
    let config = Arc::clone(&state.config);
    let session = session_id.to_string();
    let user = user_content.to_string();
    let assistant = assistant_content.to_string();
    let tools = tool_results.to_vec();
    let llm = Arc::clone(&state.llm);
    tokio::spawn(async move {
        ironclad_agent::memory::ingest_turn(&db, &session, &user, &assistant, &tools);

        // Periodic context checkpoint
        let ctx_cfg = &config.read().await.context;
        if ctx_cfg.checkpoint_enabled
            && let Ok(msgs) = ironclad_db::sessions::list_messages(&db, &session, None)
        {
            let turn_count = msgs.len() as u32;
            if turn_count > 0 && turn_count.is_multiple_of(ctx_cfg.checkpoint_interval_turns) {
                let mem_summary = msgs
                    .iter()
                    .filter(|m| m.role == "system")
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n---\n");
                let digest = msgs.last().map(|m| m.content.as_str());
                if let Err(e) = ironclad_db::checkpoint::save_checkpoint(
                    &db,
                    &session,
                    "", // system prompt hash — placeholder until we thread it
                    &mem_summary[..mem_summary.len().min(2000)],
                    None,
                    digest,
                    turn_count as i64,
                ) {
                    tracing::warn!(error = %e, session_id = %session, "failed to save context checkpoint");
                } else {
                    tracing::debug!(session_id = %session, turn_count, "saved context checkpoint");
                }
            }
        }

        let llm = llm.read().await;
        let chunk_config = ironclad_agent::retrieval::ChunkConfig::default();
        let chunks = ironclad_agent::retrieval::chunk_text(&assistant, &chunk_config);

        for chunk in &chunks {
            if let Ok(embedding) = llm.embedding.embed_single(&chunk.text).await {
                let embed_id = uuid::Uuid::new_v4().to_string();
                ironclad_db::embeddings::store_embedding(
                    &db,
                    &embed_id,
                    "turn",
                    &session,
                    &chunk.text[..chunk.text.len().min(200)],
                    &embedding,
                )
                .inspect_err(
                    |e| tracing::warn!(error = %e, chunk_idx = chunk.index, "failed to store chunk embedding"),
                )
                .ok();
            }
        }
    });
}

#[allow(dead_code)] // all fields used by various callers (API, streaming, channel)
/// Result of the unified inference pipeline (cache check → inference → post-turn ops).
pub(super) struct PipelineResult {
    pub content: String,
    /// Model selected by routing before execution.
    pub selected_model: String,
    /// Model that actually produced the response (may differ on fallback/cache hit).
    pub model: String,
    /// When actual model differs, contains the originally selected model.
    pub model_shift_from: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: f64,
    pub react_turns: usize,
    pub latency_ms: u64,
    pub quality_score: f64,
    pub escalated: bool,
    pub cached: bool,
    pub tokens_saved: u32,
    pub assistant_message_id: String,
    /// Tool calls executed during inference: (tool_name, result_text).
    pub tool_results: Vec<(String, String)>,
}

/// Unified post-prepare pipeline used by all entry points (API, streaming, channel).
///
/// Handles: cache check → inference + ReAct → store assistant message → record cost →
/// background ingest → cache store. Callers only need to handle session setup,
/// input validation, and formatting the final response.
#[allow(clippy::too_many_arguments)] // central pipeline requires full request context
pub(super) async fn execute_inference_pipeline(
    state: &AppState,
    prepared: &PreparedInference,
    session_id: &str,
    user_content: &str,
    turn_id: &str,
    authority: InputAuthority,
    channel_label: Option<&str>,
    delegation_provenance: &mut DelegationProvenance,
) -> Result<PipelineResult, String> {
    // 1. Cache check
    let cached = check_cache(
        state,
        user_content,
        &prepared.cache_hash,
        prepared.query_embedding.as_deref(),
    )
    .await;

    if let Some(cached) = cached {
        let cached_content = enforce_execution_truth_guard(user_content, cached.content, &[]);
        let cached_content =
            enforce_model_identity_truth_guard(user_content, cached_content, &cached.model);
        let guarded_cached_content =
            enforce_non_repetition(cached_content, prepared.previous_assistant.as_deref());
        let cached_provider_prefix = cached
            .model
            .split('/')
            .next()
            .unwrap_or("unknown")
            .to_string();
        record_cost(
            state,
            &cached.model,
            &cached_provider_prefix,
            0,
            0,
            0.0,
            Some("cached"),
            true,
            Some(0),
            None,
            false,
            Some(turn_id),
        );
        let asst_id = ironclad_db::sessions::append_message(
            &state.db,
            session_id,
            "assistant",
            &guarded_cached_content,
        )
        .map_err(|e| format!("failed to store cached response: {e}"))?;
        if cached.model != prepared.model {
            state.event_bus.publish(
                serde_json::json!({
                    "type": "model_shift",
                    "turn_id": turn_id,
                    "session_id": session_id,
                    "channel": channel_label.unwrap_or("unknown"),
                    "selected_model": prepared.model,
                    "executed_model": cached.model,
                    "reason": "cache_hit",
                })
                .to_string(),
            );
        }

        return Ok(PipelineResult {
            content: guarded_cached_content,
            selected_model: prepared.model.clone(),
            model: cached.model.clone(),
            model_shift_from: if cached.model != prepared.model {
                Some(prepared.model.clone())
            } else {
                None
            },
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            react_turns: 0,
            latency_ms: 0,
            quality_score: 0.0,
            escalated: false,
            cached: true,
            tokens_saved: cached.tokens_saved,
            assistant_message_id: asst_id,
            tool_results: vec![],
        });
    }

    // 2. Inference + ReAct loop
    let inference = run_inference_and_react(
        state,
        prepared,
        turn_id,
        authority,
        channel_label,
        delegation_provenance,
    )
    .await;

    // 3. Store assistant message
    let asst_id = ironclad_db::sessions::append_message(
        &state.db,
        session_id,
        "assistant",
        &inference.content,
    )
    .map_err(|e| format!("failed to store assistant response: {e}"))?;

    // 4. Record cost
    let executed_provider_prefix = inference
        .model
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string();
    record_cost(
        state,
        &inference.model,
        &executed_provider_prefix,
        inference.tokens_in,
        inference.tokens_out,
        inference.cost,
        None,
        false,
        Some(inference.latency_ms as i64),
        Some(inference.quality_score),
        inference.escalated,
        Some(turn_id),
    );

    // 5. Post-turn ingest (spawns background task)
    post_turn_ingest(
        state,
        session_id,
        user_content,
        &inference.content,
        &inference.tool_results,
    );

    // 6. Cache store
    store_in_cache(
        state,
        &prepared.cache_hash,
        user_content,
        &inference.content,
        &inference.model,
        inference.tokens_out,
    )
    .await;

    if inference.model != prepared.model {
        state.event_bus.publish(
            serde_json::json!({
                "type": "model_shift",
                "turn_id": turn_id,
                "session_id": session_id,
                "channel": channel_label.unwrap_or("unknown"),
                "selected_model": prepared.model,
                "executed_model": inference.model,
                "reason": "fallback",
            })
            .to_string(),
        );
    }

    Ok(PipelineResult {
        content: inference.content,
        selected_model: prepared.model.clone(),
        model: inference.model.clone(),
        model_shift_from: if inference.model != prepared.model {
            Some(prepared.model.clone())
        } else {
            None
        },
        tokens_in: inference.tokens_in,
        tokens_out: inference.tokens_out,
        cost: inference.cost,
        react_turns: inference.react_turns,
        latency_ms: inference.latency_ms,
        quality_score: inference.quality_score,
        escalated: inference.escalated,
        cached: false,
        tokens_saved: 0,
        assistant_message_id: asst_id,
        tool_results: inference.tool_results,
    })
}

/// Record inference cost metrics.
#[allow(clippy::too_many_arguments)] // thin pass-through to ironclad_db::metrics
pub(super) fn record_cost(
    state: &AppState,
    model: &str,
    provider_prefix: &str,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    variant: Option<&str>,
    cached: bool,
    latency_ms: Option<i64>,
    quality_score: Option<f64>,
    escalation: bool,
    turn_id: Option<&str>,
) {
    ironclad_db::metrics::record_inference_cost(
        &state.db,
        model,
        provider_prefix,
        tokens_in,
        tokens_out,
        cost,
        variant,
        cached,
        latency_ms,
        quality_score,
        escalation,
        turn_id,
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record inference cost"))
    .ok();
}
