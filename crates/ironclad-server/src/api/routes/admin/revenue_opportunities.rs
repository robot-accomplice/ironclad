pub async fn intake_revenue_opportunity(
    State(state): State<AppState>,
    Json(req): Json<RevenueOpportunityIntakeRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let source = req.source.trim().to_ascii_lowercase();
    let strategy = req.strategy.trim().to_ascii_lowercase();
    validate_short("source", &source)?;
    validate_short("strategy", &strategy)?;
    if source.is_empty() || strategy.is_empty() {
        return Err(bad_request("source and strategy must be non-empty"));
    }
    if req.expected_revenue_usdc <= 0.0 {
        return Err(bad_request("expected_revenue_usdc must be positive"));
    }

    let opportunity_id = format!("ro_{}", uuid::Uuid::new_v4().simple());
    let payload_json = serde_json::to_string(&req.payload)
        .map_err(|e| bad_request(format!("invalid payload: {e}")))?;
    let new_opp = ironclad_db::service_revenue::NewRevenueOpportunity {
        id: &opportunity_id,
        source: &source,
        strategy: &strategy,
        payload_json: &payload_json,
        expected_revenue_usdc: req.expected_revenue_usdc,
        request_id: req.request_id.as_deref(),
    };
    ironclad_db::service_revenue::create_revenue_opportunity(&state.db, &new_opp)
        .map_err(|e| internal_err(&e))?;

    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": ironclad_db::service_revenue::OPPORTUNITY_STATUS_INTAKE,
        "source": source,
        "strategy": strategy,
        "expected_revenue_usdc": req.expected_revenue_usdc,
    })))
}

pub async fn intake_micro_bounty_opportunity(
    State(state): State<AppState>,
    Json(req): Json<MicroBountyIntakeRequest>,
) -> Result<impl IntoResponse, JsonError> {
    // Shared lifecycle adapter: normalize micro-bounty into canonical intake.
    let adapted = RevenueOpportunityIntakeRequest {
        source: "micro_bounty_board".to_string(),
        strategy: "micro_bounty".to_string(),
        request_id: req.request_id,
        expected_revenue_usdc: req.expected_revenue_usdc,
        payload: req.payload,
    };
    intake_revenue_opportunity(State(state), Json(adapted)).await
}

pub async fn get_revenue_opportunity(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let row = ironclad_db::service_revenue::get_revenue_opportunity(&state.db, &id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| not_found(format!("revenue opportunity '{}' not found", id)))?;
    Ok(axum::Json(json!({
        "id": row.id,
        "source": row.source,
        "strategy": row.strategy,
        "payload": serde_json::from_str::<Value>(&row.payload_json).unwrap_or_else(|_| json!({"raw": row.payload_json})),
        "expected_revenue_usdc": row.expected_revenue_usdc,
        "status": row.status,
        "qualification_reason": row.qualification_reason,
        "plan": row.plan_json.and_then(|v| serde_json::from_str::<Value>(&v).ok()),
        "evidence": row.evidence_json.and_then(|v| serde_json::from_str::<Value>(&v).ok()),
        "request_id": row.request_id,
        "settlement_ref": row.settlement_ref,
        "settled_amount_usdc": row.settled_amount_usdc,
        "attributable_costs_usdc": row.attributable_costs_usdc,
        "net_profit_usdc": row.net_profit_usdc,
        "tax_rate": row.tax_rate,
        "tax_amount_usdc": row.tax_amount_usdc,
        "retained_earnings_usdc": row.retained_earnings_usdc,
        "tax_destination_wallet": row.tax_destination_wallet,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })))
}

pub async fn qualify_revenue_opportunity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RevenueOpportunityQualifyRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let reason = req.reason.trim();
    let updated = ironclad_db::service_revenue::qualify_revenue_opportunity(
        &state.db,
        &id,
        req.approved,
        if reason.is_empty() {
            None
        } else {
            Some(reason)
        },
    )
    .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request(
            "revenue opportunity must be in intake state to qualify/reject",
        ));
    }
    Ok(axum::Json(json!({
        "opportunity_id": id,
        "status": if req.approved {
            ironclad_db::service_revenue::OPPORTUNITY_STATUS_QUALIFIED
        } else {
            ironclad_db::service_revenue::OPPORTUNITY_STATUS_REJECTED
        },
    })))
}

pub async fn plan_revenue_opportunity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RevenueOpportunityPlanRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let plan_json =
        serde_json::to_string(&req.plan).map_err(|e| bad_request(format!("invalid plan: {e}")))?;
    let updated =
        ironclad_db::service_revenue::plan_revenue_opportunity(&state.db, &id, &plan_json)
            .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request(
            "revenue opportunity must be qualified before planning",
        ));
    }
    Ok(axum::Json(json!({
        "opportunity_id": id,
        "status": ironclad_db::service_revenue::OPPORTUNITY_STATUS_PLANNED,
    })))
}

pub async fn fulfill_revenue_opportunity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RevenueOpportunityFulfillRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let evidence_json = serde_json::to_string(&req.evidence)
        .map_err(|e| bad_request(format!("invalid evidence: {e}")))?;
    let updated = ironclad_db::service_revenue::mark_revenue_opportunity_fulfilled(
        &state.db,
        &id,
        &evidence_json,
    )
    .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request(
            "revenue opportunity must be planned before fulfillment",
        ));
    }
    Ok(axum::Json(json!({
        "opportunity_id": id,
        "status": ironclad_db::service_revenue::OPPORTUNITY_STATUS_FULFILLED,
    })))
}
