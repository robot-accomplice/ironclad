use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;

use super::{AppState, internal_err};

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub agent_id: String,
}

#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub role: String,
    pub content: String,
}

pub async fn list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, agent_id, scope_key, status, model, created_at, updated_at, metadata \
             FROM sessions ORDER BY created_at DESC",
        )
        .map_err(|e| internal_err(&e))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "agent_id": row.get::<_, String>(1)?,
                "scope_key": row.get::<_, Option<String>>(2)?,
                "status": row.get::<_, String>(3)?,
                "model": row.get::<_, Option<String>>(4)?,
                "created_at": row.get::<_, String>(5)?,
                "updated_at": row.get::<_, String>(6)?,
                "metadata": row.get::<_, Option<String>>(7)?,
            }))
        })
        .map_err(|e| internal_err(&e))?;

    let sessions: Vec<Value> = rows.filter_map(|r| r.ok()).collect();

    Ok::<_, (axum::http::StatusCode, String)>(axum::Json(
        serde_json::json!({ "sessions": sessions }),
    ))
}

pub async fn create_session(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateSessionRequest>,
) -> impl IntoResponse {
    match ironclad_db::sessions::find_or_create(&state.db, &body.agent_id, None) {
        Ok(id) => Ok(axum::Json(serde_json::json!({ "session_id": id }))),
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
            "created_at": s.created_at,
            "updated_at": s.updated_at,
            "metadata": s.metadata,
        }))),
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("session {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::sessions::list_messages(&state.db, &id, None) {
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

pub async fn post_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<PostMessageRequest>,
) -> impl IntoResponse {
    match ironclad_db::sessions::append_message(&state.db, &id, &body.role, &body.content) {
        Ok(msg_id) => Ok(axum::Json(serde_json::json!({ "message_id": msg_id }))),
        Err(e) => Err(internal_err(&e)),
    }
}
