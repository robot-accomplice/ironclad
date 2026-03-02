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
use super::guards::{enforce_non_repetition, enforce_subagent_claim_guard};
use super::routing::{
    infer_with_fallback, persist_model_selection_audit, select_routed_model_with_audit,
};
use super::tools::{execute_tool_call, parse_tool_call};

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
    pub provider_prefix: String,
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

    let provider_prefix = model.split('/').next().unwrap_or("unknown").to_string();

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
    let system_prompt = format!(
        "{system_prompt}{}",
        ironclad_agent::prompt::runtime_metadata_block(
            env!("CARGO_PKG_VERSION"),
            &input.primary_model,
            &model,
        )
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
    };

    Ok(PreparedInference {
        model,
        model_for_api,
        provider_prefix,
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
    let (initial_content, mut total_in, mut total_out, mut total_cost, latency_ms, quality_score, escalated) =
        match infer_with_fallback(state, &prepared.request, &prepared.model).await {
            Ok(result) => (
                result.content,
                result.tokens_in,
                result.tokens_out,
                result.cost,
                result.latency_ms,
                result.quality_score,
                result.escalated,
            ),
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

    // ReAct loop
    let mut react_loop = AgentLoop::new(10);
    let mut final_content = initial_content.clone();

    if let Some((tool_name, tool_params)) = parse_tool_call(&initial_content) {
        react_loop.transition(ReactAction::Think);
        let mut react_messages = prepared.request.messages.clone();
        react_messages.push(ironclad_llm::format::UnifiedMessage {
            role: "assistant".into(),
            content: initial_content,
            parts: None,
        });

        let mut current_tool = Some((tool_name, tool_params));

        while let Some((ref tn, ref tp)) = current_tool {
            // Loop detection: break if the same tool+params repeats consecutively
            if react_loop.is_looping(tn, &tp.to_string()) {
                tracing::warn!(
                    tool = tn.as_str(),
                    "ReAct loop detected — same tool+params repeated"
                );
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
            let observation = if ironclad_agent::injection::scan_output(&observation) {
                tracing::warn!(
                    tool = tn.as_str(),
                    "tool result flagged by output scan, sanitizing"
                );
                format!("[Tool {tn} result blocked by safety filter]")
            } else {
                observation
            };

            react_loop.transition(ReactAction::Observe);
            react_messages.push(ironclad_llm::format::UnifiedMessage {
                role: "user".into(),
                content: observation,
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
            };

            let follow_content =
                match infer_with_fallback(state, &follow_req, &prepared.model).await {
                    Ok(result) => {
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

            current_tool = parse_tool_call(&follow_content);
            if current_tool.is_none() {
                react_loop.transition(ReactAction::Finish);
                final_content = follow_content;
            }
        }
    }

    // Post-ReAct guards
    let final_content = enforce_subagent_claim_guard(final_content, delegation_provenance);
    let final_content =
        enforce_non_repetition(final_content, prepared.previous_assistant.as_deref());

    InferenceOutput {
        content: final_content,
        model: prepared.model.clone(),
        tokens_in: total_in,
        tokens_out: total_out,
        cost: total_cost,
        react_turns: react_loop.turn_count,
        latency_ms,
        quality_score,
        escalated,
    }
}

/// Check the semantic cache. Returns `Some(CachedResponse)` on hit.
pub(super) async fn check_cache(
    state: &AppState,
    user_content: &str,
    cache_hash: &str,
    query_embedding: Option<&[f32]>,
) -> Option<ironclad_llm::CachedResponse> {
    let mut llm = state.llm.write().await;
    if let Some(emb) = query_embedding {
        llm.cache.lookup_with_embedding(cache_hash, emb)
    } else {
        llm.cache.lookup(cache_hash, user_content)
    }
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
) {
    let db = state.db.clone();
    let config = Arc::clone(&state.config);
    let session = session_id.to_string();
    let user = user_content.to_string();
    let assistant = assistant_content.to_string();
    let llm = Arc::clone(&state.llm);
    tokio::spawn(async move {
        ironclad_agent::memory::ingest_turn(&db, &session, &user, &assistant, &[]);

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
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record inference cost"))
    .ok();
}
