use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;

use super::{AppState, internal_err};

#[derive(Deserialize)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

pub async fn get_working_memory(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::memory::retrieve_working(&state.db, &session_id) {
        Ok(entries) => {
            let items: Vec<Value> = entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "session_id": e.session_id,
                        "entry_type": e.entry_type,
                        "content": e.content,
                        "importance": e.importance,
                        "created_at": e.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "entries": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_episodic_memory(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50);
    match ironclad_db::memory::retrieve_episodic(&state.db, limit) {
        Ok(entries) => {
            let items: Vec<Value> = entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "classification": e.classification,
                        "content": e.content,
                        "importance": e.importance,
                        "created_at": e.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "entries": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_semantic_memory(
    State(state): State<AppState>,
    Path(category): Path<String>,
) -> impl IntoResponse {
    match ironclad_db::memory::retrieve_semantic(&state.db, &category) {
        Ok(entries) => {
            let items: Vec<Value> = entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "category": e.category,
                        "key": e.key,
                        "value": e.value,
                        "confidence": e.confidence,
                        "created_at": e.created_at,
                        "updated_at": e.updated_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "entries": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn memory_search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let query = params.q.unwrap_or_default();
    if query.is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "missing ?q= parameter".to_string(),
        ));
    }
    match ironclad_db::memory::fts_search(&state.db, &query, 100) {
        Ok(results) => Ok(axum::Json(serde_json::json!({ "results": results }))),
        Err(e) => Err(internal_err(&e)),
    }
}
