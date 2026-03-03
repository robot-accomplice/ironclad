//! Model selection, inference fallback chain, and routing audit persistence.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ironclad_core::IroncladError;
use serde::Serialize;
use serde_json::json;

use super::AppState;

#[allow(dead_code)] // model/provider reserved for future per-turn audit trails
pub(super) struct InferenceResult {
    pub content: String,
    pub model: String,
    pub provider: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: f64,
    pub latency_ms: u64,
    pub quality_score: f64,
    pub escalated: bool,
}

pub(super) struct ResolvedInferenceProvider {
    pub url: String,
    pub api_key: String,
    pub auth_header: String,
    pub extra_headers: HashMap<String, String>,
    pub format: ironclad_core::ApiFormat,
    pub cost_in: f64,
    pub cost_out: f64,
    pub is_local: bool,
    pub provider_prefix: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ModelCandidateAudit {
    pub model: String,
    pub source: String,
    pub provider_available: bool,
    pub breaker_blocked: bool,
    pub usable: bool,
    pub note: String,
    /// Metascore for this candidate (populated when metascore routing is active).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metascore: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ModelSelectionAudit {
    pub selected_model: String,
    pub strategy: String,
    pub primary_model: String,
    pub override_model: Option<String>,
    pub ordered_models: Vec<String>,
    pub candidates: Vec<ModelCandidateAudit>,
    /// Metascore breakdown for the selected model (when metascore routing was used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metascore_breakdown: Option<ironclad_llm::MetascoreBreakdown>,
    /// Complexity score \[0,1\] from feature extraction (when complexity routing active).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity_score: Option<f64>,
}

pub(super) fn summarize_user_excerpt(input: &str) -> String {
    input
        .split_whitespace()
        .take(20)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}

pub(super) async fn persist_model_selection_audit(
    state: &AppState,
    turn_id: &str,
    session_id: &str,
    channel: &str,
    complexity: Option<&str>,
    user_content: &str,
    audit: &ModelSelectionAudit,
) {
    let agent_id = {
        let cfg = state.config.read().await;
        cfg.agent.id.clone()
    };
    let row = ironclad_db::model_selection::ModelSelectionEventRow {
        id: uuid::Uuid::new_v4().to_string(),
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        agent_id,
        channel: channel.to_string(),
        selected_model: audit.selected_model.clone(),
        strategy: audit.strategy.clone(),
        primary_model: audit.primary_model.clone(),
        override_model: audit.override_model.clone(),
        complexity: complexity.map(|s| s.to_string()),
        user_excerpt: summarize_user_excerpt(user_content),
        candidates_json: serde_json::to_string(&audit.candidates).unwrap_or_else(|_| "[]".into()),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Err(e) = ironclad_db::model_selection::record_model_selection_event(&state.db, &row) {
        tracing::warn!(error = %e, turn_id, "failed to persist model selection audit");
    }
    state.event_bus.publish(
        json!({
            "type": "model_selection",
            "turn_id": turn_id,
            "session_id": session_id,
            "channel": channel,
            "selected_model": audit.selected_model,
            "strategy": audit.strategy,
            "primary_model": audit.primary_model,
            "override_model": audit.override_model,
            "complexity": complexity,
            "complexity_score": audit.complexity_score,
            "metascore_breakdown": audit.metascore_breakdown,
            "candidates": audit.candidates,
            "created_at": row.created_at,
        })
        .to_string(),
    );
}

pub(super) fn fallback_candidates(
    config: &ironclad_core::IroncladConfig,
    initial_model: &str,
) -> Vec<String> {
    fallback_candidates_with_preferred(config, initial_model, &[])
}

pub(super) fn fallback_candidates_with_preferred(
    config: &ironclad_core::IroncladConfig,
    initial_model: &str,
    preferred_fallbacks: &[String],
) -> Vec<String> {
    let mut candidates = vec![initial_model.to_string()];
    for fb in preferred_fallbacks {
        if fb != initial_model && !candidates.iter().any(|c| c == fb) {
            candidates.push(fb.clone());
        }
    }
    for fb in &config.models.fallbacks {
        if fb != initial_model && !candidates.iter().any(|c| c == fb) {
            candidates.push(fb.clone());
        }
    }
    candidates
}

pub(crate) async fn select_routed_model(state: &AppState, user_content: &str) -> String {
    select_routed_model_with_audit(state, user_content)
        .await
        .selected_model
}

pub(super) async fn select_routed_model_with_audit(
    state: &AppState,
    user_content: &str,
) -> ModelSelectionAudit {
    let config = state.config.read().await;
    let primary = config.models.primary.clone();
    let routing_mode = config.models.routing.mode.clone();
    let cost_aware = config.models.routing.cost_aware;
    let mut ordered_models = vec![primary.clone()];
    for fb in &config.models.fallbacks {
        if !fb.is_empty() && !ordered_models.iter().any(|m| m == fb) {
            ordered_models.push(fb.clone());
        }
    }
    drop(config);

    let llm_read = state.llm.read().await;

    let evaluate = |model: &str, source: &str| {
        let provider_available = llm_read.providers.get_by_model(model).is_some();
        let provider_prefix = model.split('/').next().unwrap_or("unknown");
        let breaker_blocked = llm_read.breakers.is_blocked(provider_prefix);
        let usable = provider_available && !breaker_blocked;
        let note = if usable {
            "usable".to_string()
        } else if !provider_available {
            "no provider configured for model".to_string()
        } else {
            "provider breaker open".to_string()
        };
        ModelCandidateAudit {
            model: model.to_string(),
            source: source.to_string(),
            provider_available,
            breaker_blocked,
            usable,
            note,
            metascore: None,
        }
    };
    let mut candidates = Vec::new();

    // Phase 1: Override takes absolute priority.
    if let Some(ovr) = llm_read.router.get_override() {
        let c = evaluate(ovr, "override");
        let usable = c.usable;
        candidates.push(c);
        if usable {
            return ModelSelectionAudit {
                selected_model: ovr.to_string(),
                strategy: "override_usable".to_string(),
                primary_model: primary,
                override_model: Some(ovr.to_string()),
                ordered_models,
                candidates,
                metascore_breakdown: None,
                complexity_score: None,
            };
        }
        tracing::warn!(
            model = ovr,
            "configured override is not usable (missing provider or breaker open), falling back"
        );
    }

    // Phase 2: Metascore routing (2.19).
    // Build per-model profiles from current system state, score with metascore,
    // and select the highest-scoring candidate.
    if routing_mode != "primary" {
        let features = ironclad_llm::extract_features(user_content, 0, 0);
        let complexity = ironclad_llm::classify_complexity(&features);

        let profiles = ironclad_llm::build_model_profiles(
            &llm_read.router,
            &llm_read.providers,
            &llm_read.quality,
            &llm_read.capacity,
            &llm_read.breakers,
        );

        // Tiered feedback: adjust local/cloud preference from observed escalation behavior.
        // If local acceptance is low, penalize local candidates and favor cloud candidates.
        // If local acceptance is high, bias in the opposite direction.
        let local_total = llm_read.escalation.local_accepted + llm_read.escalation.local_escalated;
        let local_acceptance = llm_read.escalation.local_acceptance_rate();
        let escalation_bias = if local_total >= 5 {
            // Map [0,1] acceptance -> [-0.10, +0.10] score delta.
            ((local_acceptance - 0.5) * 0.2).clamp(-0.10, 0.10)
        } else {
            0.0
        };

        // Build audit entries for all profiled candidates.
        let mut best_selection: Option<(String, ironclad_llm::MetascoreBreakdown, f64)> = None;
        for profile in &profiles {
            let mut breakdown = profile.metascore(complexity, cost_aware);
            if escalation_bias != 0.0 {
                let delta = if profile.is_local {
                    escalation_bias
                } else {
                    -escalation_bias
                };
                breakdown.final_score = (breakdown.final_score + delta).clamp(0.0, 1.0);
            }
            let mut c = evaluate(&profile.model_name, "metascore_candidate");
            c.metascore = Some(breakdown.final_score);
            candidates.push(c);

            match &best_selection {
                Some((_, _, best_score)) if breakdown.final_score <= *best_score => {}
                _ => {
                    best_selection = Some((
                        profile.model_name.clone(),
                        breakdown.clone(),
                        breakdown.final_score,
                    ));
                }
            }
        }

        if let Some((selected, breakdown, _)) = best_selection {
            let strategy = format!(
                "metascore_{:.3}_c{complexity:.2}{}{}",
                breakdown.final_score,
                if cost_aware { "_cost" } else { "" },
                if escalation_bias != 0.0 { "_esc" } else { "" }
            );
            tracing::debug!(
                model = selected.as_str(),
                complexity,
                cost_aware,
                metascore = breakdown.final_score,
                efficacy = breakdown.efficacy,
                cost_score = breakdown.cost,
                availability = breakdown.availability,
                locality = breakdown.locality,
                confidence = breakdown.confidence,
                escalation_bias,
                local_acceptance,
                local_total,
                "metascore routing selected model"
            );
            return ModelSelectionAudit {
                selected_model: selected,
                strategy,
                primary_model: primary,
                override_model: llm_read.router.get_override().map(|s| s.to_string()),
                ordered_models,
                candidates,
                metascore_breakdown: Some(breakdown),
                complexity_score: Some(complexity),
            };
        }
        tracing::debug!(
            complexity,
            "metascore returned no candidates, falling back to ordered iteration"
        );
    }

    // Phase 3: Availability-first fallback — iterate ordered models.
    for (idx, model) in ordered_models.iter().enumerate() {
        let mut c = evaluate(
            model,
            if idx == 0 {
                "primary_ordered"
            } else {
                "fallback_ordered"
            },
        );
        c.metascore = None;
        let usable = c.usable;
        candidates.push(c);
        if usable {
            return ModelSelectionAudit {
                selected_model: model.clone(),
                strategy: if idx == 0 {
                    "primary_usable".to_string()
                } else {
                    format!("fallback_usable_{idx}")
                },
                primary_model: primary,
                override_model: llm_read.router.get_override().map(|s| s.to_string()),
                ordered_models,
                candidates,
                metascore_breakdown: None,
                complexity_score: None,
            };
        }
    }

    // Last resort: return configured primary even if provider is degraded/unavailable.
    ModelSelectionAudit {
        selected_model: primary.clone(),
        strategy: "last_resort_primary".to_string(),
        primary_model: primary,
        override_model: llm_read.router.get_override().map(|s| s.to_string()),
        ordered_models,
        candidates,
        metascore_breakdown: None,
        complexity_score: None,
    }
}

pub(super) async fn resolve_inference_provider(
    state: &AppState,
    model: &str,
) -> Option<ResolvedInferenceProvider> {
    let llm = state.llm.read().await;
    let provider = llm.providers.get_by_model(model)?;
    let url = format!("{}{}", provider.url, provider.chat_path);
    let key = super::super::admin::resolve_provider_key(
        &provider.name,
        provider.is_local,
        &provider.auth_mode,
        provider.api_key_ref.as_deref(),
        &provider.api_key_env,
        &state.oauth,
        &state.keystore,
    )
    .await
    .unwrap_or_else(|| {
        if !provider.is_local {
            tracing::warn!(provider = %provider.name, "API key resolved to None for non-local provider");
        }
        String::new()
    });
    Some(ResolvedInferenceProvider {
        url,
        api_key: key,
        auth_header: provider.auth_header.clone(),
        extra_headers: provider.extra_headers.clone(),
        format: provider.format,
        cost_in: provider.cost_per_input_token,
        cost_out: provider.cost_per_output_token,
        is_local: provider.is_local,
        provider_prefix: model.split('/').next().unwrap_or("unknown").to_string(),
    })
}

pub(super) fn estimate_cost_from_provider(
    in_rate: f64,
    out_rate: f64,
    tokens_in: i64,
    tokens_out: i64,
) -> f64 {
    tokens_in as f64 * in_rate + tokens_out as f64 * out_rate
}

#[derive(Debug, Clone, Copy)]
pub(super) struct InferenceBudget {
    pub max_fallback_attempts: usize,
    pub max_total_inference_time: Duration,
    pub per_provider_timeout: Duration,
}

pub(super) const INTERACTIVE_INFERENCE_BUDGET: InferenceBudget = InferenceBudget {
    max_fallback_attempts: 4,
    max_total_inference_time: Duration::from_secs(45),
    per_provider_timeout: Duration::from_secs(25),
};

pub(super) const DELEGATED_INFERENCE_BUDGET: InferenceBudget = InferenceBudget {
    max_fallback_attempts: 5,
    max_total_inference_time: Duration::from_secs(80),
    per_provider_timeout: Duration::from_secs(20),
};

/// Attempt inference on the selected model, falling back through the configured
/// chain on transient errors. Updates circuit breakers on success/failure.
///
/// When tiered inference is enabled, local model responses are evaluated for
/// confidence. If the response falls below the confidence floor and the
/// latency budget allows, inference escalates to the next (cloud) candidate.
/// The low-confidence local response is preserved as a fallback in case all
/// cloud candidates fail.
pub(super) async fn infer_with_fallback(
    state: &AppState,
    unified_req: &ironclad_llm::format::UnifiedRequest,
    initial_model: &str,
) -> Result<InferenceResult, String> {
    infer_with_fallback_with_budget(
        state,
        unified_req,
        initial_model,
        INTERACTIVE_INFERENCE_BUDGET,
    )
    .await
}

pub(super) async fn infer_with_fallback_with_budget(
    state: &AppState,
    unified_req: &ironclad_llm::format::UnifiedRequest,
    initial_model: &str,
    budget: InferenceBudget,
) -> Result<InferenceResult, String> {
    infer_with_fallback_with_budget_and_preferred(state, unified_req, initial_model, budget, &[])
        .await
}

pub(super) async fn infer_with_fallback_with_budget_and_preferred(
    state: &AppState,
    unified_req: &ironclad_llm::format::UnifiedRequest,
    initial_model: &str,
    budget: InferenceBudget,
    preferred_fallbacks: &[String],
) -> Result<InferenceResult, String> {
    let config = state.config.read().await;
    let candidates =
        fallback_candidates_with_preferred(&config, initial_model, preferred_fallbacks);
    let tiered_enabled = config.models.tiered_inference.enabled;
    let confidence_floor = config.models.tiered_inference.confidence_floor;
    let escalation_budget_ms = config.models.tiered_inference.escalation_latency_budget_ms;
    drop(config);

    // Use the shared ConfidenceEvaluator from LlmService rather than constructing a local copy.
    let confidence_eval = {
        let llm = state.llm.read().await;
        llm.confidence.clone()
    };
    let mut fallback_result: Option<InferenceResult> = None;
    let mut last_error = String::new();
    let infer_started = Instant::now();

    for (attempt_idx, model) in candidates.iter().enumerate() {
        if attempt_idx >= budget.max_fallback_attempts {
            tracing::warn!(
                attempted = attempt_idx,
                cap = budget.max_fallback_attempts,
                "fallback attempt budget exhausted"
            );
            last_error = format!(
                "fallback attempt budget exhausted ({})",
                budget.max_fallback_attempts
            );
            break;
        }
        let elapsed = infer_started.elapsed();
        if elapsed >= budget.max_total_inference_time {
            tracing::warn!(
                elapsed_ms = elapsed.as_millis() as u64,
                cap_ms = budget.max_total_inference_time.as_millis() as u64,
                "total inference timeout reached"
            );
            last_error = format!(
                "inference timeout after {}s",
                budget.max_total_inference_time.as_secs()
            );
            break;
        }
        let remaining_budget = budget.max_total_inference_time.saturating_sub(elapsed);
        if remaining_budget.is_zero() {
            last_error = format!(
                "inference timeout after {}s",
                budget.max_total_inference_time.as_secs()
            );
            break;
        }

        // Skip if circuit breaker is open
        {
            let llm = state.llm.read().await;
            let provider_prefix = model.split('/').next().unwrap_or("unknown");
            if llm.breakers.is_blocked(provider_prefix) {
                tracing::debug!(model, "skipping model — circuit breaker open");
                last_error = format!("{provider_prefix} circuit breaker open");
                continue;
            }
        }

        let Some(resolved) = resolve_inference_provider(state, model).await else {
            tracing::debug!(model, "no provider found, skipping");
            last_error = format!("no provider configured for {model}");
            continue;
        };

        if !resolved.is_local && resolved.api_key.is_empty() {
            tracing::debug!(model, "skipping cloud provider — no API key configured");
            last_error = format!("no API key for {}", resolved.provider_prefix);
            continue;
        }

        let model_for_api = model
            .split_once('/')
            .map(|(_, m)| m)
            .unwrap_or(model)
            .to_string();
        let mut req_clone = unified_req.clone();
        // Ensure the request targets this model's API name
        if !req_clone.model.is_empty() {
            req_clone.model = model_for_api;
        }

        let llm_body = ironclad_llm::format::translate_request(&req_clone, resolved.format)
            .unwrap_or_else(|_| serde_json::json!({}));

        let inference_start = std::time::Instant::now();
        let llm = state.llm.read().await;
        let attempt_timeout = std::cmp::min(budget.per_provider_timeout, remaining_budget);
        let result = match tokio::time::timeout(
            attempt_timeout,
            llm.client.forward_with_provider(
                &resolved.url,
                &resolved.api_key,
                llm_body,
                &resolved.auth_header,
                &resolved.extra_headers,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(IroncladError::Network(format!(
                "request failed: timeout after {}s",
                attempt_timeout.as_secs()
            ))),
        };
        drop(llm);

        match result {
            Ok(resp) => {
                let unified_resp = ironclad_llm::format::translate_response(&resp, resolved.format)
                    .unwrap_or_else(|_| ironclad_llm::format::UnifiedResponse {
                        content: "(no response)".into(),
                        model: model.clone(),
                        tokens_in: 0,
                        tokens_out: 0,
                        finish_reason: None,
                    });
                let tin = unified_resp.tokens_in as i64;
                let tout = unified_resp.tokens_out as i64;
                let cost =
                    estimate_cost_from_provider(resolved.cost_in, resolved.cost_out, tin, tout);
                let total_tokens = tin.max(0) as u64 + tout.max(0) as u64;

                // Evaluate confidence for ALL models (feeds quality tracker).
                // Escalation is only considered for local models when tiered inference is on.
                let latency_ms = inference_start.elapsed().as_millis() as u64;
                let confidence = confidence_eval.evaluate(&unified_resp.content, latency_ms);
                let should_escalate = tiered_enabled
                    && resolved.is_local
                    && confidence < confidence_floor
                    && latency_ms < escalation_budget_ms;

                // Quality score: use actual confidence rather than binary 0.5/1.0.
                let quality_score = if unified_resp.content.trim().is_empty() {
                    0.0
                } else {
                    confidence
                };

                // Single write lock for all recording: breakers, capacity, quality, escalation.
                let mut llm = state.llm.write().await;
                llm.breakers.record_success(&resolved.provider_prefix);
                llm.capacity.record(&resolved.provider_prefix, total_tokens);
                let pressured = llm.capacity.is_sustained_hot(&resolved.provider_prefix);
                llm.breakers
                    .set_capacity_pressure(&resolved.provider_prefix, pressured);
                llm.quality.record(model, quality_score);
                if tiered_enabled {
                    if resolved.is_local {
                        llm.escalation
                            .record(ironclad_llm::InferenceTier::Local, should_escalate);
                    } else {
                        llm.escalation
                            .record(ironclad_llm::InferenceTier::Cloud, false);
                    }
                }
                drop(llm);

                // Tiered inference: escalate low-confidence local responses to cloud.
                if should_escalate {
                    tracing::info!(
                        model,
                        confidence,
                        latency_ms,
                        floor = confidence_floor,
                        "local response below confidence floor, escalating to next candidate"
                    );
                    fallback_result = Some(InferenceResult {
                        content: unified_resp.content,
                        model: model.clone(),
                        provider: resolved.provider_prefix,
                        tokens_in: tin,
                        tokens_out: tout,
                        cost,
                        latency_ms,
                        quality_score,
                        escalated: true,
                    });
                    last_error =
                        format!("confidence {confidence:.2} below floor {confidence_floor:.2}");
                    continue;
                }

                if model != initial_model {
                    tracing::info!(
                        initial_model = initial_model,
                        fallback = model.as_str(),
                        "initial model failed, succeeded on fallback"
                    );
                }

                return Ok(InferenceResult {
                    content: unified_resp.content,
                    model: model.clone(),
                    provider: resolved.provider_prefix,
                    tokens_in: tin,
                    tokens_out: tout,
                    cost,
                    latency_ms,
                    quality_score,
                    escalated: false,
                });
            }
            Err(e) => {
                let is_credit = e.is_credit_error();
                tracing::warn!(
                    model,
                    error = %e,
                    is_credit,
                    "inference failed, trying next fallback"
                );
                let mut llm = state.llm.write().await;
                if is_credit {
                    llm.breakers.record_credit_error(&resolved.provider_prefix);
                } else {
                    llm.breakers.record_failure(&resolved.provider_prefix);
                }
                llm.breakers
                    .set_capacity_pressure(&resolved.provider_prefix, false);
                // Record quality failure — transient errors still count against the model.
                llm.quality.record(model, 0.0);
                drop(llm);
                last_error = e.to_string();
            }
        }
    }

    // Prefer low-confidence local response over total failure.
    if let Some(fallback) = fallback_result {
        tracing::info!(
            model = fallback.model.as_str(),
            "all escalation candidates failed, returning local fallback"
        );
        return Ok(fallback);
    }

    Err(last_error)
}

pub(crate) async fn infer_content_with_fallback(
    state: &AppState,
    unified_req: &ironclad_llm::format::UnifiedRequest,
    initial_model: &str,
) -> Result<String, String> {
    infer_with_fallback(state, unified_req, initial_model)
        .await
        .map(|r| r.content)
}
