//! Tool call parsing, execution dispatch, and provider error classification.

use axum::http::StatusCode;
use ironclad_agent::script_runner::ScriptRunner;
use ironclad_agent::tools::ToolContext;
use ironclad_core::InputAuthority;
use serde_json::json;

use super::super::JsonError;
use super::AppState;

/// Try to extract a tool call from the LLM's text response.
/// Looks for `{"tool_call": {"name": "...", "params": {...}}}` in the response.
pub(super) fn parse_tool_call(response: &str) -> Option<(String, serde_json::Value)> {
    // Search from the end to avoid locking onto a fake "tool_call" earlier in the text.
    // Try each candidate from last to first; accept the first valid parse.
    let mut search_end = response.len();
    while let Some(rel) = response[..search_end].rfind(r#""tool_call""#) {
        if let Some(brace_start) = response[..rel].rfind('{') {
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
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str)
                && let Some((name, params)) = extract_tool_invocation(&parsed)
            {
                return Some((name, params));
            }
        }
        search_end = rel;
    }
    None
}

fn extract_tool_invocation(parsed: &serde_json::Value) -> Option<(String, serde_json::Value)> {
    let tool_call = parsed.get("tool_call")?;

    if let Some(name) = tool_call.get("name").and_then(|n| n.as_str()) {
        let params = tool_call.get("params").cloned().unwrap_or(json!({}));
        return Some((name.to_string(), params));
    }

    // Accept shorthand shape:
    // {"tool_call":"bash","params":{"command":"ls"}}
    if let Some(name) = tool_call.as_str() {
        let params = parsed.get("params").cloned().unwrap_or(json!({}));
        return Some((name.to_string(), params));
    }

    None
}

/// Extract **all** tool calls from the LLM's text response.
///
/// Scans forward through the text for `{"tool_call": {"name": "...", "params": {...}}}`
/// blocks and returns them in order. This handles the case where the LLM (or the shim)
/// emits multiple tool calls separated by newlines.
pub(super) fn parse_tool_calls(response: &str) -> Vec<(String, serde_json::Value)> {
    let mut results = Vec::new();
    let mut search_start = 0;
    while search_start < response.len() {
        let Some(rel) = response[search_start..].find(r#""tool_call""#) else {
            break;
        };
        let abs_pos = search_start + rel;
        // Walk backwards to find the opening brace
        let Some(brace_start) = response[..abs_pos].rfind('{') else {
            search_start = abs_pos + 1;
            continue;
        };
        // But only accept it if there's no intervening closing brace (this brace
        // belongs to the tool_call, not a prior JSON object).
        if response[brace_start + 1..abs_pos].contains('}') {
            // The opening brace belongs to an earlier JSON — try the next `{` forward
            let fallback_start = response[abs_pos..].find('{').map(|i| abs_pos + i);
            if let Some(fb) = fallback_start
                && fb < abs_pos
            {
                search_start = abs_pos + 1;
                continue;
            }
            search_start = abs_pos + 1;
            continue;
        }

        // Find the matching closing brace
        let mut depth = 0;
        let mut end = brace_start;
        let mut found_end = false;
        for (i, ch) in response[brace_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = brace_start + i + 1;
                        found_end = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !found_end {
            // Unterminated JSON object; stop scanning to avoid looping forever.
            break;
        }

        let json_str = &response[brace_start..end];
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str)
            && let Some((name, params)) = extract_tool_invocation(&parsed)
        {
            results.push((name, params));
        }
        search_start = end;
    }
    results
}

pub(crate) fn classify_provider_error(raw: &str) -> &'static str {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("circuit breaker") {
        "provider temporarily unavailable"
    } else if lower.contains("no api key") || lower.contains("no provider configured") {
        "no provider configured for this model"
    } else if lower.contains("401") || lower.contains("403") || lower.contains("authentication") {
        "provider authentication error"
    } else if lower.contains("429") || lower.contains("rate limit") || lower.contains("rate_limit")
    {
        "provider rate limit reached"
    } else if lower.contains("402")
        || lower.contains("quota")
        || lower.contains("billing")
        || lower.contains("credit")
    {
        "provider quota or billing issue"
    } else if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
    {
        "provider server error"
    } else if lower.contains("request failed")
        || lower.contains("timeout")
        || lower.contains("connection")
    {
        "network error reaching provider"
    } else {
        "provider error"
    }
}

pub(super) fn provider_failure_user_message(
    last_error: &str,
    message_already_stored: bool,
) -> String {
    let category = classify_provider_error(last_error);
    if message_already_stored {
        format!(
            "Acknowledged. I hit provider routing failure across all LLM providers ({category}). \
             Your message is stored and I will retry as soon as a provider path is healthy."
        )
    } else {
        format!(
            "Acknowledged. I hit provider routing failure across all LLM providers ({category}). Please retry."
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
    channel: Option<&str>,
) -> Result<String, String> {
    execute_tool_call_internal(state, tool_name, params, turn_id, authority, channel, true).await
}

/// Replay a previously approved tool call.
///
/// This bypasses the approval gate (already satisfied) but still enforces policy and
/// records tool execution/audit trails exactly like normal execution.
pub(crate) async fn execute_tool_call_after_approval(
    state: &AppState,
    tool_name: &str,
    params: &serde_json::Value,
    turn_id: &str,
    authority: InputAuthority,
    channel: Option<&str>,
) -> Result<String, String> {
    execute_tool_call_internal(state, tool_name, params, turn_id, authority, channel, false).await
}

async fn execute_tool_call_internal(
    state: &AppState,
    tool_name: &str,
    params: &serde_json::Value,
    turn_id: &str,
    authority: InputAuthority,
    channel: Option<&str>,
    enforce_approval_gate: bool,
) -> Result<String, String> {
    fn parse_risk_level(raw: &str) -> Result<ironclad_core::RiskLevel, String> {
        match raw.to_ascii_lowercase().as_str() {
            "safe" => Ok(ironclad_core::RiskLevel::Safe),
            "caution" => Ok(ironclad_core::RiskLevel::Caution),
            "dangerous" => Ok(ironclad_core::RiskLevel::Dangerous),
            "forbidden" => Ok(ironclad_core::RiskLevel::Forbidden),
            _ => Err(format!("invalid skill risk_level '{raw}'")),
        }
    }
    let balance = state.wallet.wallet.get_usdc_balance().await.unwrap_or(0.0);
    let tier = ironclad_core::SurvivalTier::from_balance(balance, 0.0);
    if super::is_virtual_delegation_tool(tool_name) {
        let start = std::time::Instant::now();
        let result = super::execute_virtual_subagent_tool_call(
            state, tool_name, params, turn_id, authority, tier,
        )
        .await;
        let duration_ms = start.elapsed().as_millis() as i64;
        let (output, status) = match &result {
            Ok(out) => (out.clone(), "success"),
            Err(err) => (err.clone(), "error"),
        };
        ironclad_db::tools::record_tool_call_with_skill(
            &state.db,
            turn_id,
            tool_name,
            &params.to_string(),
            Some(&output),
            status,
            Some(duration_ms),
            None,
            None,
            None,
        )
        .inspect_err(|e| tracing::warn!(error = %e, "failed to record virtual tool call"))
        .ok();
        return result;
    }
    if super::is_virtual_orchestration_tool(tool_name) {
        let start = std::time::Instant::now();
        let result = super::execute_virtual_orchestration_tool(
            state, tool_name, params, turn_id, authority, tier,
        )
        .await;
        let duration_ms = start.elapsed().as_millis() as i64;
        let (output, status) = match &result {
            Ok(out) => (out.clone(), "success"),
            Err(err) => (err.clone(), "error"),
        };
        ironclad_db::tools::record_tool_call_with_skill(
            &state.db,
            turn_id,
            tool_name,
            &params.to_string(),
            Some(&output),
            status,
            Some(duration_ms),
            None,
            None,
            None,
        )
        .inspect_err(|e| tracing::warn!(error = %e, "failed to record orchestration tool call"))
        .ok();
        return result;
    }
    let tool = match state.tools.get(tool_name) {
        Some(t) => t,
        None => return Err(format!("Unknown tool: {tool_name}")),
    };
    let mut effective_risk = tool.risk_level();
    let mut matched_skill: Option<(String, String, String)> = None;

    if tool_name == "run_script" {
        let script_arg = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let config = state.config.read().await;
        let runner = ScriptRunner::new(config.skills.clone());
        let maybe_script_path = runner
            .resolve_script_path(std::path::Path::new(script_arg))
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        drop(config);

        if let Some(script_path) = maybe_script_path {
            let skill = ironclad_db::skills::find_skill_by_script_path(&state.db, &script_path)
                .map_err(|e| format!("Skill policy lookup failed: {e}"))?;
            if let Some(skill) = skill {
                if !skill.enabled {
                    return Err(format!(
                        "Policy override denied: skill '{}' is disabled",
                        skill.name
                    ));
                }

                effective_risk = parse_risk_level(&skill.risk_level).map_err(|e| {
                    format!("Policy override denied: skill '{}' has {}", skill.name, e)
                })?;
                matched_skill = Some((
                    skill.id.clone(),
                    skill.name.clone(),
                    skill.content_hash.clone(),
                ));

                if let Some(overrides_raw) = skill.policy_overrides_json.as_deref() {
                    let overrides = serde_json::from_str::<serde_json::Value>(overrides_raw)
                        .map_err(|e| {
                            format!(
                                "Policy override parse failed for skill '{}': {e}",
                                skill.name
                            )
                        })?;

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
    }

    if authority == InputAuthority::Creator {
        ironclad_db::policy::record_policy_decision(
            &state.db,
            Some(turn_id),
            tool_name,
            "allow",
            Some("creator_override"),
            Some("Creator authority bypassed policy/approval gates"),
        )
        .inspect_err(|e| tracing::warn!(error = %e, "failed to record creator policy decision"))
        .ok();
    } else {
        let policy_result = super::check_tool_policy(
            &state.policy_engine,
            tool_name,
            params,
            authority,
            tier,
            effective_risk,
        );

        let (decision_str, rule_name, reason) = match &policy_result {
            Ok(()) => ("allow".to_string(), None, None),
            Err(JsonError(_status, msg)) => (
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

        if let Err(JsonError(_status, msg)) = policy_result {
            return Err(format!("Policy denied: {msg}"));
        }
    }

    if enforce_approval_gate && authority != InputAuthority::Creator {
        // Approval gate: block gated tools until a human approves
        match state.approvals.check_tool(tool_name) {
            Ok(ironclad_agent::approvals::ToolClassification::Gated) => {
                let request = state
                    .approvals
                    .request_approval(tool_name, &params.to_string(), Some(turn_id), authority)
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
        channel: channel.map(|s| s.to_string()),
        db: Some(state.db.clone()),
    };

    // BUG-027: Use actual agent_id from config instead of hardcoded "ironclad".
    let ws_agent_id = {
        let config = state.config.read().await;
        config.agent.id.clone()
    };
    state.event_bus.publish(
        serde_json::json!({
            "type": "agent_working",
            "agent_id": ws_agent_id,
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
                "agent_id": ws_agent_id,
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
            "agent_id": ws_agent_id,
            "workstation": "exec",
            "turn_id": turn_id,
        })
        .to_string(),
    );

    result.map(|_| output).map_err(|e| e.message)
}

/// Checks whether a tool call is allowed by the policy engine.
/// Returns Ok(()) if allowed, or an error tuple for HTTP responses.
pub(crate) fn check_tool_policy(
    engine: &ironclad_agent::policy::PolicyEngine,
    tool_name: &str,
    params: &serde_json::Value,
    authority: InputAuthority,
    tier: ironclad_core::SurvivalTier,
    risk_level: ironclad_core::RiskLevel,
) -> Result<(), JsonError> {
    let call = ironclad_agent::policy::ToolCallRequest {
        tool_name: tool_name.into(),
        params: params.clone(),
        risk_level,
    };
    let ctx = ironclad_agent::policy::PolicyContext {
        authority,
        survival_tier: tier,
        claim: None,
    };
    let decision = engine.evaluate_all(&call, &ctx);
    match decision {
        ironclad_core::PolicyDecision::Allow => Ok(()),
        ironclad_core::PolicyDecision::Deny { rule, reason } => {
            tracing::warn!(tool = tool_name, rule = %rule, reason = %reason, "Policy denied tool call");
            Err(JsonError(
                StatusCode::FORBIDDEN,
                format!("Policy denied: {reason}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_calls_single() {
        let input = r#"{"tool_call": {"name": "echo", "params": {"message": "hi"}}}"#;
        let calls = parse_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "echo");
    }

    #[test]
    fn parse_tool_calls_multiple() {
        let input = r#"{"tool_call": {"name": "echo", "params": {"message": "hi"}}}
{"tool_call": {"name": "web-search", "params": {"query": "rust"}}}"#;
        let calls = parse_tool_calls(input);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "echo");
        assert_eq!(calls[1].0, "web-search");
    }

    #[test]
    fn parse_tool_calls_with_surrounding_text() {
        let input = r#"Let me help you with that.
{"tool_call": {"name": "echo", "params": {"message": "test"}}}
I will also search:
{"tool_call": {"name": "web-search", "params": {"query": "rust lang"}}}"#;
        let calls = parse_tool_calls(input);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "echo");
        assert_eq!(calls[1].0, "web-search");
    }

    #[test]
    fn parse_tool_calls_empty() {
        let calls = parse_tool_calls("No tool calls here");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_tool_calls_unterminated_json_stops_cleanly() {
        let input = r#"{"tool_call": {"name": "echo", "params": {"message": "hi"}}"#;
        let calls = parse_tool_calls(input);
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_tool_call_backward_compat() {
        let input = r#"Some text {"tool_call": {"name": "echo", "params": {"message": "hi"}}}"#;
        let single = parse_tool_call(input);
        assert!(single.is_some());
        assert_eq!(single.unwrap().0, "echo");
    }

    #[test]
    fn parse_tool_call_shorthand_shape() {
        let input = r#"{"tool_call":"bash","params":{"command":"ls -la"}}"#;
        let single = parse_tool_call(input).expect("should parse shorthand shape");
        assert_eq!(single.0, "bash");
        assert_eq!(single.1["command"], "ls -la");
    }

    #[test]
    fn parse_tool_calls_shorthand_shape() {
        let input = r#"{"tool_call":"orchestrate-subagents","params":{"task":"sitrep"}}"#;
        let calls = parse_tool_calls(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "orchestrate-subagents");
        assert_eq!(calls[0].1["task"], "sitrep");
    }
}
