use axum::{extract::State, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use serde_json::json;

use super::AppState;

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

        session.history.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: body.content,
        });
        session.history.clone()
    }; // write lock dropped — LLM call below won't serialize other interview traffic

    let config = state.config.read().await;
    let model = config.models.primary.clone();
    drop(config);

    let model_for_api = model.split('/').nth(1).unwrap_or(&model).to_string();
    let req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages: history,
        max_tokens: Some(4096),
        temperature: None,
        system: None,
    };

    let llm_read = state.llm.read().await;
    let provider = llm_read.providers.get_by_model(&model);
    let (url, key, auth_header, extra_headers, format) = match provider {
        Some(p) => {
            let url = format!("{}{}", p.url, p.chat_path);
            let key = if p.auth_mode == "oauth" {
                state.oauth.resolve_token(&p.name).await.unwrap_or_default()
            } else if let Some(ref key_ref) = p.api_key_ref {
                if let Some(name) = key_ref.strip_prefix("keystore:") {
                    state.keystore.get(name).unwrap_or_default()
                } else {
                    std::env::var(&p.api_key_env).unwrap_or_default()
                }
            } else {
                std::env::var(&p.api_key_env).unwrap_or_default()
            };
            (
                url,
                key,
                p.auth_header.clone(),
                p.extra_headers.clone(),
                p.format,
            )
        }
        None => {
            let prefix = model.split('/').next().unwrap_or("unknown");
            let key =
                std::env::var(format!("{}_API_KEY", prefix.to_uppercase())).unwrap_or_default();
            (
                String::new(),
                key,
                "Authorization".to_string(),
                std::collections::HashMap::new(),
                ironclad_core::ApiFormat::OpenAiCompletions,
            )
        }
    };
    drop(llm_read);

    if url.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"error": "no provider configured for interview model"})),
        ));
    }

    let llm_body = ironclad_llm::format::translate_request(&req, format)
        .unwrap_or_else(|_| serde_json::json!({}));

    let llm_read = state.llm.read().await;
    let response = llm_read
        .client
        .forward_with_provider(&url, &key, llm_body, &auth_header, &extra_headers)
        .await;
    drop(llm_read);

    match response {
        Ok(resp) => {
            let unified =
                ironclad_llm::format::translate_response(&resp, format).unwrap_or_else(|_| {
                    ironclad_llm::format::UnifiedResponse {
                        content: "(no response)".into(),
                        model: model.clone(),
                        tokens_in: 0,
                        tokens_out: 0,
                        finish_reason: None,
                    }
                });

            let mut interviews = state.interviews.write().await;
            if let Some(session) = interviews.get_mut(&body.session_key) {
                session.history.push(ironclad_llm::format::UnifiedMessage {
                    role: "assistant".into(),
                    content: unified.content.clone(),
                });
            }
            let turn_count = interviews
                .get(&body.session_key)
                .map(|s| s.history.len())
                .unwrap_or(0);

            Ok(axum::Json(json!({
                "session_key": body.session_key,
                "content": unified.content,
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

    let pending = session.pending_output.as_ref().expect("just assigned");
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
