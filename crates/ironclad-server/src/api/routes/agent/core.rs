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
use super::guard_registry::{GuardContext, GuardId, guard_sets};
use super::guards::{is_low_value_response, is_parroting_user_prompt};
use super::intent_registry::{Intent, IntentRegistry};
use super::intents::{
    requests_capability_summary, requests_current_events, requests_obsidian_insights,
    requests_personality_profile, requests_provider_inventory,
};
use super::routing::{
    infer_with_fallback, persist_model_selection_audit, select_routed_model_with_audit,
};
use super::shortcuts::{ShortcutContext, ShortcutDispatcher};
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
    pub os_text: String,
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
    /// When true, the user is correcting/contradicting the previous reply.
    /// Shortcuts must be skipped so the expanded correction prompt reaches the LLM.
    pub is_correction_turn: bool,
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
    /// Carried from `InferenceInput` — when true, skip execution shortcuts.
    pub is_correction_turn: bool,
    /// Unified intent classification results from [`IntentRegistry::classify`].
    /// Sorted by priority (highest first). Computed once in `prepare_inference()`
    /// and threaded through shortcuts, guards, and cache bypass decisions.
    pub intents: Vec<Intent>,
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

fn deterministic_quality_fallback(user_prompt: &str, agent_name: &str) -> String {
    let lower = user_prompt.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "awesome" | "great" | "nice" | "perfect" | "go ahead"
    ) {
        return "Copy. Next concrete step: I can produce paste-ready markdown bodies for Wallet-Defaults.md, Cadence-and-Approvals.md, and Daily-Progress-Template.md right now."
            .to_string();
    }
    if requests_obsidian_insights(user_prompt)
        || lower.contains("my vault")
        || lower.contains("obsidian")
    {
        return "Obsidian vault starter scaffold:\n\nMy Vault/\n- Governance/\n  - Wallet-Defaults.md\n  - Cadence-and-Approvals.md\n- Ledger/\n  - Ledger-Skeleton.md\n- Subagents/\n  - web3-dispatcher.md\n  - api-vender.md\n  - audit-fuzzer.md\n- Data/\n  - Data-Sources.md\n  - Data-Flows.md\n- Reports/\n  - Daily-Progress-Template.md\n- Templates/\n- References/\n\nIf you want, I will now produce the first three file bodies (Wallet-Defaults, Cadence-and-Approvals, Daily-Progress-Template) in paste-ready markdown."
            .to_string();
    }
    if requests_current_events(user_prompt) {
        return format!(
            "{agent_name} here. I failed to produce a reliable live sitrep in that turn. I can still provide a concrete briefing now if you specify scope (global, US, or region) and I will return it with dated caveats."
        );
    }
    if requests_capability_summary(user_prompt) {
        return "I can execute tools for filesystem/command tasks, delegate to subagents, inspect runtime state, schedule jobs, and report outcomes with evidence from executed steps.".to_string();
    }
    if requests_personality_profile(user_prompt) {
        return format!(
            "{agent_name}: concise, direct, and execution-first. I acknowledge quickly, act with tools when needed, and avoid fabricated claims."
        );
    }
    if requests_provider_inventory(user_prompt) {
        return "I can list active provider/model routing from runtime state. Ask me for a provider inventory and I will return the current configured primary and fallback chain."
            .to_string();
    }
    format!(
        "{agent_name} here. The prior generation degraded. I am returning a concrete fallback: state the exact outcome format you want (for example: bullet summary, command output, or action plan) and I will deliver it directly."
    )
}

/// Build a retry `UnifiedRequest` for a guard that requests inference retry.
///
/// Each guard that can emit `RetryRequested` needs a specific operator directive
/// and token budget. This function encapsulates the mapping from `GuardId` to
/// the appropriate retry configuration.
fn build_retry_request(
    guard_id: GuardId,
    prepared: &PreparedInference,
) -> ironclad_llm::format::UnifiedRequest {
    let (directive, max_tokens) = match guard_id {
        GuardId::LiteraryQuoteRetry => (
            "Operator directive: Provide a brief literary quote/paraphrase response only. \
             Do not provide tactical guidance; keep it contextual and non-operational.",
            256,
        ),
        GuardId::LowValueParroting => (
            "Operator directive: your previous response was placeholder/status-only. \
             Provide a concrete, complete answer to the original user request now. \
             Do not output placeholder lines such as 'ready' or status-only acknowledgements.",
            768,
        ),
        _ => (
            "Operator directive: the previous response was filtered. Provide a concrete, \
             complete answer now.",
            512,
        ),
    };

    let mut retry_messages = prepared.request.messages.clone();
    retry_messages.push(ironclad_llm::format::UnifiedMessage {
        role: "user".into(),
        content: directive.into(),
        parts: None,
    });

    ironclad_llm::format::UnifiedRequest {
        model: prepared.request.model.clone(),
        messages: retry_messages,
        max_tokens: Some(max_tokens),
        temperature: match guard_id {
            GuardId::LowValueParroting => prepared.request.temperature,
            _ => None,
        },
        system: None,
        quality_target: match guard_id {
            GuardId::LowValueParroting => prepared.request.quality_target,
            _ => None,
        },
        tools: vec![],
    }
}

/// Apply the full guard chain to post-inference output, handling retries.
///
/// When a guard emits `RetryRequested`, this function:
/// 1. Builds a retry request with the appropriate operator directive.
/// 2. Calls `infer_with_fallback()` to re-infer.
/// 3. Sanitizes the retried output.
/// 4. Resumes the guard chain from the guard *after* the one that requested the retry.
/// 5. If the retried output still fails, uses `deterministic_quality_fallback()`.
///
/// Returns the final content and any additional token/cost deltas from retries.
#[allow(clippy::too_many_arguments)]
async fn apply_guards_with_retry(
    content: String,
    ctx: &GuardContext<'_>,
    state: &AppState,
    prepared: &PreparedInference,
    resolved_model: &mut String,
    total_in: &mut i64,
    total_out: &mut i64,
    total_cost: &mut f64,
) -> String {
    let chain = guard_sets::full();
    let result = chain.apply(content, ctx);

    match result.retry {
        None => result.content,
        Some(signal) => {
            tracing::warn!(
                guard = ?signal.guard_id,
                reason = %signal.reason,
                "guard chain requested inference retry"
            );
            let retry_req = build_retry_request(signal.guard_id, prepared);
            match infer_with_fallback(state, &retry_req, &prepared.model).await {
                Ok(retried_result) => {
                    *resolved_model = retried_result.model.clone();
                    *total_in += retried_result.tokens_in;
                    *total_out += retried_result.tokens_out;
                    *total_cost += retried_result.cost;

                    let retried =
                        sanitize_model_output(retried_result.content, state.hmac_secret.as_ref());
                    // Resume guard chain from the guard after the one that triggered retry.
                    let resumed = chain.apply_from(retried, ctx, signal.resume_index);
                    match resumed.retry {
                        None => resumed.content,
                        Some(_) => {
                            // Second retry failed — use deterministic fallback.
                            tracing::warn!(
                                "guard chain retry still failing; using deterministic fallback"
                            );
                            deterministic_quality_fallback(ctx.user_prompt, ctx.agent_name)
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "guard retry inference failed; using deterministic fallback");
                    deterministic_quality_fallback(ctx.user_prompt, ctx.agent_name)
                }
            }
        }
    }
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
    if let Err(e) = ironclad_db::sessions::update_model(&state.db, input.session_id, &model) {
        tracing::warn!(session_id = %input.session_id, model = %model, error = %e, "failed to update session model");
    }

    // Tier resolution + embedding client — single lock acquisition.
    let (tier, embedding_client) = {
        let llm = state.llm.read().await;
        let tier = llm
            .providers
            .get_by_model(&model)
            .map(|p| p.tier)
            .unwrap_or_else(|| ironclad_llm::tier::classify(&model));
        (tier, llm.embedding.clone())
    };

    // Embedding for RAG + cache L2.
    // EmbeddingClient cloned above so the LLM read lock is released before
    // this potentially I/O-bound call.
    let query_embedding = embedding_client
        .embed_single(input.user_content)
        .await
        .inspect_err(|e| {
            tracing::warn!(error = %e, "embedding generation failed, RAG retrieval will be skipped")
        })
        .ok();

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
    let system_prompt = if input.os_text.is_empty() {
        format!(
            "You are {name}, an autonomous AI agent (id: {id}). \
             When asked who you are, always identify as {name}. \
             Never reveal the underlying model name or claim to be a generic assistant.",
            name = input.agent_name,
            id = input.agent_id,
        )
    } else {
        let mut prompt = input.os_text.clone();
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

    // Instruction anti-fade: inject compact directive reminder before the user
    // message when conversation is long enough that system prompt instructions
    // may have faded from the model's attention window (OPENDEV pattern).
    if let Some(reminder) =
        ironclad_agent::prompt::build_instruction_reminder(&input.os_text, &input.firmware_text)
    {
        ironclad_agent::context::inject_instruction_reminder(&mut messages, &reminder);
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

    // Unified intent classification — evaluated exactly once per request.
    let intents = IntentRegistry::default_registry().classify(input.user_content);

    Ok(PreparedInference {
        model,
        model_for_api,
        tier,
        request,
        previous_assistant,
        query_embedding,
        cache_hash,
        is_correction_turn: input.is_correction_turn,
        intents,
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
    let (max_react_turns, max_turn_duration_seconds) = {
        let cfg = state.config.read().await;
        (
            cfg.agent.autonomy_max_react_turns,
            cfg.agent.autonomy_max_turn_duration_seconds,
        )
    };
    let user_prompt = prepared
        .request
        .messages
        .last()
        .map(|m| m.content.as_str())
        .unwrap_or_default();
    // Unified shortcut dispatch — replaces the 983-line try_execution_shortcut().
    // ShortcutDispatcher respects is_correction_turn internally (returns None).
    {
        let agent_name = {
            let cfg = state.config.read().await;
            cfg.agent.name.clone()
        };
        let registry = IntentRegistry::default_registry();
        let bypass_cache = registry.should_bypass_cache(&prepared.intents);
        let label = channel_label.unwrap_or("api");
        let mut shortcut_ctx = ShortcutContext {
            state,
            user_content: user_prompt,
            turn_id,
            intents: &prepared.intents,
            agent_name: &agent_name,
            channel_label: label,
            prepared_model: &prepared.model,
            authority,
            delegation_provenance,
            is_correction_turn: prepared.is_correction_turn,
        };
        match ShortcutDispatcher::default_dispatcher()
            .try_dispatch(&mut shortcut_ctx, bypass_cache)
            .await
        {
            Ok(Some(shortcut)) => return shortcut,
            Err(e) => {
                tracing::warn!(error = %e, "shortcut dispatch failed; falling through to inference");
            }
            Ok(None) => {} // no shortcut matched — continue to inference
        }
    }

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
    let mut react_loop = AgentLoop::new(max_react_turns);
    let mut final_content = initial_content.clone();
    let mut tool_results_acc: Vec<(String, String)> = Vec::new();
    let react_deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(max_turn_duration_seconds);

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
            if std::time::Instant::now() >= react_deadline {
                final_content = format!(
                    "I stopped this turn after reaching the autonomy duration limit ({}s). \
Please continue with a narrower or next-step command.",
                    max_turn_duration_seconds
                );
                pending_calls.clear();
                break;
            }
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

    // Post-ReAct guards — unified guard chain with retry handling.
    let agent_name = {
        let cfg = state.config.read().await;
        cfg.agent.name.clone()
    };
    // Snapshot model for guard display — retry handler may update resolved_model.
    let model_snapshot = resolved_model.clone();
    let guard_ctx = GuardContext {
        user_prompt,
        intents: &prepared.intents,
        tool_results: &tool_results_acc,
        agent_name: &agent_name,
        resolved_model: &model_snapshot,
        delegation_provenance,
        previous_assistant: prepared.previous_assistant.as_deref(),
    };
    let final_content = apply_guards_with_retry(
        final_content,
        &guard_ctx,
        state,
        prepared,
        &mut resolved_model,
        &mut total_in,
        &mut total_out,
        &mut total_cost,
    )
    .await;

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
    // NOTE: write lock required because lookup_strict() increments hit/miss stats.
    // This is fast (microseconds) so contention is low.  A future optimization
    // could wrap SemanticCache in its own Mutex for interior mutability, allowing
    // check_cache to take only a read lock on LlmService.
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
    if tokens_out > 0
        && !is_low_value_response(user_content, content)
        && !is_parroting_user_prompt(user_content, content)
    {
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
    // 1. Cache check — uses unified IntentRegistry to decide bypass.
    let registry = IntentRegistry::default_registry();
    let cached = if registry.should_bypass_cache(&prepared.intents) {
        None
    } else {
        check_cache(
            state,
            user_content,
            &prepared.cache_hash,
            prepared.query_embedding.as_deref(),
        )
        .await
    };

    if let Some(cached) = cached {
        let agent_name = {
            let cfg = state.config.read().await;
            cfg.agent.name.clone()
        };
        // Apply the cached guard set — includes SubagentClaim + LiteraryQuoteRetry
        // which were missing from the original inline guards.
        let cached_guard_ctx = GuardContext {
            user_prompt: user_content,
            intents: &prepared.intents,
            tool_results: &[],
            agent_name: &agent_name,
            resolved_model: &cached.model,
            delegation_provenance,
            previous_assistant: prepared.previous_assistant.as_deref(),
        };
        let chain = guard_sets::cached();
        let guard_result = chain.apply(cached.content, &cached_guard_ctx);

        // If any guard requests a retry or the result is empty, discard the cache
        // hit and fall through to fresh inference.
        let discard_cache = guard_result.retry.is_some()
            || guard_result.content.trim().is_empty()
            || guard_result
                .content
                .contains("filtered internal execution protocol");
        let guarded_cached_content = if discard_cache {
            deterministic_quality_fallback(user_content, &agent_name)
        } else {
            guard_result.content
        };
        if discard_cache {
            tracing::warn!("discarding cache hit after guard chain flagged content");
        } else {
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
