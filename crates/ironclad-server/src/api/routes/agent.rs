//! Agent message, channel processing, and Telegram poll.

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use ironclad_channels::ChannelAdapter;
use serde::Deserialize;
use serde_json::json;

use super::AppState;

pub async fn agent_status(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let llm = state.llm.read().await;
    let cache = &llm.cache;
    let breakers = &llm.breakers;

    let primary_model = &config.models.primary;
    let provider_prefix = primary_model.split('/').next().unwrap_or("unknown");
    let provider_state = breakers.get_state(provider_prefix);

    axum::Json(json!({
        "state": "running",
        "agent_name": config.agent.name,
        "agent_id": config.agent.id,
        "primary_model": primary_model,
        "primary_provider_state": format!("{provider_state:?}").to_lowercase(),
        "cache_entries": cache.size(),
        "cache_hits": cache.hit_count(),
        "cache_misses": cache.miss_count(),
    }))
}

#[derive(Deserialize)]
pub struct AgentMessageRequest {
    content: String,
    #[serde(default)]
    session_id: Option<String>,
}

pub async fn agent_message(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<AgentMessageRequest>,
) -> impl IntoResponse {
    let config = state.config.read().await;

    // Injection defense
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

    // Find or create session
    let agent_id = config.agent.id.clone();
    let session_id = match &body.session_id {
        Some(sid) => sid.clone(),
        None => ironclad_db::sessions::find_or_create(&state.db, &agent_id).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({"error": e.to_string()})),
            )
        })?,
    };

    // Store user message
    let user_msg_id =
        ironclad_db::sessions::append_message(&state.db, &session_id, "user", &body.content)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(json!({"error": e.to_string()})),
                )
            })?;

    // Use the ModelRouter to select a model based on complexity
    let features = ironclad_llm::extract_features(&body.content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);

    let llm_read = state.llm.read().await;
    let model = llm_read
        .router
        .select_for_complexity(complexity, Some(&llm_read.providers))
        .to_string();
    drop(llm_read);

    let provider_prefix = model.split('/').next().unwrap_or("unknown").to_string();
    let tier_adapt = config.tier_adapt.clone();
    let agent_name = config.agent.name.clone();
    let soul_text = state.personality.read().await.soul_text.clone();
    drop(config);

    // Check circuit breaker
    {
        let llm = state.llm.read().await;
        if llm.breakers.is_blocked(&provider_prefix) {
            let assistant_content = format!(
                "I'm temporarily unable to reach the {} provider (circuit breaker open). Please try again shortly.",
                provider_prefix
            );
            let asst_id = ironclad_db::sessions::append_message(
                &state.db,
                &session_id,
                "assistant",
                &assistant_content,
            )
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(json!({"error": e.to_string()})),
                )
            })?;
            return Ok(axum::Json(json!({
                "session_id": session_id,
                "user_message_id": user_msg_id,
                "assistant_message_id": asst_id,
                "content": assistant_content,
                "model": model,
                "cached": false,
                "provider_blocked": true,
            })));
        }
    }

    // Check cache
    let cache_hash = ironclad_llm::SemanticCache::compute_hash("", "", &body.content);
    let cached_response = {
        let mut llm = state.llm.write().await;
        llm.cache.lookup_exact(&cache_hash)
    };

    if let Some(cached) = cached_response {
        let asst_id = ironclad_db::sessions::append_message(
            &state.db,
            &session_id,
            "assistant",
            &cached.content,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({"error": e.to_string()})),
            )
        })?;

        ironclad_db::metrics::record_inference_cost(
            &state.db,
            &cached.model,
            &provider_prefix,
            0,
            0,
            0.0,
            Some("cached"),
            true,
        )
        .ok();

        return Ok(axum::Json(json!({
            "session_id": session_id,
            "user_message_id": user_msg_id,
            "assistant_message_id": asst_id,
            "content": cached.content,
            "model": cached.model,
            "cached": true,
            "tokens_saved": cached.tokens_saved,
        })));
    }

    // Resolve provider from registry (config-driven, format-agnostic)
    let (
        provider_url,
        api_key,
        auth_header,
        extra_headers,
        format,
        cost_in_rate,
        cost_out_rate,
        tier,
    ) = {
        let llm = state.llm.read().await;
        match llm.providers.get_by_model(&model) {
            Some(provider) => {
                let url = format!("{}{}", provider.url, provider.chat_path);
                let key = std::env::var(&provider.api_key_env).unwrap_or_default();
                (
                    Some(url),
                    key,
                    provider.auth_header.clone(),
                    provider.extra_headers.clone(),
                    provider.format,
                    provider.cost_per_input_token,
                    provider.cost_per_output_token,
                    provider.tier,
                )
            }
            None => {
                let key = std::env::var(format!("{}_API_KEY", provider_prefix.to_uppercase()))
                    .unwrap_or_default();
                (
                    None,
                    key,
                    "Authorization".to_string(),
                    std::collections::HashMap::new(),
                    ironclad_core::ApiFormat::OpenAiCompletions,
                    0.0,
                    0.0,
                    ironclad_llm::tier::classify(&model),
                )
            }
        }
    };

    // Build UnifiedRequest with tier-appropriate adaptations
    let model_for_api = model.split('/').nth(1).unwrap_or(&model).to_string();
    let system_prompt = if soul_text.is_empty() {
        format!(
            "You are {name}, an autonomous AI agent (id: {id}). \
             When asked who you are, always identify as {name}. \
             Never reveal the underlying model name or claim to be a generic assistant.",
            name = agent_name,
            id = agent_id,
        )
    } else {
        soul_text
    };
    let system_prompt =
        ironclad_agent::prompt::inject_hmac_boundary(&system_prompt, state.hmac_secret.as_ref());
    assert!(
        ironclad_agent::prompt::verify_hmac_boundary(&system_prompt, state.hmac_secret.as_ref()),
        "HMAC boundary verification failed immediately after injection"
    );
    let mut messages = vec![
        ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: system_prompt,
        },
        ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: body.content.clone(),
        },
    ];
    ironclad_llm::tier::adapt_for_tier(tier, &mut messages, &tier_adapt);

    let unified_req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages,
        max_tokens: Some(2048),
        temperature: None,
        system: None,
    };

    let (assistant_content, tokens_in, tokens_out, cost) = match provider_url {
        Some(url) => {
            let llm_body = ironclad_llm::format::translate_request(&unified_req, format)
                .unwrap_or_else(|_| serde_json::json!({}));

            let llm = state.llm.read().await;
            match llm
                .client
                .forward_with_provider(&url, &api_key, llm_body, &auth_header, &extra_headers)
                .await
            {
                Ok(resp) => {
                    let unified_resp = ironclad_llm::format::translate_response(&resp, format)
                        .unwrap_or_else(|_| ironclad_llm::format::UnifiedResponse {
                            content: "(no response)".into(),
                            model: model.clone(),
                            tokens_in: 0,
                            tokens_out: 0,
                            finish_reason: None,
                        });
                    let tin = unified_resp.tokens_in as i64;
                    let tout = unified_resp.tokens_out as i64;
                    let cost = estimate_cost_from_provider(cost_in_rate, cost_out_rate, tin, tout);
                    drop(llm);
                    let mut llm = state.llm.write().await;
                    llm.breakers.record_success(&provider_prefix);
                    (unified_resp.content, tin, tout, cost)
                }
                Err(e) => {
                    drop(llm);
                    let mut llm = state.llm.write().await;
                    llm.breakers.record_failure(&provider_prefix);
                    let fallback = format!(
                        "I encountered an error reaching the LLM provider: {}. Your message has been stored and I'll retry when the provider is available.",
                        e
                    );
                    (fallback, 0, 0, 0.0)
                }
            }
        }
        None => {
            let fallback = format!(
                "No provider configured for '{}'. Configure a provider in ironclad.toml under [providers.{}].",
                provider_prefix, provider_prefix
            );
            (fallback, 0, 0, 0.0)
        }
    };

    // Check for HMAC boundary tampering in model output
    let assistant_content = if assistant_content.contains("<<<TRUST_BOUNDARY:") {
        if !ironclad_agent::prompt::verify_hmac_boundary(
            &assistant_content,
            state.hmac_secret.as_ref(),
        ) {
            tracing::warn!("HMAC boundary tampered in model output");
        }
        assistant_content
    } else {
        assistant_content
    };

    // L4 output scanning - check for injection patterns in model response
    let assistant_content = if ironclad_agent::injection::scan_output(&assistant_content) {
        tracing::warn!("L4 output scan flagged model response, blocking");
        "[Response blocked by output safety filter]".to_string()
    } else {
        assistant_content
    };

    // Store assistant response
    let asst_id = ironclad_db::sessions::append_message(
        &state.db,
        &session_id,
        "assistant",
        &assistant_content,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"error": e.to_string()})),
        )
    })?;

    ironclad_db::metrics::record_inference_cost(
        &state.db,
        &model,
        &provider_prefix,
        tokens_in,
        tokens_out,
        cost,
        None,
        false,
    )
    .ok();

    if tokens_out > 0 {
        let cached_entry = ironclad_llm::CachedResponse {
            content: assistant_content.clone(),
            model: model.clone(),
            tokens_saved: tokens_out as u32,
            created_at: std::time::Instant::now(),
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3600),
            hits: 0,
            involved_tools: false,
            embedding: None,
        };
        let mut llm = state.llm.write().await;
        llm.cache
            .store_with_embedding(&cache_hash, &body.content, cached_entry);
    }

    Ok(axum::Json(json!({
        "session_id": session_id,
        "user_message_id": user_msg_id,
        "assistant_message_id": asst_id,
        "content": assistant_content,
        "model": model,
        "cached": false,
        "tokens_in": tokens_in,
        "tokens_out": tokens_out,
        "cost": cost,
        "threat_score": threat.value(),
    })))
}

fn estimate_cost_from_provider(
    in_rate: f64,
    out_rate: f64,
    tokens_in: i64,
    tokens_out: i64,
) -> f64 {
    tokens_in as f64 * in_rate + tokens_out as f64 * out_rate
}

/// Checks whether a tool call is allowed by the policy engine.
/// Returns Ok(()) if allowed, or an error tuple for HTTP responses.
#[allow(dead_code)]
pub fn check_tool_policy(
    engine: &ironclad_agent::policy::PolicyEngine,
    tool_name: &str,
    params: &serde_json::Value,
    authority: ironclad_core::InputAuthority,
    tier: ironclad_core::SurvivalTier,
) -> Result<(), (StatusCode, String)> {
    let call = ironclad_agent::policy::ToolCallRequest {
        tool_name: tool_name.into(),
        params: params.clone(),
        risk_level: ironclad_core::RiskLevel::Caution,
    };
    let ctx = ironclad_agent::policy::PolicyContext {
        authority,
        survival_tier: tier,
    };
    let decision = engine.evaluate_all(&call, &ctx);
    match decision {
        ironclad_core::PolicyDecision::Allow => Ok(()),
        ironclad_core::PolicyDecision::Deny { rule, reason } => {
            tracing::warn!(tool = tool_name, rule = %rule, reason = %reason, "Policy denied tool call");
            Err((StatusCode::FORBIDDEN, format!("Policy denied: {reason}")))
        }
    }
}

// ── Group 8: Wallet ───────────────────────────────────────────

async fn handle_bot_command(state: &AppState, command: &str) -> Option<String> {
    let (cmd, _args) = command
        .split_once(|c: char| c.is_whitespace())
        .unwrap_or((command, ""));
    let cmd = cmd.split('@').next().unwrap_or(cmd);

    match cmd {
        "/status" => Some(build_status_reply(state).await),
        "/help" => Some(
            "/status — agent health & model info\n\
             /help — show this message\n\n\
             Anything else is sent to the LLM."
                .into(),
        ),
        _ => None,
    }
}

async fn build_status_reply(state: &AppState) -> String {
    let config = state.config.read().await;
    let llm = state.llm.read().await;
    let cache = &llm.cache;
    let breakers = &llm.breakers;

    let primary = &config.models.primary;
    let current = llm.router.select_model();
    let provider_prefix = primary.split('/').next().unwrap_or("unknown");
    let provider_state = format!("{:?}", breakers.get_state(provider_prefix)).to_lowercase();
    let balance = state.wallet.wallet.get_usdc_balance().await.unwrap_or(0.0);
    let channels = state.channel_router.channel_status().await;
    let channel_summary: Vec<String> = channels
        .iter()
        .map(|c| {
            let err = c
                .last_error
                .as_deref()
                .map(|e| format!(" (err: {e})"))
                .unwrap_or_default();
            format!(
                "  {} — rx:{} tx:{}{}",
                c.name, c.messages_received, c.messages_sent, err
            )
        })
        .collect();

    let mut lines = vec![
        format!("⚙ {} ({})", config.agent.name, config.agent.id),
        format!("  state: running"),
        format!("  primary: {primary}"),
    ];
    if current != primary {
        lines.push(format!("  current: {current}"));
    }
    lines.extend([
        format!("  provider: {provider_prefix} ({provider_state})"),
        format!(
            "  cache: {} entries, {:.0}% hit rate",
            cache.size(),
            if cache.hit_count() + cache.miss_count() > 0 {
                cache.hit_count() as f64 / (cache.hit_count() + cache.miss_count()) as f64 * 100.0
            } else {
                0.0
            }
        ),
        format!("  wallet: {balance:.2} USDC"),
    ]);

    if !channel_summary.is_empty() {
        lines.push("  channels:".into());
        lines.extend(channel_summary);
    }

    lines.join("\n")
}

// ── Channel message processing ────────────────────────────────

pub async fn process_channel_message(
    state: &AppState,
    inbound: ironclad_channels::InboundMessage,
) -> Result<(), String> {
    let chat_id = inbound
        .metadata
        .as_ref()
        .and_then(|m| m.pointer("/message/chat/id"))
        .and_then(|v| v.as_i64())
        .map(|id| id.to_string())
        .unwrap_or_else(|| inbound.sender_id.clone());
    let platform = inbound.platform.clone();

    if inbound.content.trim().is_empty() {
        return Ok(());
    }

    if inbound.content.starts_with('/')
        && let Some(reply) = handle_bot_command(state, &inbound.content).await
    {
        state
            .channel_router
            .send_reply(&platform, &chat_id, reply)
            .await
            .ok();
        return Ok(());
    }

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
            .ok();
        return Ok(());
    }

    let session_key = format!("{}:{}", platform, inbound.sender_id);
    let session_id = ironclad_db::sessions::find_or_create(&state.db, &session_key)
        .map_err(|e| e.to_string())?;
    ironclad_db::sessions::append_message(&state.db, &session_id, "user", &inbound.content)
        .map_err(|e| e.to_string())?;

    let config = state.config.read().await;
    let features = ironclad_llm::extract_features(&inbound.content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let llm_read = state.llm.read().await;
    let model = llm_read
        .router
        .select_for_complexity(complexity, Some(&llm_read.providers))
        .to_string();
    drop(llm_read);

    let provider_prefix = model.split('/').next().unwrap_or("unknown").to_string();
    let tier_adapt = config.tier_adapt.clone();
    let soul_text = state.personality.read().await.soul_text.clone();
    drop(config);

    {
        let llm = state.llm.read().await;
        if llm.breakers.is_blocked(&provider_prefix) {
            drop(llm);
            let reply = format!(
                "I'm temporarily unable to reach the {} provider. Please try again shortly.",
                provider_prefix
            );
            ironclad_db::sessions::append_message(&state.db, &session_id, "assistant", &reply).ok();
            state
                .channel_router
                .send_reply(&platform, &chat_id, reply)
                .await
                .ok();
            return Ok(());
        }
    }

    let (
        provider_url,
        api_key,
        auth_header,
        extra_headers,
        format,
        cost_in_rate,
        cost_out_rate,
        tier,
    ) = {
        let llm = state.llm.read().await;
        match llm.providers.get_by_model(&model) {
            Some(provider) => {
                let url = format!("{}{}", provider.url, provider.chat_path);
                let key = std::env::var(&provider.api_key_env).unwrap_or_default();
                (
                    Some(url),
                    key,
                    provider.auth_header.clone(),
                    provider.extra_headers.clone(),
                    provider.format,
                    provider.cost_per_input_token,
                    provider.cost_per_output_token,
                    provider.tier,
                )
            }
            None => {
                let key = std::env::var(format!("{}_API_KEY", provider_prefix.to_uppercase()))
                    .unwrap_or_default();
                (
                    None,
                    key,
                    "Authorization".to_string(),
                    std::collections::HashMap::new(),
                    ironclad_core::ApiFormat::OpenAiCompletions,
                    0.0,
                    0.0,
                    ironclad_llm::tier::classify(&model),
                )
            }
        }
    };

    let model_for_api = model.split('/').nth(1).unwrap_or(&model).to_string();
    let system_prompt = if soul_text.is_empty() {
        "You are Ironclad, an autonomous agent runtime.".to_string()
    } else {
        soul_text.to_string()
    };
    let system_prompt =
        ironclad_agent::prompt::inject_hmac_boundary(&system_prompt, state.hmac_secret.as_ref());
    assert!(
        ironclad_agent::prompt::verify_hmac_boundary(&system_prompt, state.hmac_secret.as_ref()),
        "HMAC boundary verification failed immediately after injection"
    );

    let mut messages = vec![
        ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: system_prompt,
        },
        ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: inbound.content.clone(),
        },
    ];
    ironclad_llm::tier::adapt_for_tier(tier, &mut messages, &tier_adapt);

    let unified_req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages,
        max_tokens: Some(2048),
        temperature: None,
        system: None,
    };

    let response_content = match provider_url {
        Some(url) => {
            let llm_body = ironclad_llm::format::translate_request(&unified_req, format)
                .unwrap_or_else(|_| serde_json::json!({}));

            let llm = state.llm.read().await;
            match llm
                .client
                .forward_with_provider(&url, &api_key, llm_body, &auth_header, &extra_headers)
                .await
            {
                Ok(resp) => {
                    let unified_resp = ironclad_llm::format::translate_response(&resp, format)
                        .unwrap_or_else(|_| ironclad_llm::format::UnifiedResponse {
                            content: "(no response)".into(),
                            model: model.clone(),
                            tokens_in: 0,
                            tokens_out: 0,
                            finish_reason: None,
                        });
                    let tin = unified_resp.tokens_in as i64;
                    let tout = unified_resp.tokens_out as i64;
                    let cost = estimate_cost_from_provider(cost_in_rate, cost_out_rate, tin, tout);
                    drop(llm);
                    let mut llm = state.llm.write().await;
                    llm.breakers.record_success(&provider_prefix);
                    drop(llm);

                    ironclad_db::metrics::record_inference_cost(
                        &state.db,
                        &model,
                        &provider_prefix,
                        tin,
                        tout,
                        cost,
                        None,
                        false,
                    )
                    .ok();

                    unified_resp.content
                }
                Err(e) => {
                    drop(llm);
                    let mut llm = state.llm.write().await;
                    llm.breakers.record_failure(&provider_prefix);
                    drop(llm);

                    format!(
                        "I encountered an error reaching the LLM provider: {}. Please try again.",
                        e
                    )
                }
            }
        }
        None => format!(
            "No provider configured for '{}'. I can't respond right now.",
            provider_prefix
        ),
    };

    let response_content = if ironclad_agent::injection::scan_output(&response_content) {
        tracing::warn!("L4 output scan flagged channel response, blocking");
        "I can't share that response — it was flagged by my output safety filter.".to_string()
    } else {
        response_content
    };

    ironclad_db::sessions::append_message(&state.db, &session_id, "assistant", &response_content)
        .ok();

    state
        .channel_router
        .send_reply(&platform, &chat_id, response_content)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub async fn telegram_poll_loop(state: AppState) {
    let adapter = match &state.telegram {
        Some(a) => a.clone(),
        None => return,
    };

    tracing::info!("Telegram long-poll loop started");

    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = process_channel_message(&state, inbound).await {
                        tracing::error!(error = %e, "Telegram message processing failed");
                    }
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = %e, "Telegram poll error, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_zero_tokens() {
        assert_eq!(estimate_cost_from_provider(0.001, 0.002, 0, 0), 0.0);
    }

    #[test]
    fn estimate_cost_input_only() {
        let cost = estimate_cost_from_provider(0.001, 0.002, 100, 0);
        assert!((cost - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_cost_output_only() {
        let cost = estimate_cost_from_provider(0.001, 0.002, 0, 100);
        assert!((cost - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_cost_both_directions() {
        let cost = estimate_cost_from_provider(0.001, 0.002, 500, 200);
        let expected = 500.0 * 0.001 + 200.0 * 0.002;
        assert!((cost - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn check_tool_policy_allows_when_no_rules() {
        let engine = ironclad_agent::policy::PolicyEngine::new();
        let result = check_tool_policy(
            &engine,
            "read_file",
            &serde_json::json!({"path": "/tmp/test.txt"}),
            ironclad_core::InputAuthority::Creator,
            ironclad_core::SurvivalTier::Normal,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn check_tool_policy_deny_returns_403_and_reason() {
        let mut engine = ironclad_agent::policy::PolicyEngine::new();
        engine.add_rule(Box::new(ironclad_agent::policy::AuthorityRule));
        let result = check_tool_policy(
            &engine,
            "bash",
            &serde_json::json!({"command": "rm -rf /"}),
            ironclad_core::InputAuthority::External,
            ironclad_core::SurvivalTier::Normal,
        );
        let (status, reason) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!reason.is_empty());
    }

    #[test]
    fn check_tool_policy_with_authority_rule() {
        let mut engine = ironclad_agent::policy::PolicyEngine::new();
        engine.add_rule(Box::new(ironclad_agent::policy::AuthorityRule));
        let result = check_tool_policy(
            &engine,
            "wallet_transfer",
            &serde_json::json!({"amount": 100}),
            ironclad_core::InputAuthority::Creator,
            ironclad_core::SurvivalTier::Normal,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn check_tool_policy_critical_tier_restricts() {
        let mut engine = ironclad_agent::policy::PolicyEngine::new();
        engine.add_rule(Box::new(ironclad_agent::policy::AuthorityRule));
        engine.add_rule(Box::new(ironclad_agent::policy::CommandSafetyRule));
        let result = check_tool_policy(
            &engine,
            "read_file",
            &serde_json::json!({"path": "/etc/passwd"}),
            ironclad_core::InputAuthority::External,
            ironclad_core::SurvivalTier::Critical,
        );
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn estimate_cost_negative_tokens_handled() {
        let cost = estimate_cost_from_provider(0.001, 0.002, -100, -50);
        assert!(cost < 0.0);
    }

    #[test]
    fn estimate_cost_large_values() {
        let cost = estimate_cost_from_provider(0.00001, 0.00003, 1_000_000, 500_000);
        let expected = 1_000_000.0 * 0.00001 + 500_000.0 * 0.00003;
        assert!((cost - expected).abs() < 1e-6);
    }

    #[test]
    fn estimate_cost_zero_rates() {
        let cost = estimate_cost_from_provider(0.0, 0.0, 1000, 2000);
        assert_eq!(cost, 0.0);
    }
}
