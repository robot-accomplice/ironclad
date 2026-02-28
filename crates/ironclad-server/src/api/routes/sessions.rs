use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct FeedbackRequest {
    pub grade: i32,
    pub comment: Option<String>,
}

use ironclad_agent::analyzer::{ContextAnalyzer, SessionData, TurnData};

use super::{
    AppState, JsonError, PaginationQuery, bad_request, internal_err, not_found, sanitize_html,
    validate_long, validate_short,
};

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub agent_id: String,
}

#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub role: String,
    pub content: String,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Query(pagination): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (limit, offset) = pagination.resolve();
    let conn = state.db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.agent_id, s.scope_key, s.status, s.model, s.nickname, s.created_at, s.updated_at, s.metadata, \
                    (SELECT COUNT(1) FROM turns t WHERE t.session_id = s.id) AS turn_count \
             FROM sessions s ORDER BY s.created_at DESC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| internal_err(&e))?;

    let rows = stmt
        .query_map(rusqlite::params![limit, offset], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "agent_id": row.get::<_, String>(1)?,
                "scope_key": row.get::<_, Option<String>>(2)?,
                "status": row.get::<_, String>(3)?,
                "model": row.get::<_, Option<String>>(4)?,
                "nickname": row.get::<_, Option<String>>(5)?,
                "created_at": row.get::<_, String>(6)?,
                "updated_at": row.get::<_, String>(7)?,
                "metadata": row.get::<_, Option<String>>(8)?,
                "turn_count": row.get::<_, i64>(9)?,
            }))
        })
        .map_err(|e| internal_err(&e))?;

    let sessions: Vec<Value> = rows.filter_map(|r| r.ok()).collect();

    Ok::<_, JsonError>(axum::Json(serde_json::json!({ "sessions": sessions })))
}

pub async fn create_session(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateSessionRequest>,
) -> impl IntoResponse {
    validate_short("agent_id", &body.agent_id)?;
    let agent_id = sanitize_html(&body.agent_id);
    // Keep "New session" semantics while preserving active-session consistency.
    let id = match ironclad_db::sessions::rotate_agent_session(&state.db, &agent_id) {
        Ok(id) => id,
        Err(e) => return Err(internal_err(&e)),
    };

    // Return the full session object, not just the ID (BUG-20).
    match ironclad_db::sessions::get_session(&state.db, &id) {
        Ok(Some(s)) => Ok(axum::Json(serde_json::json!({
            "id": s.id,
            "agent_id": s.agent_id,
            "scope_key": s.scope_key,
            "status": s.status,
            "model": s.model,
            "nickname": s.nickname,
            "created_at": s.created_at,
            "updated_at": s.updated_at,
            "metadata": s.metadata,
        }))),
        Ok(None) => Err(internal_err(&format_args!("created session {id} vanished"))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::sessions::get_session(&state.db, &id) {
        Ok(Some(s)) => Ok(axum::Json(serde_json::json!({
            "id": s.id,
            "agent_id": s.agent_id,
            "scope_key": s.scope_key,
            "status": s.status,
            "model": s.model,
            "nickname": s.nickname,
            "created_at": s.created_at,
            "updated_at": s.updated_at,
            "metadata": s.metadata,
        }))),
        Ok(None) => Err(not_found(format!("session {id} not found"))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(pagination): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (limit, _offset) = pagination.resolve();
    match ironclad_db::sessions::list_messages(&state.db, &id, Some(limit)) {
        Ok(msgs) => {
            let items: Vec<Value> = msgs
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "session_id": m.session_id,
                        "parent_id": m.parent_id,
                        "role": m.role,
                        "content": m.content,
                        "usage_json": m.usage_json,
                        "created_at": m.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "messages": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

const ALLOWED_ROLES: &[&str] = &["user", "assistant"];

pub async fn post_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<PostMessageRequest>,
) -> impl IntoResponse {
    validate_short("role", &body.role)?;
    validate_long("content", &body.content)?;
    if !ALLOWED_ROLES.contains(&body.role.as_str()) {
        return Err(bad_request(format!(
            "invalid role '{}': must be one of {:?}",
            body.role, ALLOWED_ROLES
        )));
    }

    match ironclad_db::sessions::get_session(&state.db, &id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Err(not_found(format!("session '{id}' not found")));
        }
        Err(e) => return Err(internal_err(&e)),
    }

    match ironclad_db::sessions::append_message(&state.db, &id, &body.role, &body.content) {
        Ok(msg_id) => Ok(axum::Json(serde_json::json!({ "message_id": msg_id }))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn backfill_nicknames(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::sessions::backfill_nicknames(&state.db) {
        Ok(count) => Ok(axum::Json(serde_json::json!({ "backfilled": count }))),
        Err(e) => Err(internal_err(&e)),
    }
}

// ── Turn & context API endpoints ────────────────────────────────

pub async fn list_session_turns(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::sessions::list_turns_for_session(&state.db, &id) {
        Ok(turns) => {
            let items: Vec<Value> = turns
                .into_iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "session_id": t.session_id,
                        "thinking": t.thinking,
                        "tokens_in": t.tokens_in,
                        "tokens_out": t.tokens_out,
                        "cost": t.cost,
                        "model": t.model,
                        "created_at": t.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "turns": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_turn(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match ironclad_db::sessions::get_turn_by_id(&state.db, &id) {
        Ok(Some(t)) => Ok(axum::Json(serde_json::json!({
            "id": t.id,
            "session_id": t.session_id,
            "thinking": t.thinking,
            "tokens_in": t.tokens_in,
            "tokens_out": t.tokens_out,
            "cost": t.cost,
            "model": t.model,
            "created_at": t.created_at,
        }))),
        Ok(None) => Err(not_found(format!("turn {id} not found"))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_turn_model_selection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::model_selection::get_model_selection_by_turn_id(&state.db, &id) {
        Ok(Some(row)) => {
            let candidates: Vec<serde_json::Value> =
                serde_json::from_str(&row.candidates_json)
                    .inspect_err(|e| tracing::warn!(turn_id = %id, error = %e, "corrupt candidates JSON in model selection"))
                    .unwrap_or_default();
            Ok(axum::Json(serde_json::json!({
                "event_id": row.id,
                "turn_id": row.turn_id,
                "session_id": row.session_id,
                "agent_id": row.agent_id,
                "channel": row.channel,
                "selected_model": row.selected_model,
                "strategy": row.strategy,
                "primary_model": row.primary_model,
                "override_model": row.override_model,
                "complexity": row.complexity,
                "user_excerpt": row.user_excerpt,
                "candidates": candidates,
                "created_at": row.created_at,
            })))
        }
        Ok(None) => Err(not_found(format!("no model selection trace for turn {id}"))),
        Err(e) => Err(internal_err(&e)),
    }
}

#[derive(Deserialize)]
pub struct ModelSelectionListQuery {
    pub limit: Option<usize>,
}

pub async fn list_model_selection_events(
    State(state): State<AppState>,
    Query(query): Query<ModelSelectionListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    match ironclad_db::model_selection::list_model_selection_events(&state.db, limit) {
        Ok(rows) => {
            let events: Vec<Value> = rows
                .into_iter()
                .map(|row| {
                    let candidates: Vec<serde_json::Value> =
                        serde_json::from_str(&row.candidates_json)
                            .inspect_err(|e| tracing::warn!(event_id = %row.id, error = %e, "corrupt candidates JSON in model selection"))
                            .unwrap_or_default();
                    serde_json::json!({
                        "event_id": row.id,
                        "turn_id": row.turn_id,
                        "session_id": row.session_id,
                        "agent_id": row.agent_id,
                        "channel": row.channel,
                        "selected_model": row.selected_model,
                        "strategy": row.strategy,
                        "primary_model": row.primary_model,
                        "override_model": row.override_model,
                        "complexity": row.complexity,
                        "user_excerpt": row.user_excerpt,
                        "candidates": candidates,
                        "created_at": row.created_at,
                    })
                })
                .collect();
            let count = events.len();
            Ok(axum::Json(serde_json::json!({
                "events": events,
                "count": count,
            })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_turn_context(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::sessions::get_turn_by_id(&state.db, &id) {
        Ok(Some(t)) => {
            let tool_calls = ironclad_db::tools::get_tool_calls_for_turn(&state.db, &id)
                .map_err(|e| internal_err(&e))?;
            Ok(axum::Json(serde_json::json!({
                "turn_id": t.id,
                "model": t.model,
                "token_budget": 0,
                "system_prompt_tokens": 0,
                "memory_tokens": 0,
                "history_tokens": 0,
                "history_depth": 0,
                "complexity_level": "L1",
                "tokens_in": t.tokens_in,
                "tokens_out": t.tokens_out,
                "cost": t.cost,
                "tool_call_count": tool_calls.len(),
                "tool_failure_count": tool_calls.iter().filter(|tc| tc.status != "success").count(),
            })))
        }
        Ok(None) => Err(not_found(format!("turn {id} not found"))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_turn_tools(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::tools::get_tool_calls_for_turn(&state.db, &id) {
        Ok(calls) => {
            let items: Vec<Value> = calls
                .into_iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "tool_name": tc.tool_name,
                        "skill_id": tc.skill_id,
                        "skill_name": tc.skill_name,
                        "skill_hash": tc.skill_hash,
                        "status": tc.status,
                        "duration_ms": tc.duration_ms,
                        "created_at": tc.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "tool_calls": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

// ── Analyzer endpoints ──────────────────────────────────────────

fn build_turn_data(
    turn: &ironclad_db::sessions::TurnRecord,
    tool_calls: &[ironclad_db::tools::ToolCallRecord],
) -> TurnData {
    let thinking_text = turn.thinking.as_deref().unwrap_or("");
    let model = turn.model.clone().unwrap_or_default();
    let has_reasoning = model.contains("claude")
        || model.contains("o1")
        || model.contains("o3")
        || model.contains("deepseek");
    TurnData {
        turn_id: turn.id.clone(),
        token_budget: 0,
        system_prompt_tokens: 0,
        memory_tokens: 0,
        history_tokens: 0,
        history_depth: 0,
        complexity_level: "L1".into(),
        model,
        cost: turn.cost.unwrap_or(0.0),
        tokens_in: turn.tokens_in.unwrap_or(0),
        tokens_out: turn.tokens_out.unwrap_or(0),
        tool_call_count: tool_calls.len() as i64,
        tool_failure_count: tool_calls
            .iter()
            .filter(|tc| tc.status != "success")
            .count() as i64,
        thinking_length: thinking_text.len() as i64,
        has_reasoning,
        cached: false,
    }
}

pub async fn get_turn_tips(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let turn_record = match ironclad_db::sessions::get_turn_by_id(&state.db, &id) {
        Ok(Some(t)) => t,
        Ok(None) => {
            return Err(not_found(format!("turn {id} not found")));
        }
        Err(e) => return Err(internal_err(&e)),
    };

    let tool_calls = ironclad_db::tools::get_tool_calls_for_turn(&state.db, &id)
        .map_err(|e| internal_err(&e))?;

    let turn_data = build_turn_data(&turn_record, &tool_calls);

    let session_avg_cost =
        ironclad_db::sessions::list_turns_for_session(&state.db, &turn_record.session_id)
            .ok()
            .and_then(|turns| {
                if turns.is_empty() {
                    return None;
                }
                let total: f64 = turns.iter().map(|t| t.cost.unwrap_or(0.0)).sum();
                Some(total / turns.len() as f64)
            });

    let analyzer = ContextAnalyzer::new();
    let tips = analyzer.analyze_turn(&turn_data, session_avg_cost);

    Ok(axum::Json(serde_json::json!({
        "turn_id": id,
        "tips": tips,
        "tip_count": tips.len(),
    })))
}

pub async fn get_session_insights(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let turns = match ironclad_db::sessions::list_turns_for_session(&state.db, &id) {
        Ok(t) => t,
        Err(e) => return Err(internal_err(&e)),
    };

    let all_tool_calls = ironclad_db::tools::get_tool_calls_for_session(&state.db, &id)
        .map_err(|e| internal_err(&e))?;

    let turn_data: Vec<TurnData> = turns
        .iter()
        .map(|t| {
            let empty = Vec::new();
            let tool_calls = all_tool_calls.get(&t.id).unwrap_or(&empty);
            build_turn_data(t, tool_calls)
        })
        .collect();

    let grades: Vec<(String, i32)> = ironclad_db::sessions::list_session_feedback(&state.db, &id)
        .inspect_err(
            |e| tracing::warn!(error = %e, session_id = %id, "failed to list session feedback"),
        )
        .unwrap_or_default()
        .into_iter()
        .map(|fb| (fb.turn_id, fb.grade))
        .collect();

    let session_data = SessionData {
        turns: turn_data,
        session_id: id.clone(),
        grades,
    };

    let analyzer = ContextAnalyzer::new();
    let insights = analyzer.analyze_session(&session_data);

    Ok(axum::Json(serde_json::json!({
        "session_id": id,
        "insights": insights,
        "insight_count": insights.len(),
        "turn_count": turns.len(),
    })))
}

pub async fn analyze_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let turn_record = match ironclad_db::sessions::get_turn_by_id(&state.db, &id) {
        Ok(Some(t)) => t,
        Ok(None) => {
            return Err(not_found(format!("turn {id} not found")));
        }
        Err(e) => return Err(internal_err(&e)),
    };

    let tool_calls = ironclad_db::tools::get_tool_calls_for_turn(&state.db, &id)
        .inspect_err(
            |e| tracing::warn!(error = %e, turn_id = %id, "failed to get tool calls for turn"),
        )
        .unwrap_or_default();
    let turn_data = build_turn_data(&turn_record, &tool_calls);

    let analyzer = ContextAnalyzer::new();
    let tips = analyzer.analyze_turn(&turn_data, None);
    let critical_count = tips
        .iter()
        .filter(|t| matches!(t.severity, ironclad_agent::analyzer::Severity::Critical))
        .count();
    let warning_count = tips
        .iter()
        .filter(|t| matches!(t.severity, ironclad_agent::analyzer::Severity::Warning))
        .count();
    let summary = if critical_count > 0 {
        "High-risk context issues detected. Address critical guidance first."
    } else if warning_count > 0 {
        "Turn has optimization opportunities based on context heuristics."
    } else {
        "Turn context looks healthy; no major optimization flags."
    };
    let prompt = format!(
        "Analyze this agent turn and provide concrete, actionable guidance.\n\
         Return concise markdown with:\n\
         1) Key issues\n\
         2) Likely root causes\n\
         3) Top 3 remediation steps\n\
         4) Risk level (low/medium/high)\n\n\
         Turn summary: {summary}\n\
         Critical findings: {critical_count}\n\
         Warning findings: {warning_count}\n\
         Heuristic tips:\n{}",
        serde_json::to_string_pretty(&tips).unwrap_or_else(|_| "[]".to_string())
    );

    let llm = run_llm_analysis(&state, &prompt, Some(1200), Some(0.2)).await?;

    Ok(axum::Json(serde_json::json!({
        "turn_id": id,
        "status": "complete",
        "heuristic_tips": tips,
        "analysis": llm["content"],
        "analysis_model": llm["model"],
        "tokens_in": llm["tokens_in"],
        "tokens_out": llm["tokens_out"],
        "cost": llm["cost"],
    })))
}

pub async fn analyze_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let turns = match ironclad_db::sessions::list_turns_for_session(&state.db, &id) {
        Ok(t) => t,
        Err(e) => return Err(internal_err(&e)),
    };

    let all_tool_calls = ironclad_db::tools::get_tool_calls_for_session(&state.db, &id)
        .map_err(|e| internal_err(&e))?;

    let turn_data: Vec<TurnData> = turns
        .iter()
        .map(|t| {
            let empty = Vec::new();
            let tool_calls = all_tool_calls.get(&t.id).unwrap_or(&empty);
            build_turn_data(t, tool_calls)
        })
        .collect();

    let grades: Vec<(String, i32)> = ironclad_db::sessions::list_session_feedback(&state.db, &id)
        .inspect_err(
            |e| tracing::warn!(error = %e, session_id = %id, "failed to list session feedback"),
        )
        .unwrap_or_default()
        .into_iter()
        .map(|fb| (fb.turn_id, fb.grade))
        .collect();

    let session_data = SessionData {
        turns: turn_data,
        session_id: id.clone(),
        grades,
    };

    let analyzer = ContextAnalyzer::new();
    let insights = analyzer.analyze_session(&session_data);
    let critical_count = insights
        .iter()
        .filter(|t| matches!(t.severity, ironclad_agent::analyzer::Severity::Critical))
        .count();
    let warning_count = insights
        .iter()
        .filter(|t| matches!(t.severity, ironclad_agent::analyzer::Severity::Warning))
        .count();
    let top_actions: Vec<String> = insights
        .iter()
        .take(3)
        .map(|t| t.suggestion.clone())
        .collect();
    let prompt = format!(
        "Analyze this session and provide strategic optimization guidance.\n\
         Return concise markdown with:\n\
         1) Session-level bottlenecks\n\
         2) Pattern diagnosis\n\
         3) Prioritized remediation plan\n\
         4) Expected impact\n\n\
         Session ID: {id}\n\
         Turn count: {}\n\
         Critical findings: {critical_count}\n\
         Warning findings: {warning_count}\n\
         Top actions: {}\n\
         Heuristic insights:\n{}",
        turns.len(),
        top_actions.join("; "),
        serde_json::to_string_pretty(&insights).unwrap_or_else(|_| "[]".to_string())
    );

    let llm = run_llm_analysis(&state, &prompt, Some(1800), Some(0.2)).await?;

    Ok(axum::Json(serde_json::json!({
        "session_id": id,
        "status": "complete",
        "heuristic_insights": insights,
        "analysis": llm["content"],
        "analysis_model": llm["model"],
        "tokens_in": llm["tokens_in"],
        "tokens_out": llm["tokens_out"],
        "cost": llm["cost"],
    })))
}

async fn run_llm_analysis(
    state: &AppState,
    prompt: &str,
    max_tokens: Option<u32>,
    temperature: Option<f64>,
) -> Result<serde_json::Value, JsonError> {
    let model = {
        let llm = state.llm.read().await;
        llm.router.select_model().to_string()
    };
    let model_for_api = model
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model)
        .to_string();
    let req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages: vec![ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: prompt.to_string(),
            parts: None,
        }],
        max_tokens,
        temperature,
        system: None,
        quality_target: None,
    };

    let llm = state.llm.read().await;
    let provider = match llm.providers.get_by_model(&model) {
        Some(p) => p.clone(),
        None => {
            return Err(JsonError(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                format!("no provider configured for model {model}"),
            ));
        }
    };
    drop(llm);

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
    if !provider.is_local && key.is_empty() {
        return Err(JsonError(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            format!("missing API key for provider {}", provider.name),
        ));
    }

    let body = ironclad_llm::format::translate_request(&req, provider.format)
        .unwrap_or_else(|_| serde_json::json!({}));
    let llm = state.llm.read().await;
    let resp = llm
        .client
        .forward_with_provider(
            &url,
            &key,
            body,
            &provider.auth_header,
            &provider.extra_headers,
        )
        .await
        .map_err(|e| {
            JsonError(
                axum::http::StatusCode::BAD_GATEWAY,
                format!("analysis provider call failed: {e}"),
            )
        })?;
    drop(llm);

    let unified =
        ironclad_llm::format::translate_response(&resp, provider.format).unwrap_or_else(|_| {
            ironclad_llm::format::UnifiedResponse {
                content: "(no response)".into(),
                model: model.clone(),
                tokens_in: 0,
                tokens_out: 0,
                finish_reason: None,
            }
        });

    let tin = unified.tokens_in as i64;
    let tout = unified.tokens_out as i64;
    let cost = (tin.max(0) as f64 * provider.cost_per_input_token)
        + (tout.max(0) as f64 * provider.cost_per_output_token);
    ironclad_db::metrics::record_inference_cost(
        &state.db,
        &model,
        &provider.name,
        tin,
        tout,
        cost,
        Some("analysis"),
        false,
    )
    .ok();

    Ok(serde_json::json!({
        "content": unified.content,
        "model": model,
        "provider": provider.name,
        "tokens_in": tin,
        "tokens_out": tout,
        "cost": cost,
    }))
}

// ── Turn feedback endpoints ─────────────────────────────────────

pub async fn post_turn_feedback(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
    axum::Json(body): axum::Json<FeedbackRequest>,
) -> impl IntoResponse {
    if !(1..=5).contains(&body.grade) {
        return Err(bad_request("grade must be between 1 and 5"));
    }

    let turn = match ironclad_db::sessions::get_turn_by_id(&state.db, &turn_id) {
        Ok(Some(t)) => t,
        Ok(None) => {
            return Err(not_found(format!("turn {turn_id} not found")));
        }
        Err(e) => return Err(internal_err(&e)),
    };

    match ironclad_db::sessions::record_feedback(
        &state.db,
        &turn_id,
        &turn.session_id,
        body.grade,
        "dashboard",
        body.comment.as_deref(),
    ) {
        Ok(id) => Ok(axum::Json(serde_json::json!({
            "id": id,
            "turn_id": turn_id,
            "grade": body.grade,
        }))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_turn_feedback(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::sessions::get_feedback(&state.db, &turn_id) {
        Ok(Some(fb)) => Ok(axum::Json(serde_json::json!({
            "id": fb.id,
            "turn_id": fb.turn_id,
            "session_id": fb.session_id,
            "grade": fb.grade,
            "source": fb.source,
            "comment": fb.comment,
            "created_at": fb.created_at,
        }))),
        Ok(None) => Err(not_found(format!("no feedback for turn {turn_id}"))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn put_turn_feedback(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
    axum::Json(body): axum::Json<FeedbackRequest>,
) -> impl IntoResponse {
    if !(1..=5).contains(&body.grade) {
        return Err(bad_request("grade must be between 1 and 5"));
    }

    match ironclad_db::sessions::update_feedback(
        &state.db,
        &turn_id,
        body.grade,
        body.comment.as_deref(),
    ) {
        Ok(()) => Ok(axum::Json(serde_json::json!({
            "turn_id": turn_id,
            "grade": body.grade,
            "updated": true,
        }))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_session_feedback(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::sessions::list_session_feedback(&state.db, &session_id) {
        Ok(fbs) => {
            let items: Vec<Value> = fbs
                .into_iter()
                .map(|fb| {
                    serde_json::json!({
                        "id": fb.id,
                        "turn_id": fb.turn_id,
                        "session_id": fb.session_id,
                        "grade": fb.grade,
                        "source": fb.source,
                        "comment": fb.comment,
                        "created_at": fb.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "feedback": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}
