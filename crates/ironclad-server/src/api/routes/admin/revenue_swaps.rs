
#[derive(Deserialize, Default)]
pub struct RevenueSwapListParams {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct RevenueSwapConfirmRequest {
    pub tx_hash: String,
}

#[derive(Deserialize)]
pub struct RevenueSwapFailRequest {
    pub reason: String,
}

pub async fn list_revenue_swap_tasks(
    State(state): State<AppState>,
    Query(query): Query<RevenueSwapListParams>,
) -> Result<impl IntoResponse, JsonError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let rows = ironclad_db::revenue_swap_tasks::list_revenue_swap_tasks(&state.db, limit)
        .map_err(|e| internal_err(&e))?;
    Ok(axum::Json(json!({
        "swap_tasks": rows,
        "count": rows.len(),
    })))
}

pub async fn start_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let updated = ironclad_db::revenue_swap_tasks::mark_revenue_swap_in_progress(
        &state.db,
        &opportunity_id,
    )
    .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request(
            "revenue swap task must exist and be pending before start",
        ));
    }
    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": "in_progress",
    })))
}

pub async fn confirm_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueSwapConfirmRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let tx_hash = req.tx_hash.trim();
    if tx_hash.is_empty() || tx_hash.len() > 128 {
        return Err(bad_request("tx_hash must be non-empty and <= 128 chars"));
    }
    let updated =
        ironclad_db::revenue_swap_tasks::mark_revenue_swap_confirmed(&state.db, &opportunity_id, tx_hash)
            .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(not_found(format!(
            "revenue swap task for opportunity '{}' not found",
            opportunity_id
        )));
    }
    ironclad_db::metrics::record_transaction_with_metadata(
        &state.db,
        "revenue_swap_execution",
        0.0,
        "USDC",
        Some("revenue_swap"),
        Some(tx_hash),
        Some(
            &serde_json::to_string(&json!({
                "opportunity_id": opportunity_id,
                "status": "completed",
            }))
            .map_err(|e| internal_err(&ironclad_core::IroncladError::Database(e.to_string())))?,
        ),
    )
    .map_err(|e| internal_err(&e))?;
    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": "completed",
        "tx_hash": tx_hash,
    })))
}

pub async fn fail_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueSwapFailRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let reason = req.reason.trim();
    if reason.is_empty() {
        return Err(bad_request("reason must be non-empty"));
    }
    let updated = ironclad_db::revenue_swap_tasks::mark_revenue_swap_failed(
        &state.db,
        &opportunity_id,
        reason,
    )
    .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(not_found(format!(
            "revenue swap task for opportunity '{}' not found",
            opportunity_id
        )));
    }
    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": "failed",
        "reason": reason,
    })))
}
