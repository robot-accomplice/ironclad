use std::time::Instant;

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use serde_json::json;

use super::AppState;

const MAX_INTERVIEW_SESSIONS: usize = 1000;
const INTERVIEW_TTL_SECS: u64 = 3600;
/// Hard cap on conversation turns per interview session to prevent
/// unbounded memory growth within the 3600s TTL.
const MAX_TURNS_PER_SESSION: usize = 200;

#[derive(Deserialize)]
pub struct InterviewStartRequest {
    #[serde(default)]
    pub session_key: Option<String>,
}

pub async fn start_interview(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<InterviewStartRequest>,
) -> impl IntoResponse {
    let key = body
        .session_key
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut interviews = state.interviews.write().await;

    if interviews.contains_key(&key) {
        return Err((
            StatusCode::CONFLICT,
            axum::Json(json!({"error": "interview already in progress", "session_key": key})),
        ));
    }

    // Evict expired sessions (older than TTL)
    let ttl = std::time::Duration::from_secs(INTERVIEW_TTL_SECS);
    let now = Instant::now();
    interviews.retain(|_, session| now.duration_since(session.created_at) < ttl);

    // Evict oldest if at capacity
    if interviews.len() >= MAX_INTERVIEW_SESSIONS
        && let Some(oldest_key) = interviews
            .iter()
            .min_by_key(|(_, s)| s.created_at)
            .map(|(k, _)| k.clone())
    {
        interviews.remove(&oldest_key);
    }

    let session = super::InterviewSession::new();
    interviews.insert(key.clone(), session);

    Ok(axum::Json(json!({
        "session_key": key,
        "status": "started",
        "opening": "Initiating personality interview sequence.",
    })))
}

#[derive(Deserialize)]
pub struct InterviewTurnRequest {
    pub session_key: String,
    pub content: String,
}

pub async fn interview_turn(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<InterviewTurnRequest>,
) -> impl IntoResponse {
    // Acquire write lock, push user message, clone history, then release
    let user_content = body.content.clone();
    let history = {
        let mut interviews = state.interviews.write().await;
        let session = match interviews.get_mut(&body.session_key) {
            Some(s) => s,
            None => {
                return Err((
                    StatusCode::NOT_FOUND,
                    axum::Json(
                        json!({"error": "no interview session found", "session_key": body.session_key}),
                    ),
                ));
            }
        };

        if session.history.len() >= MAX_TURNS_PER_SESSION {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                axum::Json(json!({
                    "error": "interview session has reached the maximum number of turns",
                    "session_key": body.session_key,
                    "max_turns": MAX_TURNS_PER_SESSION,
                })),
            ));
        }

        session.history.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: user_content.clone(),
            parts: None,
        });
        session.history.clone()
    }; // write lock dropped — LLM call below won't serialize other interview traffic

    let model = super::agent::select_routed_model(&state, &user_content).await;
    let req = ironclad_llm::format::UnifiedRequest {
        model: model
            .split_once('/')
            .map(|(_, m)| m)
            .unwrap_or(&model)
            .to_string(),
        messages: history,
        max_tokens: Some(4096),
        temperature: None,
        system: None,
        quality_target: None,
        tools: vec![],
    };
    match super::agent::infer_content_with_fallback(&state, &req, &model).await {
        Ok(content) => {
            let mut interviews = state.interviews.write().await;
            if let Some(session) = interviews.get_mut(&body.session_key) {
                session.history.push(ironclad_llm::format::UnifiedMessage {
                    role: "assistant".into(),
                    content: content.clone(),
                    parts: None,
                });
            }
            let turn_count = interviews
                .get(&body.session_key)
                .map(|s| s.history.len())
                .unwrap_or(0);

            Ok(axum::Json(json!({
                "session_key": body.session_key,
                "content": content,
                "turn": turn_count,
            })))
        }
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            axum::Json(json!({"error": format!("LLM call failed: {e}")})),
        )),
    }
}

#[derive(Deserialize)]
pub struct InterviewFinishRequest {
    pub session_key: String,
}

pub async fn finish_interview(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<InterviewFinishRequest>,
) -> impl IntoResponse {
    let mut interviews = state.interviews.write().await;
    let session = match interviews.get_mut(&body.session_key) {
        Some(s) => s,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                axum::Json(json!({"error": "no interview session found"})),
            ));
        }
    };

    let last_assistant = session
        .history
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let parsed = ironclad_core::personality::parse_interview_output(&last_assistant);
    let file_count = parsed.file_count();

    if file_count == 0 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(json!({
                "error": "no TOML personality files found in the last assistant response",
                "hint": "continue the interview until the agent generates OS.toml, FIRMWARE.toml, etc."
            })),
        ));
    }

    if let Err(errors) = parsed.validate() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(json!({
                "error": "generated TOML has validation errors",
                "errors": errors,
            })),
        ));
    }

    session.pending_output = Some(parsed);
    session.awaiting_confirmation = true;

    let Some(pending) = session.pending_output.as_ref() else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({
                "error": "pending output was not retained after generation",
            })),
        ));
    };
    Ok(axum::Json(json!({
        "session_key": body.session_key,
        "status": "awaiting_confirmation",
        "files_generated": file_count,
        "has_os": pending.os_toml.is_some(),
        "has_firmware": pending.firmware_toml.is_some(),
        "has_operator": pending.operator_toml.is_some(),
        "has_directives": pending.directives_toml.is_some(),
    })))
}
