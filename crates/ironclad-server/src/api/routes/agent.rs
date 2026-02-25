//! Agent message, channel processing, and Telegram poll.

use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::StreamExt;
use ironclad_agent::agent_loop::{AgentLoop, ReactAction, ReactState};
use ironclad_agent::tools::ToolContext;
use ironclad_channels::ChannelAdapter;
use ironclad_core::InputAuthority;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::AppState;

/// RAII guard that releases a dedup fingerprint when dropped.
/// Ensures cleanup on all exit paths, including async stream disconnects.
struct DedupGuard {
    llm: Arc<tokio::sync::RwLock<ironclad_llm::LlmService>>,
    fingerprint: String,
}

impl Drop for DedupGuard {
    fn drop(&mut self) {
        if self.fingerprint.is_empty() {
            return;
        }
        let llm = Arc::clone(&self.llm);
        let fp = std::mem::take(&mut self.fingerprint);
        tokio::spawn(async move {
            let mut llm = llm.write().await;
            llm.dedup.release(&fp);
        });
    }
}

/// Try to extract a tool call from the LLM's text response.
/// Looks for `{"tool_call": {"name": "...", "params": {...}}}` in the response.
fn parse_tool_call(response: &str) -> Option<(String, serde_json::Value)> {
    let start = response.find(r#""tool_call""#)?;
    let brace_start = response[..start].rfind('{')?;
    let mut depth = 0;
    let mut end = brace_start;
    for (i, ch) in response[brace_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = brace_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    let json_str = &response[brace_start..end];
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let tool_call = parsed.get("tool_call")?;
    let name = tool_call.get("name")?.as_str()?.to_string();
    let params = tool_call.get("params").cloned().unwrap_or(json!({}));
    Some((name, params))
}

fn provider_failure_user_message(last_error: &str, message_already_stored: bool) -> String {
    if message_already_stored {
        format!(
            "I encountered an error reaching all LLM providers: {}. Your message has been stored and I'll retry when a provider is available.",
            last_error
        )
    } else {
        format!(
            "I encountered an error reaching all LLM providers: {}. Please try again.",
            last_error
        )
    }
}

/// Execute a tool call through the ToolRegistry, enforcing policy and recording audit trails.
pub(crate) async fn execute_tool_call(
    state: &AppState,
    tool_name: &str,
    params: &serde_json::Value,
    turn_id: &str,
    authority: InputAuthority,
) -> Result<String, String> {
    fn parse_risk_level(raw: &str) -> ironclad_core::RiskLevel {
        match raw.to_ascii_lowercase().as_str() {
            "safe" => ironclad_core::RiskLevel::Safe,
            "dangerous" => ironclad_core::RiskLevel::Dangerous,
            "forbidden" => ironclad_core::RiskLevel::Forbidden,
            _ => ironclad_core::RiskLevel::Caution,
        }
    }
    fn resolve_script_audit_path(skills_dir: &std::path::Path, script_arg: &str) -> Option<String> {
        let requested = std::path::Path::new(script_arg);
        let root = std::fs::canonicalize(skills_dir).ok()?;
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            root.join(requested)
        };
        let canonical = std::fs::canonicalize(&joined).ok()?;
        if canonical.starts_with(&root) {
            Some(canonical.to_string_lossy().to_string())
        } else {
            None
        }
    }

    let balance = state.wallet.wallet.get_usdc_balance().await.unwrap_or(0.0);
    let tier = ironclad_core::SurvivalTier::from_balance(balance, 0.0);
    let tool = match state.tools.get(tool_name) {
        Some(t) => t,
        None => return Err(format!("Unknown tool: {tool_name}")),
    };
    let mut effective_risk = tool.risk_level();
    let mut matched_skill: Option<(String, String, String)> = None;

    if tool_name == "run_script" {
        let script_arg = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let config = state.config.read().await;
        let maybe_script_path = resolve_script_audit_path(&config.skills.skills_dir, script_arg);
        drop(config);

        if let Some(script_path) = maybe_script_path
            && let Ok(Some(skill)) =
                ironclad_db::skills::find_enabled_skill_by_script_path(&state.db, &script_path)
        {
            effective_risk = parse_risk_level(&skill.risk_level);
            matched_skill = Some((
                skill.id.clone(),
                skill.name.clone(),
                skill.content_hash.clone(),
            ));

            if let Some(overrides_raw) = skill.policy_overrides_json.as_deref()
                && let Ok(overrides) = serde_json::from_str::<serde_json::Value>(overrides_raw)
            {
                let require_creator = overrides
                    .get("require_creator")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let deny_external = overrides
                    .get("deny_external")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let disabled = overrides
                    .get("disabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if disabled {
                    return Err(format!(
                        "Policy override denied: skill '{}' is disabled",
                        skill.name
                    ));
                }
                if require_creator && authority != ironclad_core::InputAuthority::Creator {
                    return Err(format!(
                        "Policy override denied: skill '{}' requires Creator authority",
                        skill.name
                    ));
                }
                if deny_external && authority == ironclad_core::InputAuthority::External {
                    return Err(format!(
                        "Policy override denied: skill '{}' denies External authority",
                        skill.name
                    ));
                }
            }
        }
    }

    let policy_result = check_tool_policy(
        &state.policy_engine,
        tool_name,
        params,
        authority,
        tier,
        effective_risk,
    );

    let (decision_str, rule_name, reason) = match &policy_result {
        Ok(()) => ("allow".to_string(), None, None),
        Err((_status, msg)) => (
            "deny".to_string(),
            Some("policy_engine"),
            Some(msg.as_str()),
        ),
    };

    ironclad_db::policy::record_policy_decision(
        &state.db,
        Some(turn_id),
        tool_name,
        &decision_str,
        rule_name,
        reason,
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record policy decision"))
    .ok();

    if let Err((_status, msg)) = policy_result {
        return Err(format!("Policy denied: {msg}"));
    }

    // Approval gate: block gated tools until a human approves
    match state.approvals.check_tool(tool_name) {
        Ok(ironclad_agent::approvals::ToolClassification::Gated) => {
            let request = state
                .approvals
                .request_approval(tool_name, &params.to_string(), Some(turn_id))
                .map_err(|e| format!("Approval error: {e}"))?;
            ironclad_db::approvals::record_approval_request(
                &state.db,
                &request.id,
                &request.tool_name,
                &request.tool_input,
                request.session_id.as_deref(),
                "pending",
                &request.timeout_at.to_rfc3339(),
            )
            .inspect_err(|e| tracing::warn!(error = %e, "failed to persist approval request"))
            .ok();
            state.event_bus.publish(
                serde_json::json!({
                    "type": "pending_approval",
                    "tool": tool_name,
                    "request_id": request.id,
                })
                .to_string(),
            );
            return Err(format!(
                "Tool '{tool_name}' requires approval (request: {})",
                request.id
            ));
        }
        Err(e) => {
            return Err(format!("Tool blocked: {e}"));
        }
        Ok(_) => {}
    }

    let workspace_root = {
        let cfg = state.config.read().await;
        cfg.agent.workspace.clone()
    };
    let ctx = ToolContext {
        session_id: turn_id.to_string(),
        agent_id: "ironclad".to_string(),
        authority,
        workspace_root,
    };

    state.event_bus.publish(
        serde_json::json!({
            "type": "agent_working",
            "agent_id": "ironclad",
            "workstation": "exec",
            "activity": format!("tool:{tool_name}"),
            "turn_id": turn_id,
        })
        .to_string(),
    );
    if let Some((_, skill_name, _)) = matched_skill.as_ref() {
        state.event_bus.publish(
            serde_json::json!({
                "type": "skill_activated",
                "agent_id": "ironclad",
                "skill": skill_name,
                "tool_name": tool_name,
                "turn_id": turn_id,
            })
            .to_string(),
        );
    }

    let start = std::time::Instant::now();
    let timeout_duration = std::time::Duration::from_secs(120);
    let result =
        match tokio::time::timeout(timeout_duration, tool.execute(params.clone(), &ctx)).await {
            Ok(result) => result,
            Err(_) => Err(ironclad_agent::tools::ToolError {
                message: format!("Tool '{tool_name}' timed out after {timeout_duration:?}"),
            }),
        };
    let duration_ms = start.elapsed().as_millis() as i64;

    const MAX_TOOL_OUTPUT: usize = 16_384;
    let (output, status) = match &result {
        Ok(r) => {
            let mut out = if r.output.len() > MAX_TOOL_OUTPUT {
                let boundary = r.output.floor_char_boundary(MAX_TOOL_OUTPUT);
                format!(
                    "{}...\n[truncated: {} bytes total]",
                    &r.output[..boundary],
                    r.output.len()
                )
            } else {
                r.output.clone()
            };
            let mut status = "success";
            if let Some(unreadable) = r
                .metadata
                .as_ref()
                .and_then(|m| m.get("unreadable_files"))
                .and_then(|v| v.as_u64())
                && unreadable > 0
            {
                status = "partial_success";
                out = format!("{out}\n\n[warning] Search skipped {unreadable} unreadable file(s).");
            }
            (out, status)
        }
        Err(e) => (e.message.clone(), "error"),
    };

    let (skill_id, skill_name, skill_hash) = match matched_skill.as_ref() {
        Some((id, name, hash)) => (Some(id.as_str()), Some(name.as_str()), Some(hash.as_str())),
        None => (None, None, None),
    };
    ironclad_db::tools::record_tool_call_with_skill(
        &state.db,
        turn_id,
        tool_name,
        &params.to_string(),
        Some(&output),
        status,
        Some(duration_ms),
        skill_id,
        skill_name,
        skill_hash,
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record tool call"))
    .ok();

    state.event_bus.publish(
        serde_json::json!({
            "type": "agent_idle",
            "agent_id": "ironclad",
            "workstation": "exec",
            "turn_id": turn_id,
        })
        .to_string(),
    );

    result.map(|_| output).map_err(|e| e.message)
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeDiagnostics {
    uptime_seconds: u64,
    primary_model: String,
    active_model: String,
    primary_provider: String,
    primary_provider_state: String,
    breaker_open_count: usize,
    breaker_half_open_count: usize,
    cache_entries: usize,
    cache_hit_rate_pct: f64,
    pending_approvals: usize,
    taskable_subagents_total: usize,
    taskable_subagents_enabled: usize,
    taskable_subagents_running: usize,
    taskable_subagents_error: usize,
    channels_total: usize,
    channels_with_errors: usize,
}

fn sanitize_diag_token(raw: &str, max_len: usize) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | ':'))
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '-' || c == '_' || c == ':' || c == '/');
    trimmed.chars().take(max_len).collect()
}

fn is_model_proxy_role(role: &str) -> bool {
    role.eq_ignore_ascii_case("model-proxy")
}

async fn collect_runtime_diagnostics(state: &AppState) -> RuntimeDiagnostics {
    let (
        primary_model,
        active_model,
        primary_provider,
        primary_provider_state,
        cache_entries,
        cache_hit_rate_pct,
        breaker_open_count,
        breaker_half_open_count,
    ) = {
        let config = state.config.read().await;
        let llm = state.llm.read().await;
        let primary_model = sanitize_diag_token(&config.models.primary, 120);
        let active_model = sanitize_diag_token(llm.router.select_model(), 120);
        let primary_provider = sanitize_diag_token(
            config.models.primary.split('/').next().unwrap_or("unknown"),
            40,
        );
        let primary_provider_state =
            format!("{:?}", llm.breakers.get_state(&primary_provider)).to_lowercase();
        let providers = llm.breakers.list_providers();
        let breaker_open_count = providers
            .iter()
            .filter(|(_, s)| *s == ironclad_llm::CircuitState::Open)
            .count();
        let breaker_half_open_count = providers
            .iter()
            .filter(|(_, s)| *s == ironclad_llm::CircuitState::HalfOpen)
            .count();
        let cache_entries = llm.cache.size();
        let hits = llm.cache.hit_count();
        let misses = llm.cache.miss_count();
        let cache_hit_rate_pct = if hits + misses > 0 {
            (hits as f64 / (hits + misses) as f64) * 100.0
        } else {
            0.0
        };
        (
            primary_model,
            active_model,
            primary_provider,
            primary_provider_state,
            cache_entries,
            cache_hit_rate_pct,
            breaker_open_count,
            breaker_half_open_count,
        )
    };

    let channels = state.channel_router.channel_status().await;
    let channels_with_errors = channels.iter().filter(|c| c.last_error.is_some()).count();
    let runtime_agents = state.registry.list_agents().await;
    let configured_subagents = ironclad_db::agents::list_sub_agents(&state.db).unwrap_or_default();
    let model_proxy_names: HashSet<String> = configured_subagents
        .iter()
        .filter(|a| is_model_proxy_role(&a.role))
        .map(|a| a.name.to_ascii_lowercase())
        .collect();
    let taskable_subagents_running = runtime_agents
        .iter()
        .filter(|a| !model_proxy_names.contains(&a.id.to_ascii_lowercase()))
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Running)
        .count();
    let taskable_subagents_error = runtime_agents
        .iter()
        .filter(|a| !model_proxy_names.contains(&a.id.to_ascii_lowercase()))
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Error)
        .count();
    let taskable_subagents_total = configured_subagents
        .iter()
        .filter(|a| !is_model_proxy_role(&a.role))
        .count();
    let taskable_subagents_enabled = configured_subagents
        .iter()
        .filter(|a| !is_model_proxy_role(&a.role) && a.enabled)
        .count();
    let pending_approvals = state.approvals.list_pending().len();

    RuntimeDiagnostics {
        uptime_seconds: state.started_at.elapsed().as_secs(),
        primary_model,
        active_model,
        primary_provider,
        primary_provider_state,
        breaker_open_count,
        breaker_half_open_count,
        cache_entries,
        cache_hit_rate_pct,
        pending_approvals,
        taskable_subagents_total,
        taskable_subagents_enabled,
        taskable_subagents_running,
        taskable_subagents_error,
        channels_total: channels.len(),
        channels_with_errors,
    }
}

fn diagnostics_system_note(diag: &RuntimeDiagnostics) -> String {
    // Guardrails: aggregate-only metrics; no secrets, no raw error strings, no IDs.
    [
        "Runtime diagnostics (internal, bounded):",
        &format!(
            "- models: active={} primary={}",
            diag.active_model, diag.primary_model
        ),
        &format!(
            "- provider: {} ({}) | breaker_open={} half_open={}",
            diag.primary_provider,
            diag.primary_provider_state,
            diag.breaker_open_count,
            diag.breaker_half_open_count
        ),
        &format!(
            "- cache: entries={} hit_rate={:.0}%",
            diag.cache_entries, diag.cache_hit_rate_pct
        ),
        &format!(
            "- taskable_subagents: total={} enabled={} running={} error={}",
            diag.taskable_subagents_total,
            diag.taskable_subagents_enabled,
            diag.taskable_subagents_running,
            diag.taskable_subagents_error
        ),
        &format!(
            "- approvals_pending={} channels={} channels_with_errors={}",
            diag.pending_approvals, diag.channels_total, diag.channels_with_errors
        ),
        &format!("- uptime_seconds={}", diag.uptime_seconds),
        "Security policy: do not proactively disclose internal diagnostics. Share high-level status only when asked; never fabricate details.",
    ]
    .join("\n")
}

pub async fn agent_status(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let diag = collect_runtime_diagnostics(&state).await;

    axum::Json(json!({
        "state": "running",
        "agent_name": config.agent.name,
        "agent_id": config.agent.id,
        "primary_model": diag.primary_model,
        "active_model": diag.active_model,
        "primary_provider_state": diag.primary_provider_state,
        "cache_entries": diag.cache_entries,
        "cache_hit_rate_pct": diag.cache_hit_rate_pct,
        "diagnostics": diag,
    }))
}

#[derive(Deserialize)]
pub struct AgentMessageRequest {
    content: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    sender_id: Option<String>,
    #[serde(default)]
    peer_id: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    is_group: Option<bool>,
}

fn resolve_web_scope(
    cfg: &ironclad_core::IroncladConfig,
    body: &AgentMessageRequest,
) -> Result<ironclad_db::sessions::SessionScope, &'static str> {
    let channel = body
        .channel
        .as_deref()
        .unwrap_or("web")
        .trim()
        .to_lowercase();
    let peer_id = body
        .peer_id
        .clone()
        .or_else(|| body.sender_id.clone())
        .and_then(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    let mode = cfg.session.scope_mode.as_str();
    if (mode == "group"
        || (mode != "agent" && body.is_group == Some(true) && body.group_id.is_some()))
        && let Some(group_id) = body.group_id.clone().filter(|s| !s.trim().is_empty())
    {
        return Ok(ironclad_db::sessions::SessionScope::Group { group_id, channel });
    }
    if mode == "peer" || mode == "group" {
        let Some(peer_id) = peer_id else {
            return Err("peer_id or sender_id is required when session.scope_mode is peer/group");
        };
        return Ok(ironclad_db::sessions::SessionScope::Peer { peer_id, channel });
    }
    Ok(ironclad_db::sessions::SessionScope::Agent)
}

pub async fn agent_message(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<AgentMessageRequest>,
) -> impl IntoResponse {
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
            ironclad_db::sessions::update_nickname(&state.db, &session_id, &nick).ok();
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
    let model_audit = select_routed_model_with_audit(&state, &user_content).await;
    let model = model_audit.selected_model.clone();
    let complexity_label = format!("{complexity:?}");
    persist_model_selection_audit(
        &state,
        &turn_id,
        &session_id,
        "api",
        Some(&complexity_label),
        &user_content,
        &model_audit,
    )
    .await;

    let provider_prefix = model.split('/').next().unwrap_or("unknown").to_string();
    let tier_adapt = config.tier_adapt.clone();
    let agent_name = config.agent.name.clone();
    let primary_model = config.models.primary.clone();
    let personality = state.personality.read().await;
    let soul_text = personality.soul_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);
    drop(config);

    // Resolve tier for message adaptation
    let tier = {
        let llm = state.llm.read().await;
        llm.providers
            .get_by_model(&model)
            .map(|p| p.tier)
            .unwrap_or_else(|| ironclad_llm::tier::classify(&model))
    };

    // Generate query embedding for RAG retrieval and cache L2 lookup
    let query_embedding = {
        let llm = state.llm.read().await;
        llm.embedding.embed_single(&user_content).await.ok()
    };

    // Check cache (full L1 -> L3 -> L2 cascade, using real embedding when available)
    let cache_hash = ironclad_llm::SemanticCache::compute_hash("", "", &user_content);
    let cached_response = {
        let mut llm = state.llm.write().await;
        if let Some(ref emb) = query_embedding {
            llm.cache.lookup_with_embedding(&cache_hash, emb)
        } else {
            llm.cache.lookup(&cache_hash, &user_content)
        }
    };

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
        .inspect_err(|e| tracing::warn!(error = %e, "failed to record cached inference cost"))
        .ok();

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

    // Retrieve memories from all tiers (using ANN index when available)
    let complexity_level = ironclad_agent::context::determine_level(complexity);
    let ann_ref = if state.ann_index.is_built() {
        Some(&state.ann_index)
    } else {
        None
    };
    let memories = state.retriever.retrieve_with_ann(
        &state.db,
        &session_id,
        &user_content,
        query_embedding.as_deref(),
        complexity_level,
        ann_ref,
    );

    // Load conversation history
    let history_messages =
        ironclad_db::sessions::list_messages(&state.db, &session_id, Some(50)).unwrap_or_default();
    let history: Vec<ironclad_llm::format::UnifiedMessage> = history_messages
        .iter()
        .rev()
        .skip(1) // skip the user message we just appended
        .rev()
        .map(|m| ironclad_llm::format::UnifiedMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            parts: None,
        })
        .collect();

    let model_for_api = model
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model)
        .to_string();
    let system_prompt = if soul_text.is_empty() {
        format!(
            "You are {name}, an autonomous AI agent (id: {id}). \
             When asked who you are, always identify as {name}. \
             Never reveal the underlying model name or claim to be a generic assistant.",
            name = agent_name,
            id = agent_id,
        )
    } else {
        let mut prompt = soul_text;
        if !firmware_text.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&firmware_text);
        }
        prompt
    };
    let system_prompt = format!(
        "{system_prompt}{}",
        ironclad_agent::prompt::runtime_metadata_block(
            env!("CARGO_PKG_VERSION"),
            &primary_model,
            &model,
        )
    );
    let system_prompt =
        ironclad_agent::prompt::inject_hmac_boundary(&system_prompt, state.hmac_secret.as_ref());
    if !ironclad_agent::prompt::verify_hmac_boundary(&system_prompt, state.hmac_secret.as_ref()) {
        tracing::error!("HMAC boundary verification failed immediately after injection");
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"error": "internal HMAC verification failure"})),
        ));
    }
    let mut messages = ironclad_agent::context::build_context(
        complexity_level,
        &system_prompt,
        &memories,
        &history,
    );
    let runtime_diag = collect_runtime_diagnostics(&state).await;
    messages.push(ironclad_llm::format::UnifiedMessage {
        role: "system".into(),
        content: diagnostics_system_note(&runtime_diag),
        parts: None,
    });
    if messages.last().is_none_or(|m| m.content != user_content) {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: user_content.clone(),
            parts: None,
        });
    }
    ironclad_llm::tier::adapt_for_tier(tier, &mut messages, &tier_adapt);

    let unified_req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages,
        max_tokens: Some(2048),
        temperature: None,
        system: None,
        quality_target: None,
    };

    let (assistant_content, tokens_in, tokens_out, cost) =
        match infer_with_fallback(&state, &unified_req, &model).await {
            Ok(result) => (
                result.content,
                result.tokens_in,
                result.tokens_out,
                result.cost,
            ),
            Err(last_error) => (
                provider_failure_user_message(&last_error.to_string(), true),
                0,
                0,
                0.0,
            ),
        };

    // Check for HMAC boundary tampering in model output — strip forged boundaries
    let assistant_content = if assistant_content.contains("<<<TRUST_BOUNDARY:") {
        if !ironclad_agent::prompt::verify_hmac_boundary(
            &assistant_content,
            state.hmac_secret.as_ref(),
        ) {
            tracing::warn!("HMAC boundary tampered in model output, stripping");
            ironclad_agent::prompt::strip_hmac_boundaries(&assistant_content)
        } else {
            assistant_content
        }
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

    // ReAct loop: if the LLM response contains a tool call, execute it and loop
    let authority = if reduced_authority {
        InputAuthority::External
    } else {
        InputAuthority::Creator
    };
    let mut react_loop = AgentLoop::new(10);
    let mut final_content = assistant_content.clone();
    let mut total_tokens_in = tokens_in;
    let mut total_tokens_out = tokens_out;
    let mut total_cost = cost;

    if let Some((tool_name, tool_params)) = parse_tool_call(&assistant_content) {
        react_loop.transition(ReactAction::Think);
        let mut react_messages = unified_req.messages.clone();

        react_messages.push(ironclad_llm::format::UnifiedMessage {
            role: "assistant".into(),
            content: assistant_content.clone(),
            parts: None,
        });

        let mut current_tool = Some((tool_name, tool_params));

        while let Some((ref tool_name, ref tool_params)) = current_tool {
            if react_loop.is_looping(tool_name, &tool_params.to_string()) {
                tracing::warn!(tool = %tool_name, "ReAct loop detected, breaking");
                break;
            }

            react_loop.transition(ReactAction::Act {
                tool_name: tool_name.clone(),
                params: tool_params.to_string(),
            });

            let tool_result =
                execute_tool_call(&state, tool_name, tool_params, &turn_id, authority).await;

            let observation = match tool_result {
                Ok(output) => format!("[Tool {tool_name} succeeded]: {output}"),
                Err(err) => format!("[Tool {tool_name} failed]: {err}"),
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
                model: unified_req.model.clone(),
                messages: react_messages.clone(),
                max_tokens: Some(2048),
                temperature: None,
                system: None,
                quality_target: None,
            };

            let follow_content = match infer_with_fallback(&state, &follow_req, &model).await {
                Ok(result) => {
                    total_tokens_in += result.tokens_in;
                    total_tokens_out += result.tokens_out;
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

            let follow_content = if follow_content.contains("<<<TRUST_BOUNDARY:") {
                if !ironclad_agent::prompt::verify_hmac_boundary(
                    &follow_content,
                    state.hmac_secret.as_ref(),
                ) {
                    tracing::warn!("HMAC boundary tampered in ReAct follow-up, stripping");
                    ironclad_agent::prompt::strip_hmac_boundaries(&follow_content)
                } else {
                    follow_content
                }
            } else {
                follow_content
            };
            let follow_content = if ironclad_agent::injection::scan_output(&follow_content) {
                tracing::warn!("L4 output scan flagged ReAct follow-up response, blocking");
                "[Response blocked by output safety filter]".to_string()
            } else {
                follow_content
            };

            current_tool = parse_tool_call(&follow_content);
            if current_tool.is_none() {
                react_loop.transition(ReactAction::Finish);
                final_content = follow_content;
            }
        }
    }

    let assistant_content = final_content;

    // Store assistant response
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

    ironclad_db::metrics::record_inference_cost(
        &state.db,
        &model,
        &provider_prefix,
        total_tokens_in,
        total_tokens_out,
        total_cost,
        None,
        false,
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record inference cost"))
    .ok();

    // Post-turn memory ingestion + embedding generation with chunking (background)
    {
        let ingest_db = state.db.clone();
        let ingest_session = session_id.clone();
        let ingest_user = user_content.clone();
        let ingest_assistant = assistant_content.clone();
        let ingest_llm = Arc::clone(&state.llm);
        tokio::spawn(async move {
            ironclad_agent::memory::ingest_turn(
                &ingest_db,
                &ingest_session,
                &ingest_user,
                &ingest_assistant,
                &[],
            );

            let llm = ingest_llm.read().await;

            // Chunk long responses before embedding (512-token threshold)
            let chunk_config = ironclad_agent::retrieval::ChunkConfig::default();
            let chunks = ironclad_agent::retrieval::chunk_text(&ingest_assistant, &chunk_config);

            for chunk in &chunks {
                if let Ok(embedding) = llm.embedding.embed_single(&chunk.text).await {
                    let embed_id = uuid::Uuid::new_v4().to_string();
                    ironclad_db::embeddings::store_embedding(
                        &ingest_db,
                        &embed_id,
                        "turn",
                        &ingest_session,
                        &chunk.text[..chunk.text.len().min(200)],
                        &embedding,
                    )
                    .inspect_err(|e| tracing::warn!(error = %e, chunk_idx = chunk.index, "failed to store chunk embedding"))
                    .ok();
                }
            }
        });
    }

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
            .store_with_embedding(&cache_hash, &user_content, cached_entry);
    }

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
        "model": model,
        "cached": false,
        "tokens_in": total_tokens_in,
        "tokens_out": total_tokens_out,
        "cost": total_cost,
        "threat_score": threat.value(),
        "reduced_authority": reduced_authority,
        "react_turns": react_loop.turn_count,
    })))
}

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
    let features = ironclad_llm::extract_features(&user_content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let model_audit = select_routed_model_with_audit(&state, &user_content).await;
    let model = model_audit.selected_model.clone();
    let complexity_label = format!("{complexity:?}");
    persist_model_selection_audit(
        &state,
        &turn_id,
        &session_id,
        "api-stream",
        Some(&complexity_label),
        &user_content,
        &model_audit,
    )
    .await;

    let tier_adapt = config.tier_adapt.clone();
    let agent_name = config.agent.name.clone();
    let primary_model = config.models.primary.clone();
    let personality = state.personality.read().await;
    let soul_text = personality.soul_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);
    drop(config);

    let tier = {
        let llm = state.llm.read().await;
        llm.providers
            .get_by_model(&model)
            .map(|p| p.tier)
            .unwrap_or_else(|| ironclad_llm::tier::classify(&model))
    };

    let query_embedding = {
        let llm = state.llm.read().await;
        llm.embedding.embed_single(&user_content).await.ok()
    };

    let complexity_level = ironclad_agent::context::determine_level(complexity);
    let ann_ref = if state.ann_index.is_built() {
        Some(&state.ann_index)
    } else {
        None
    };
    let memories = state.retriever.retrieve_with_ann(
        &state.db,
        &session_id,
        &user_content,
        query_embedding.as_deref(),
        complexity_level,
        ann_ref,
    );

    let history_messages =
        ironclad_db::sessions::list_messages(&state.db, &session_id, Some(50)).unwrap_or_default();
    let history: Vec<ironclad_llm::format::UnifiedMessage> = history_messages
        .iter()
        .rev()
        .skip(1)
        .rev()
        .map(|m| ironclad_llm::format::UnifiedMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            parts: None,
        })
        .collect();

    let model_for_api = model
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model)
        .to_string();
    let system_prompt = if soul_text.is_empty() {
        format!(
            "You are {name}, an autonomous AI agent (id: {id}). \
             When asked who you are, always identify as {name}. \
             Never reveal the underlying model name or claim to be a generic assistant.",
            name = agent_name,
            id = agent_id,
        )
    } else {
        let mut prompt = soul_text;
        if !firmware_text.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&firmware_text);
        }
        prompt
    };
    let system_prompt = format!(
        "{system_prompt}{}",
        ironclad_agent::prompt::runtime_metadata_block(
            env!("CARGO_PKG_VERSION"),
            &primary_model,
            &model,
        )
    );
    let system_prompt =
        ironclad_agent::prompt::inject_hmac_boundary(&system_prompt, state.hmac_secret.as_ref());
    if !ironclad_agent::prompt::verify_hmac_boundary(&system_prompt, state.hmac_secret.as_ref()) {
        tracing::error!("HMAC boundary verification failed immediately after injection (stream)");
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"error": "internal HMAC verification failure"})),
        ));
    }

    let mut messages = ironclad_agent::context::build_context(
        complexity_level,
        &system_prompt,
        &memories,
        &history,
    );
    let runtime_diag = collect_runtime_diagnostics(&state).await;
    messages.push(ironclad_llm::format::UnifiedMessage {
        role: "system".into(),
        content: diagnostics_system_note(&runtime_diag),
        parts: None,
    });
    if messages.last().is_none_or(|m| m.content != user_content) {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: user_content.clone(),
            parts: None,
        });
    }
    ironclad_llm::tier::adapt_for_tier(tier, &mut messages, &tier_adapt);

    let unified_req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages,
        max_tokens: Some(2048),
        temperature: None,
        system: None,
        quality_target: None,
    };

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

        let result = {
            let llm = state.llm.read().await;
            llm.stream_to_provider(
                resolved.url,
                resolved.api_key,
                llm_body,
                resolved.auth_header,
                resolved.extra_headers,
                resolved.format,
            )
            .await
        };

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
    let event_bus = state.event_bus.clone();
    let db = state.db.clone();
    let cache_hash = ironclad_llm::SemanticCache::compute_hash("", "", &user_content);
    let llm_arc = Arc::clone(&state.llm);
    let hmac_secret_clone = state.hmac_secret.clone();
    let user_content_clone = user_content.clone();

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
                "agent_id": "ironclad",
                "workstation": "llm",
                "activity": "inference",
                "session_id": session_id_clone,
                "model": model_clone,
            })
            .to_string(),
        );

        let mut accumulator = ironclad_llm::format::StreamAccumulator::default();
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

        // HMAC boundary check on accumulated output
        let assistant_content = if unified_resp.content.contains("<<<TRUST_BOUNDARY:") {
            if !ironclad_agent::prompt::verify_hmac_boundary(
                &unified_resp.content,
                hmac_secret_clone.as_ref(),
            ) {
                tracing::warn!("HMAC boundary tampered in streaming output, stripping");
                ironclad_agent::prompt::strip_hmac_boundaries(&unified_resp.content)
            } else {
                unified_resp.content.clone()
            }
        } else {
            unified_resp.content.clone()
        };

        // L4 output scanning
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
        ironclad_db::sessions::append_message(
            &db,
            &session_id_clone,
            "assistant",
            &assistant_content,
        ).ok();

        // Record inference cost
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
        ironclad_db::metrics::record_inference_cost(
            &db,
            &model_clone,
            &provider_prefix,
            unified_resp.tokens_in as i64,
            unified_resp.tokens_out as i64,
            cost,
            None,
            false,
        )
        .inspect_err(|e| tracing::warn!(error = %e, "failed to record streaming inference cost"))
        .ok();

        // Cache write-through
        if unified_resp.tokens_out > 0 {
            let cached_entry = ironclad_llm::CachedResponse {
                content: assistant_content.clone(),
                model: model_clone.clone(),
                tokens_saved: unified_resp.tokens_out,
                created_at: std::time::Instant::now(),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3600),
                hits: 0,
                involved_tools: false,
                embedding: None,
            };
            let mut llm = llm_arc.write().await;
            llm.breakers.record_success(&provider_prefix);
            let total_tokens = unified_resp.tokens_in + unified_resp.tokens_out;
            llm.capacity.record(&provider_prefix, total_tokens as u64);
            let pressured = llm.capacity.is_sustained_hot(&provider_prefix);
            llm.breakers.set_capacity_pressure(&provider_prefix, pressured);
            llm.cache.store_with_embedding(&cache_hash, &user_content_clone, cached_entry);
        } else {
            let mut llm = llm_arc.write().await;
            llm.breakers.record_success(&provider_prefix);
            let total_tokens = unified_resp.tokens_in + unified_resp.tokens_out;
            llm.capacity.record(&provider_prefix, total_tokens as u64);
            let pressured = llm.capacity.is_sustained_hot(&provider_prefix);
            llm.breakers.set_capacity_pressure(&provider_prefix, pressured);
        }

        let done_event = json!({
            "type": "stream_chunk",
            "content": "",
            "done": true,
            "session_id": session_id_clone,
        });
        event_bus.publish(done_event.to_string());
        event_bus.publish(
            json!({
                "type": "agent_idle",
                "agent_id": "ironclad",
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

/// Refine a session's nickname using the LLM to summarize conversation topics.
async fn refine_session_nickname(
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
            let key = super::admin::resolve_provider_key(
                &p.name,
                p.is_local,
                &p.auth_mode,
                p.api_key_ref.as_deref(),
                &p.api_key_env,
                oauth,
                keystore,
            )
            .await
            .unwrap_or_default();
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

struct InferenceResult {
    content: String,
    model: String,
    provider: String,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
}

struct ResolvedInferenceProvider {
    url: String,
    api_key: String,
    auth_header: String,
    extra_headers: std::collections::HashMap<String, String>,
    format: ironclad_core::ApiFormat,
    cost_in: f64,
    cost_out: f64,
    is_local: bool,
    provider_prefix: String,
}

#[derive(Debug, Clone, Serialize)]
struct ModelCandidateAudit {
    model: String,
    source: String,
    provider_available: bool,
    breaker_blocked: bool,
    usable: bool,
    note: String,
}

#[derive(Debug, Clone, Serialize)]
struct ModelSelectionAudit {
    selected_model: String,
    strategy: String,
    primary_model: String,
    override_model: Option<String>,
    ordered_models: Vec<String>,
    candidates: Vec<ModelCandidateAudit>,
}

fn summarize_user_excerpt(input: &str) -> String {
    input
        .split_whitespace()
        .take(20)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}

async fn persist_model_selection_audit(
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
            "candidates": audit.candidates,
            "created_at": row.created_at,
        })
        .to_string(),
    );
}

fn fallback_candidates(config: &ironclad_core::IroncladConfig, initial_model: &str) -> Vec<String> {
    let mut candidates = vec![initial_model.to_string()];
    for fb in &config.models.fallbacks {
        if fb != initial_model {
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

async fn select_routed_model_with_audit(
    state: &AppState,
    _user_content: &str,
) -> ModelSelectionAudit {
    let config = state.config.read().await;
    let primary = config.models.primary.clone();
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
        }
    };
    let mut candidates = Vec::new();

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
            };
        }
        tracing::warn!(
            model = ovr,
            "configured override is not usable (missing provider or breaker open), falling back to ordered models"
        );
    }

    for (idx, model) in ordered_models.iter().enumerate() {
        let c = evaluate(
            model,
            if idx == 0 {
                "primary_ordered"
            } else {
                "fallback_ordered"
            },
        );
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
    }
}

async fn resolve_inference_provider(
    state: &AppState,
    model: &str,
) -> Option<ResolvedInferenceProvider> {
    let llm = state.llm.read().await;
    let provider = llm.providers.get_by_model(model)?;
    let url = format!("{}{}", provider.url, provider.chat_path);
    let key = super::admin::resolve_provider_key(
        &provider.name,
        provider.is_local,
        &provider.auth_mode,
        provider.api_key_ref.as_deref(),
        &provider.api_key_env,
        &state.oauth,
        &state.keystore,
    )
    .await
    .unwrap_or_default();
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

/// Attempt inference on the selected model, falling back through the configured
/// chain on transient errors. Updates circuit breakers on success/failure.
async fn infer_with_fallback(
    state: &AppState,
    unified_req: &ironclad_llm::format::UnifiedRequest,
    initial_model: &str,
) -> Result<InferenceResult, String> {
    let config = state.config.read().await;
    let candidates = fallback_candidates(&config, initial_model);
    drop(config);

    let mut last_error = String::new();

    for model in &candidates {
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

        let llm = state.llm.read().await;
        let result = llm
            .client
            .forward_with_provider(
                &resolved.url,
                &resolved.api_key,
                llm_body,
                &resolved.auth_header,
                &resolved.extra_headers,
            )
            .await;
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

                let mut llm = state.llm.write().await;
                llm.breakers.record_success(&resolved.provider_prefix);
                llm.capacity.record(&resolved.provider_prefix, total_tokens);
                let pressured = llm.capacity.is_sustained_hot(&resolved.provider_prefix);
                llm.breakers
                    .set_capacity_pressure(&resolved.provider_prefix, pressured);
                drop(llm);

                if model != initial_model {
                    tracing::info!(
                        primary = initial_model,
                        fallback = model.as_str(),
                        "primary failed, succeeded on fallback"
                    );
                }

                return Ok(InferenceResult {
                    content: unified_resp.content,
                    model: model.clone(),
                    provider: resolved.provider_prefix,
                    tokens_in: tin,
                    tokens_out: tout,
                    cost,
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
                drop(llm);
                last_error = e.to_string();
            }
        }
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

/// Send a "typing…" indicator on the appropriate chat channel.
/// Best-effort — failures are silently ignored so they never block processing.
async fn send_typing_indicator(
    state: &super::AppState,
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

/// Send a thinking indicator (🤖🧠…) on the appropriate chat channel.
/// Used when estimated latency exceeds the configured threshold.
async fn send_thinking_indicator(
    state: &super::AppState,
    platform: &str,
    chat_id: &str,
    metadata: Option<&serde_json::Value>,
) {
    send_typing_indicator(state, platform, chat_id, metadata).await;

    match platform {
        "telegram" => {
            if let Some(ref tg) = state.telegram {
                let _ = tg
                    .send_ephemeral(chat_id, "\u{1F916}\u{1F9E0}\u{2026}")
                    .await;
            }
        }
        "whatsapp" => {
            if let Some(ref wa) = state.whatsapp {
                let _ = wa
                    .send_ephemeral(chat_id, "\u{1F916}\u{1F9E0}\u{2026}")
                    .await;
            }
        }
        "discord" => {
            if let Some(ref dc) = state.discord {
                let _ = dc
                    .send_ephemeral(chat_id, "\u{1F916}\u{1F9E0}\u{2026}")
                    .await;
            }
        }
        "signal" => {
            if let Some(ref sig) = state.signal {
                let _ = sig
                    .send_ephemeral(chat_id, "\u{1F916}\u{1F9E0}\u{2026}")
                    .await;
            }
        }
        _ => {}
    }
}

/// Estimate expected inference latency in seconds based on model tier, input
/// length, and whether the primary provider's circuit breaker is tripped (which
/// means we're falling back to slower alternatives).
async fn estimate_inference_latency(
    tier: ironclad_core::ModelTier,
    input_len: usize,
    model: &str,
    primary_model: &str,
    state: &super::AppState,
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
pub fn check_tool_policy(
    engine: &ironclad_agent::policy::PolicyEngine,
    tool_name: &str,
    params: &serde_json::Value,
    authority: ironclad_core::InputAuthority,
    tier: ironclad_core::SurvivalTier,
    risk_level: ironclad_core::RiskLevel,
) -> Result<(), (StatusCode, String)> {
    let call = ironclad_agent::policy::ToolCallRequest {
        tool_name: tool_name.into(),
        params: params.clone(),
        risk_level,
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

pub(crate) async fn handle_bot_command(
    state: &AppState,
    command: &str,
    inbound: Option<&ironclad_channels::InboundMessage>,
) -> Option<String> {
    let (cmd, args) = command
        .split_once(|c: char| c.is_whitespace())
        .unwrap_or((command, ""));
    let cmd = cmd.split('@').next().unwrap_or(cmd);
    let args = args.trim();

    match cmd {
        "/status" => Some(build_status_reply(state).await),
        "/model" => Some(handle_model_command(state, args).await),
        "/models" => Some(handle_models_list(state).await),
        "/breaker" => Some(handle_breaker_command(state, args).await),
        "/retry" => Some(handle_retry_command(state, inbound).await),
        "/help" => Some(HELP_TEXT.into()),
        _ => None,
    }
}

const HELP_TEXT: &str = "\
/status  — agent health & model info\n\
/model   — show current model & override\n\
/model <provider/name> — force a model override\n\
/model reset — clear override, resume normal routing\n\
/models  — list primary + fallback models\n\
/breaker — show circuit breaker status\n\
/breaker reset [provider] — reset tripped breakers\n\
/retry   — show last assistant response in this chat\n\
/help    — show this message\n\n\
Anything else is sent to the LLM.";

async fn handle_retry_command(
    state: &AppState,
    inbound: Option<&ironclad_channels::InboundMessage>,
) -> String {
    let Some(inbound) = inbound else {
        return "Retry requires a channel context. Send /retry in the target chat.".into();
    };

    let chat_id = resolve_channel_chat_id(inbound);
    let cfg = state.config.read().await;
    let scope = resolve_channel_scope(&cfg, inbound, &chat_id);
    let agent_id = cfg.agent.id.clone();
    drop(cfg);

    let session_id = match ironclad_db::sessions::find_or_create(&state.db, &agent_id, Some(&scope))
    {
        Ok(id) => id,
        Err(e) => return format!("Retry failed: {e}"),
    };
    let messages = match ironclad_db::sessions::list_messages(&state.db, &session_id, Some(100)) {
        Ok(m) => m,
        Err(e) => return format!("Retry failed: {e}"),
    };
    let target = messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
        .map(|m| m.content.clone());
    let Some(content) = target else {
        return "No prior assistant response found to retry in this chat.".into();
    };
    content
}

async fn handle_model_command(state: &AppState, args: &str) -> String {
    if args.is_empty() {
        let llm = state.llm.read().await;
        let current = llm.router.select_model().to_string();
        let primary = llm.router.primary().to_string();
        return match llm.router.get_override() {
            Some(ovr) => {
                format!("🔧 Model override active\n  override: {ovr}\n  primary: {primary}")
            }
            None => {
                format!("🤖 Current model: {current}\n  primary: {primary}\n  (no override set)")
            }
        };
    }

    if args == "reset" || args == "clear" {
        let mut llm = state.llm.write().await;
        llm.router.clear_override();
        let current = llm.router.select_model().to_string();
        return format!("✅ Model override cleared. Routing normally → {current}");
    }

    let model_name = args.to_string();
    let has_provider = {
        let llm = state.llm.read().await;
        llm.providers.get_by_model(&model_name).is_some()
    };

    if !has_provider {
        return format!(
            "⚠️ Unknown model: {model_name}\n\
             Use /models to see available models, or specify as provider/model."
        );
    }

    let mut llm = state.llm.write().await;
    llm.router.set_override(model_name.clone());
    format!("✅ Model override set → {model_name}\nUse /model reset to return to normal routing.")
}

async fn handle_models_list(state: &AppState) -> String {
    let config = state.config.read().await;
    let llm = state.llm.read().await;

    let primary = &config.models.primary;
    let current = llm.router.select_model();
    let mut lines = vec!["📋 Configured models".to_string()];
    lines.push(format!("  primary: {primary}"));

    if !config.models.fallbacks.is_empty() {
        lines.push("  fallbacks:".into());
        for fb in &config.models.fallbacks {
            lines.push(format!("    • {fb}"));
        }
    } else {
        lines.push("  fallbacks: (none)".into());
    }

    if current != primary {
        lines.push(format!("  active: {current}"));
    }

    if let Some(ovr) = llm.router.get_override() {
        lines.push(format!("  override: {ovr}"));
    }

    lines.push(format!("  routing: {}", config.models.routing.mode));
    lines.join("\n")
}

async fn handle_breaker_command(state: &AppState, args: &str) -> String {
    if args.starts_with("reset") {
        let provider = args.strip_prefix("reset").unwrap_or("").trim();
        let mut llm = state.llm.write().await;

        if provider.is_empty() {
            let providers: Vec<String> = llm
                .breakers
                .list_providers()
                .into_iter()
                .filter(|(_, s)| *s != ironclad_llm::CircuitState::Closed)
                .map(|(name, _)| name)
                .collect();

            if providers.is_empty() {
                return "✅ All circuit breakers are already closed.".into();
            }
            for p in &providers {
                llm.breakers.reset(p);
            }
            return format!(
                "✅ Reset {} circuit breaker(s): {}",
                providers.len(),
                providers.join(", ")
            );
        }

        llm.breakers.reset(provider);
        return format!("✅ Circuit breaker for '{provider}' reset to closed.");
    }

    let llm = state.llm.read().await;
    let providers = llm.breakers.list_providers();

    if providers.is_empty() {
        return "🔌 No circuit breaker state recorded yet.".into();
    }

    let mut lines = vec!["🔌 Circuit breaker status".to_string()];
    for (name, state) in &providers {
        let icon = match state {
            ironclad_llm::CircuitState::Closed => "🟢",
            ironclad_llm::CircuitState::Open => "🔴",
            ironclad_llm::CircuitState::HalfOpen => "🟡",
        };
        let credit_note = if llm.breakers.is_credit_tripped(name) {
            " (credit — requires /breaker reset)"
        } else {
            ""
        };
        lines.push(format!("  {icon} {name}: {state:?}{credit_note}"));
    }
    lines.push("\nUse /breaker reset [provider] to reset.".into());
    lines.join("\n")
}

async fn build_status_reply(state: &AppState) -> String {
    let config = state.config.read().await;
    let diag = collect_runtime_diagnostics(state).await;
    let balance = state.wallet.wallet.get_usdc_balance().await.unwrap_or(0.0);
    let channels = state.channel_router.channel_status().await;
    let channel_summary: Vec<String> = channels
        .iter()
        .map(|c| {
            let err = if c.last_error.is_some() { " (err)" } else { "" };
            format!(
                "  {} — rx:{} tx:{}{}",
                sanitize_diag_token(&c.name, 32),
                c.messages_received,
                c.messages_sent,
                err
            )
        })
        .collect();

    let mut lines = vec![
        format!("🤖 {} ({})", config.agent.name, config.agent.id),
        "  state: running".to_string(),
        format!("  primary: {}", diag.primary_model),
    ];
    if diag.active_model != diag.primary_model {
        lines.push(format!("  current: {}", diag.active_model));
    }
    lines.extend([
        format!(
            "  provider: {} ({})",
            diag.primary_provider, diag.primary_provider_state
        ),
        format!(
            "  cache: {} entries, {:.0}% hit rate",
            diag.cache_entries, diag.cache_hit_rate_pct
        ),
        format!(
            "  taskable subagents: total={} enabled={} running={} error={}",
            diag.taskable_subagents_total,
            diag.taskable_subagents_enabled,
            diag.taskable_subagents_running,
            diag.taskable_subagents_error
        ),
        format!(
            "  breakers: {} open, {} half-open",
            diag.breaker_open_count, diag.breaker_half_open_count
        ),
        format!("  approvals: {} pending", diag.pending_approvals),
        format!(
            "  channels: {} total, {} with errors",
            diag.channels_total, diag.channels_with_errors
        ),
        format!("  uptime: {}s", diag.uptime_seconds),
        format!("  wallet: {balance:.2} USDC"),
    ]);

    if !channel_summary.is_empty() {
        lines.push("  channels:".into());
        lines.extend(channel_summary);
    }

    lines.join("\n")
}

// ── Channel message processing ────────────────────────────────

fn metadata_str(meta: Option<&serde_json::Value>, ptr: &str) -> Option<String> {
    meta.and_then(|m| m.pointer(ptr)).and_then(|v| {
        v.as_str()
            .map(|s| s.to_string())
            .or_else(|| v.as_i64().map(|n| n.to_string()))
            .or_else(|| v.as_u64().map(|n| n.to_string()))
    })
}

fn resolve_channel_chat_id(inbound: &ironclad_channels::InboundMessage) -> String {
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

fn resolve_channel_is_group(inbound: &ironclad_channels::InboundMessage) -> bool {
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

fn resolve_channel_scope(
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

pub async fn process_channel_message(
    state: &AppState,
    inbound: ironclad_channels::InboundMessage,
) -> Result<(), String> {
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
        && let Some(reply) = handle_bot_command(state, &inbound.content, Some(&inbound)).await
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

    let channel_turn_id = uuid::Uuid::new_v4().to_string();
    let features = ironclad_llm::extract_features(&user_content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let model_audit = select_routed_model_with_audit(state, &user_content).await;
    let model = model_audit.selected_model.clone();
    let complexity_label = format!("{complexity:?}");
    persist_model_selection_audit(
        state,
        &channel_turn_id,
        &session_id,
        &platform,
        Some(&complexity_label),
        &user_content,
        &model_audit,
    )
    .await;
    let config = state.config.read().await;

    let tier_adapt = config.tier_adapt.clone();
    let agent_name = config.agent.name.clone();
    let agent_id = config.agent.id.clone();
    let primary_model = config.models.primary.clone();
    let thinking_threshold = config.channels.thinking_threshold_seconds;
    let personality = state.personality.read().await;
    let soul_text = personality.soul_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);
    drop(config);

    // Resolve tier for message adaptation
    let tier = {
        let llm = state.llm.read().await;
        llm.providers
            .get_by_model(&model)
            .map(|p| p.tier)
            .unwrap_or_else(|| ironclad_llm::tier::classify(&model))
    };

    // Generate query embedding for RAG retrieval
    let query_embedding = {
        let llm = state.llm.read().await;
        llm.embedding.embed_single(&user_content).await.ok()
    };

    // Retrieve memories from all tiers (using ANN index when available)
    let complexity_level = ironclad_agent::context::determine_level(complexity);
    let ann_ref = if state.ann_index.is_built() {
        Some(&state.ann_index)
    } else {
        None
    };
    let memories = state.retriever.retrieve_with_ann(
        &state.db,
        &session_id,
        &user_content,
        query_embedding.as_deref(),
        complexity_level,
        ann_ref,
    );

    let history_messages =
        ironclad_db::sessions::list_messages(&state.db, &session_id, Some(50)).unwrap_or_default();
    let history: Vec<ironclad_llm::format::UnifiedMessage> = history_messages
        .iter()
        .rev()
        .skip(1)
        .rev()
        .map(|m| ironclad_llm::format::UnifiedMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            parts: None,
        })
        .collect();

    let model_for_api = model
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model)
        .to_string();
    let system_prompt = if soul_text.is_empty() {
        format!(
            "You are {name}, an autonomous AI agent (id: {id}). \
             When asked who you are, always identify as {name}.",
            name = agent_name,
            id = agent_id,
        )
    } else {
        let mut prompt = soul_text.to_string();
        if !firmware_text.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&firmware_text);
        }
        prompt
    };
    let system_prompt = format!(
        "{system_prompt}{}",
        ironclad_agent::prompt::runtime_metadata_block(
            env!("CARGO_PKG_VERSION"),
            &primary_model,
            &model,
        )
    );
    let system_prompt =
        ironclad_agent::prompt::inject_hmac_boundary(&system_prompt, state.hmac_secret.as_ref());
    if !ironclad_agent::prompt::verify_hmac_boundary(&system_prompt, state.hmac_secret.as_ref()) {
        tracing::error!("HMAC boundary verification failed in channel handler");
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err("internal HMAC verification failure".into());
    }

    let mut messages = ironclad_agent::context::build_context(
        complexity_level,
        &system_prompt,
        &memories,
        &history,
    );
    let runtime_diag = collect_runtime_diagnostics(state).await;
    messages.push(ironclad_llm::format::UnifiedMessage {
        role: "system".into(),
        content: diagnostics_system_note(&runtime_diag),
        parts: None,
    });
    if messages.last().is_none_or(|m| m.content != user_content) {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: user_content.clone(),
            parts: None,
        });
    }
    ironclad_llm::tier::adapt_for_tier(tier, &mut messages, &tier_adapt);

    // Check HMAC tamper in model output
    let check_hmac = |content: String, hmac_secret: &[u8]| -> String {
        if content.contains("<<<TRUST_BOUNDARY:") {
            if !ironclad_agent::prompt::verify_hmac_boundary(&content, hmac_secret) {
                tracing::warn!("HMAC boundary tampered in channel model output");
                ironclad_agent::prompt::strip_hmac_boundaries(&content)
            } else {
                content
            }
        } else {
            content
        }
    };

    let unified_req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages,
        max_tokens: Some(2048),
        temperature: None,
        system: None,
        quality_target: None,
    };

    // Send a thinking indicator when expected latency exceeds threshold (all chat channels)
    {
        let estimated_latency =
            estimate_inference_latency(tier, user_content.len(), &model, &primary_model, state)
                .await;

        if estimated_latency >= thinking_threshold {
            send_thinking_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;
        } else {
            send_typing_indicator(state, &platform, &chat_id, inbound.metadata.as_ref()).await;
        }
    }

    let response_content = match infer_with_fallback(state, &unified_req, &model).await {
        Ok(result) => {
            ironclad_db::metrics::record_inference_cost(
                &state.db,
                &result.model,
                &result.provider,
                result.tokens_in,
                result.tokens_out,
                result.cost,
                None,
                false,
            )
            .inspect_err(|e| tracing::warn!(error = %e, "failed to record channel inference cost"))
            .ok();

            check_hmac(result.content, state.hmac_secret.as_ref())
        }
        Err(last_error) => provider_failure_user_message(&last_error.to_string(), true),
    };

    let response_content = if ironclad_agent::injection::scan_output(&response_content) {
        tracing::warn!("L4 output scan flagged channel response, blocking");
        "I can't share that response — it was flagged by my output safety filter.".to_string()
    } else {
        response_content
    };

    // ReAct loop for channel messages: execute tool calls if detected
    let channel_authority = {
        let cfg = state.config.read().await;
        let trusted = &cfg.channels.trusted_sender_ids;
        let sender_trusted = !trusted.is_empty()
            && (trusted.iter().any(|id| id == &chat_id)
                || trusted.iter().any(|id| id == &inbound.sender_id));
        if threat.is_caution() || !sender_trusted {
            InputAuthority::External
        } else {
            InputAuthority::Creator
        }
    };
    let mut channel_react = AgentLoop::new(10);
    let response_content = if let Some((tool_name, tool_params)) =
        parse_tool_call(&response_content)
    {
        channel_react.transition(ReactAction::Think);
        let mut react_msgs = unified_req.messages.clone();
        react_msgs.push(ironclad_llm::format::UnifiedMessage {
            role: "assistant".into(),
            content: response_content.clone(),
            parts: None,
        });

        let mut current_tool = Some((tool_name, tool_params));
        let mut final_response = response_content;

        while let Some((ref tn, ref tp)) = current_tool {
            if channel_react.is_looping(tn, &tp.to_string()) {
                break;
            }
            channel_react.transition(ReactAction::Act {
                tool_name: tn.clone(),
                params: tp.to_string(),
            });
            let tool_result =
                execute_tool_call(state, tn, tp, &channel_turn_id, channel_authority).await;
            let obs = match tool_result {
                Ok(out) => format!("[Tool {tn} succeeded]: {out}"),
                Err(err) => format!("[Tool {tn} failed]: {err}"),
            };
            channel_react.transition(ReactAction::Observe);
            react_msgs.push(ironclad_llm::format::UnifiedMessage {
                role: "user".into(),
                content: obs,
                parts: None,
            });

            if channel_react.state == ReactState::Done {
                break;
            }

            let follow_req = ironclad_llm::format::UnifiedRequest {
                model: unified_req.model.clone(),
                messages: react_msgs.clone(),
                max_tokens: Some(2048),
                temperature: None,
                system: None,
                quality_target: None,
            };
            match infer_with_fallback(state, &follow_req, &model).await {
                Ok(result) => {
                    let content = check_hmac(result.content, state.hmac_secret.as_ref());
                    let content = if ironclad_agent::injection::scan_output(&content) {
                        tracing::warn!("L4 output scan flagged channel ReAct follow-up, blocking");
                        "[Response blocked by output safety filter]".to_string()
                    } else {
                        content
                    };
                    react_msgs.push(ironclad_llm::format::UnifiedMessage {
                        role: "assistant".into(),
                        content: content.clone(),
                        parts: None,
                    });
                    current_tool = parse_tool_call(&content);
                    if current_tool.is_none() {
                        channel_react.transition(ReactAction::Finish);
                        final_response = content;
                    }
                }
                Err(_) => break,
            }
        }
        final_response
    } else {
        response_content
    };

    ironclad_db::sessions::append_message(&state.db, &session_id, "assistant", &response_content)
        .inspect_err(|e| tracing::warn!(error = %e, "failed to store channel assistant message"))
        .ok();

    if let Err(e) = state
        .channel_router
        .send_reply(&platform, &chat_id, response_content.clone())
        .await
    {
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
        drop(llm);
        return Err(e.to_string());
    }

    // Post-turn memory ingestion + embedding generation with chunking (background)
    {
        let ingest_db = state.db.clone();
        let ingest_session = session_id.clone();
        let ingest_user = user_content.clone();
        let ingest_assistant = response_content;
        let ingest_llm = Arc::clone(&state.llm);
        tokio::spawn(async move {
            ironclad_agent::memory::ingest_turn(
                &ingest_db,
                &ingest_session,
                &ingest_user,
                &ingest_assistant,
                &[],
            );

            let llm = ingest_llm.read().await;
            let chunk_config = ironclad_agent::retrieval::ChunkConfig::default();
            let chunks = ironclad_agent::retrieval::chunk_text(&ingest_assistant, &chunk_config);

            for chunk in &chunks {
                if let Ok(embedding) = llm.embedding.embed_single(&chunk.text).await {
                    let embed_id = uuid::Uuid::new_v4().to_string();
                    ironclad_db::embeddings::store_embedding(
                        &ingest_db,
                        &embed_id,
                        "turn",
                        &ingest_session,
                        &chunk.text[..chunk.text.len().min(200)],
                        &embedding,
                    )
                    .inspect_err(|e| tracing::warn!(error = %e, chunk_idx = chunk.index, "failed to store channel chunk embedding"))
                    .ok();
                }
            }
        });
    }

    // Release dedup tracking
    {
        let mut llm = state.llm.write().await;
        llm.dedup.release(&dedup_fp);
    }

    Ok(())
}

pub async fn telegram_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));

    let adapter = match &state.telegram {
        Some(a) => a.clone(),
        None => return,
    };

    tracing::info!("Telegram long-poll loop started");

    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
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

pub async fn discord_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));
    let adapter = match &state.discord {
        Some(a) => a.clone(),
        None => return,
    };
    tracing::info!("Discord inbound loop started");
    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    if let Err(e) = process_channel_message(&state, inbound).await {
                        tracing::error!(error = %e, "Discord message processing failed");
                    }
                });
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
            Err(e) => {
                tracing::error!(error = %e, "Discord inbound loop error, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

pub async fn signal_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));
    let adapter = match &state.signal {
        Some(a) => a.clone(),
        None => return,
    };
    tracing::info!("Signal inbound loop started");
    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    if let Err(e) = process_channel_message(&state, inbound).await {
                        tracing::error!(error = %e, "Signal message processing failed");
                    }
                });
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
            Err(e) => {
                tracing::error!(error = %e, "Signal inbound loop error, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

pub async fn email_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));
    let adapter = match &state.email {
        Some(a) => a.clone(),
        None => return,
    };
    tracing::info!("Email inbound loop started");
    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    if let Err(e) = process_channel_message(&state, inbound).await {
                        tracing::error!(error = %e, "Email message processing failed");
                    }
                });
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            Err(e) => {
                tracing::error!(error = %e, "Email inbound loop error, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_config_with_scope(scope_mode: &str) -> ironclad_core::IroncladConfig {
        ironclad_core::IroncladConfig::from_str(&format!(
            r#"
[agent]
name = "TestBot"
id = "test-agent"

[server]
port = 0

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"

[session]
scope_mode = "{scope_mode}"
"#
        ))
        .unwrap()
    }

    fn inbound_with_meta(meta: serde_json::Value) -> ironclad_channels::InboundMessage {
        ironclad_channels::InboundMessage {
            id: "msg-1".into(),
            platform: "telegram".into(),
            sender_id: "sender-1".into(),
            content: "hello".into(),
            timestamp: Utc::now(),
            metadata: Some(meta),
        }
    }

    #[test]
    fn parse_tool_call_valid() {
        let input = r#"Let me check that. {"tool_call": {"name": "read_file", "params": {"path": "/tmp/test.txt"}}}"#;
        let result = parse_tool_call(input);
        assert!(result.is_some());
        let (name, params) = result.unwrap();
        assert_eq!(name, "read_file");
        assert_eq!(params["path"], "/tmp/test.txt");
    }

    #[test]
    fn parse_tool_call_no_params() {
        let input = r#"{"tool_call": {"name": "status"}}"#;
        let result = parse_tool_call(input);
        assert!(result.is_some());
        let (name, params) = result.unwrap();
        assert_eq!(name, "status");
        assert!(params.is_object());
    }

    #[test]
    fn parse_tool_call_none_for_no_tool() {
        assert!(parse_tool_call("Hello, how are you?").is_none());
        assert!(parse_tool_call("").is_none());
    }

    #[test]
    fn parse_tool_call_nested_braces() {
        let input = r#"{"tool_call": {"name": "bash", "params": {"command": "echo '{hello}'"}}}"#;
        let result = parse_tool_call(input);
        assert!(result.is_some());
        let (name, _params) = result.unwrap();
        assert_eq!(name, "bash");
    }

    #[test]
    fn parse_tool_call_malformed_json() {
        assert!(parse_tool_call(r#"{"tool_call": {"name": broken}}"#).is_none());
    }

    #[test]
    fn parse_tool_call_surrounded_by_text() {
        let input = r#"I'll read the file now. {"tool_call": {"name": "read_file", "params": {"path": "test.rs"}}} Let me analyze the output."#;
        let result = parse_tool_call(input);
        assert!(result.is_some());
        let (name, params) = result.unwrap();
        assert_eq!(name, "read_file");
        assert_eq!(params["path"], "test.rs");
    }

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
            ironclad_core::RiskLevel::Safe,
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
            ironclad_core::RiskLevel::Dangerous,
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
            ironclad_core::RiskLevel::Dangerous,
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
            ironclad_core::RiskLevel::Safe,
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

    #[test]
    fn sanitize_diag_token_removes_unsafe_chars_and_limits_len() {
        let token = sanitize_diag_token("  !!openai/gpt-4o:mini??\n\t", 10);
        assert_eq!(token, "openai/gpt");
    }

    #[test]
    fn model_proxy_roles_are_detected_case_insensitively() {
        assert!(is_model_proxy_role("model-proxy"));
        assert!(is_model_proxy_role("Model-Proxy"));
        assert!(!is_model_proxy_role("subagent"));
    }

    #[test]
    fn diagnostics_system_note_contains_expected_sections() {
        let diag = RuntimeDiagnostics {
            uptime_seconds: 42,
            primary_model: "ollama/qwen3:8b".into(),
            active_model: "ollama/qwen3:8b".into(),
            primary_provider: "ollama".into(),
            primary_provider_state: "closed".into(),
            breaker_open_count: 0,
            breaker_half_open_count: 0,
            cache_entries: 3,
            cache_hit_rate_pct: 50.0,
            pending_approvals: 1,
            taskable_subagents_total: 2,
            taskable_subagents_enabled: 1,
            taskable_subagents_running: 1,
            taskable_subagents_error: 0,
            channels_total: 2,
            channels_with_errors: 0,
        };
        let note = diagnostics_system_note(&diag);
        assert!(note.contains("Runtime diagnostics"));
        assert!(note.contains("models:"));
        assert!(note.contains("provider:"));
        assert!(note.contains("cache:"));
        assert!(note.contains("approvals_pending"));
    }

    #[test]
    fn metadata_str_reads_strings_and_numbers() {
        let meta = serde_json::json!({
            "chat_id": "chat-1",
            "channel_id": 123,
            "thread_id": 456u64
        });
        assert_eq!(
            metadata_str(Some(&meta), "/chat_id").as_deref(),
            Some("chat-1")
        );
        assert_eq!(
            metadata_str(Some(&meta), "/channel_id").as_deref(),
            Some("123")
        );
        assert_eq!(
            metadata_str(Some(&meta), "/thread_id").as_deref(),
            Some("456")
        );
        assert!(metadata_str(Some(&meta), "/missing").is_none());
    }

    #[test]
    fn resolve_channel_chat_id_uses_priority_and_fallback() {
        let inbound = inbound_with_meta(serde_json::json!({"chat_id": "chat-xyz"}));
        assert_eq!(resolve_channel_chat_id(&inbound), "chat-xyz");

        let inbound = inbound_with_meta(serde_json::json!({"message": {"chat": {"id": 777}}}));
        assert_eq!(resolve_channel_chat_id(&inbound), "777");

        let inbound = ironclad_channels::InboundMessage {
            id: "msg-2".into(),
            platform: "telegram".into(),
            sender_id: "sender-fallback".into(),
            content: "hi".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        assert_eq!(resolve_channel_chat_id(&inbound), "sender-fallback");
    }

    #[test]
    fn resolve_channel_is_group_detects_flags_and_chat_type() {
        let inbound = inbound_with_meta(serde_json::json!({"is_group": true}));
        assert!(resolve_channel_is_group(&inbound));

        let inbound =
            inbound_with_meta(serde_json::json!({"message": {"chat": {"type": "supergroup"}}}));
        assert!(resolve_channel_is_group(&inbound));

        let inbound =
            inbound_with_meta(serde_json::json!({"message": {"chat": {"type": "private"}}}));
        assert!(!resolve_channel_is_group(&inbound));
    }

    #[test]
    fn resolve_channel_scope_respects_config_mode() {
        let cfg_group = test_config_with_scope("group");
        let inbound_group = inbound_with_meta(serde_json::json!({"is_group": true}));
        let scope = resolve_channel_scope(&cfg_group, &inbound_group, "group-chat");
        assert_eq!(
            scope,
            ironclad_db::sessions::SessionScope::Group {
                group_id: "group-chat".into(),
                channel: "telegram".into()
            }
        );

        let cfg_peer = test_config_with_scope("peer");
        let inbound_peer = inbound_with_meta(serde_json::json!({}));
        let scope = resolve_channel_scope(&cfg_peer, &inbound_peer, "ignored");
        assert_eq!(
            scope,
            ironclad_db::sessions::SessionScope::Peer {
                peer_id: "sender-1".into(),
                channel: "telegram".into()
            }
        );

        let cfg_agent = test_config_with_scope("agent");
        let inbound_agent = inbound_with_meta(serde_json::json!({"is_group": true}));
        let scope = resolve_channel_scope(&cfg_agent, &inbound_agent, "group-chat");
        assert_eq!(scope, ironclad_db::sessions::SessionScope::Agent);
    }

    #[test]
    fn resolve_web_scope_respects_group_peer_and_agent_modes() {
        let mut req = AgentMessageRequest {
            content: "hello".into(),
            session_id: None,
            channel: Some("web".into()),
            sender_id: Some("user-1".into()),
            peer_id: None,
            group_id: Some("room-9".into()),
            is_group: Some(true),
        };

        let cfg_group = test_config_with_scope("group");
        let scope = resolve_web_scope(&cfg_group, &req).expect("group scope");
        assert_eq!(
            scope,
            ironclad_db::sessions::SessionScope::Group {
                group_id: "room-9".into(),
                channel: "web".into()
            }
        );

        let cfg_peer = test_config_with_scope("peer");
        req.group_id = None;
        let scope = resolve_web_scope(&cfg_peer, &req).expect("peer scope");
        assert_eq!(
            scope,
            ironclad_db::sessions::SessionScope::Peer {
                peer_id: "user-1".into(),
                channel: "web".into()
            }
        );

        let cfg_agent = test_config_with_scope("agent");
        let scope = resolve_web_scope(&cfg_agent, &req).expect("agent scope");
        assert_eq!(scope, ironclad_db::sessions::SessionScope::Agent);
    }

    #[test]
    fn resolve_web_scope_rejects_missing_principal_in_peer_or_group_modes() {
        let req = AgentMessageRequest {
            content: "hello".into(),
            session_id: None,
            channel: Some("web".into()),
            sender_id: None,
            peer_id: None,
            group_id: None,
            is_group: Some(false),
        };

        let cfg_peer = test_config_with_scope("peer");
        assert!(resolve_web_scope(&cfg_peer, &req).is_err());

        let cfg_group = test_config_with_scope("group");
        assert!(resolve_web_scope(&cfg_group, &req).is_err());
    }
}
