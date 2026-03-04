use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;

use super::{AppState, bad_request, internal_err};

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

const MAX_MEMORY_LIMIT: i64 = 1000;

pub async fn get_working_memory_all(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(100).clamp(1, MAX_MEMORY_LIMIT);
    match ironclad_db::memory::retrieve_working_all(&state.db, limit) {
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
    let limit = params.limit.unwrap_or(50).clamp(1, MAX_MEMORY_LIMIT);
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

pub async fn get_semantic_categories(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::memory::list_semantic_categories(&state.db) {
        Ok(cats) => {
            let items: Vec<Value> = cats
                .into_iter()
                .map(|(cat, count)| serde_json::json!({ "category": cat, "count": count }))
                .collect();
            Ok(axum::Json(serde_json::json!({ "categories": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_semantic_memory_all(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(100).clamp(1, MAX_MEMORY_LIMIT);
    match ironclad_db::memory::retrieve_semantic_all(&state.db, limit) {
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
        return Err(bad_request("missing ?q= parameter"));
    }
    if query.len() > 512 {
        return Err(bad_request("search query too long (max 512 chars)"));
    }
    match ironclad_db::memory::fts_search(&state.db, &query, 100) {
        Ok(results) => Ok(axum::Json(serde_json::json!({ "results": results }))),
        Err(e) => Err(internal_err(&e)),
    }
}

// ── Knowledge ingestion ────────────────────────────────────────

#[derive(Deserialize)]
pub struct IngestRequest {
    pub path: String,
}

pub async fn knowledge_ingest(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<IngestRequest>,
) -> impl IntoResponse {
    use ironclad_agent::ingest::{ingest_directory, ingest_file};
    use std::path::Path;

    let workspace = {
        let cfg = state.config.read().await;
        cfg.agent.workspace.clone()
    };
    if !workspace.exists()
        && let Err(e) = std::fs::create_dir_all(&workspace)
    {
        return Err(bad_request(format!(
            "workspace root '{}' cannot be created: {e}",
            workspace.display()
        )));
    }
    let workspace_root = match std::fs::canonicalize(&workspace) {
        Ok(p) => p,
        Err(e) => {
            return Err(bad_request(format!(
                "workspace root '{}' is not accessible: {e}",
                workspace.display()
            )));
        }
    };
    let raw = Path::new(&body.path);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace_root.join(raw)
    };
    let target = match std::fs::canonicalize(&candidate) {
        Ok(p) => p,
        Err(_) => return Err(bad_request("path does not exist or is not accessible")),
    };
    if !target.starts_with(&workspace_root) {
        return Err(bad_request(format!(
            "path '{}' escapes workspace root '{}'",
            target.display(),
            workspace_root.display()
        )));
    }

    let results = if target.is_dir() {
        match ingest_directory(&state.db, &target) {
            Ok(r) => r,
            Err(e) => return Err(internal_err(&e)),
        }
    } else if target.is_file() {
        match ingest_file(&state.db, &target) {
            Ok(r) => vec![r],
            Err(e) => return Err(bad_request(format!("{e}"))),
        }
    } else {
        return Err(bad_request("path does not exist or is not accessible"));
    };

    let total_chunks: usize = results.iter().map(|r| r.chunks_stored).sum();
    Ok(axum::Json(serde_json::json!({
        "files_ingested": results.len(),
        "total_chunks": total_chunks,
        "results": results,
    })))
}
