pub async fn intake_revenue_opportunity(
    State(state): State<AppState>,
    Json(req): Json<RevenueOpportunityIntakeRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let source = req.source.trim().to_ascii_lowercase();
    let strategy = req.strategy.trim().to_ascii_lowercase();
    validate_short("source", &source)?;
    validate_short("strategy", &strategy)?;
    if !req.expected_revenue_usdc.is_finite() || req.expected_revenue_usdc <= 0.0 {
        return Err(bad_request("expected_revenue_usdc must be positive"));
    }

    let opportunity_id = format!("ro_{}", uuid::Uuid::new_v4().simple());
    let payload_json = serde_json::to_string(&req.payload)
        .map_err(|e| bad_request(format!("invalid payload: {e}")))?;
    if payload_json.len() > 65_536 {
        return Err(bad_request("payload exceeds max size of 64KB"));
    }
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
    let score = score_revenue_payload(
        &state.db,
        &opportunity_id,
        &source,
        &strategy,
        &payload_json,
        req.expected_revenue_usdc,
        req.request_id.as_deref(),
    )?;

    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": ironclad_db::service_revenue::OPPORTUNITY_STATUS_INTAKE,
        "source": source,
        "strategy": strategy,
        "expected_revenue_usdc": req.expected_revenue_usdc,
        "score": score_response_json(&score),
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
    let payload_value = serde_json::from_str::<Value>(&row.payload_json).unwrap_or_else(|e| {
        tracing::warn!(opportunity_id = %id, error = %e, "payload_json contains invalid JSON");
        json!({"raw": row.payload_json})
    });
    let plan_value = row.plan_json.and_then(|v| {
        serde_json::from_str::<Value>(&v).map_err(|e| {
            tracing::warn!(opportunity_id = %id, error = %e, "plan_json contains invalid JSON");
            e
        }).ok()
    });
    let evidence_value = row.evidence_json.and_then(|v| {
        serde_json::from_str::<Value>(&v).map_err(|e| {
            tracing::warn!(opportunity_id = %id, error = %e, "evidence_json contains invalid JSON");
            e
        }).ok()
    });
    Ok(axum::Json(json!({
        "id": row.id,
        "source": row.source,
        "strategy": row.strategy,
        "payload": payload_value,
        "expected_revenue_usdc": row.expected_revenue_usdc,
        "status": row.status,
        "qualification_reason": row.qualification_reason,
        "score": {
            "confidence_score": row.confidence_score,
            "effort_score": row.effort_score,
            "risk_score": row.risk_score,
            "priority_score": row.priority_score,
            "recommended_approved": row.recommended_approved,
            "score_reason": row.score_reason,
        },
        "plan": plan_value,
        "evidence": evidence_value,
        "request_id": row.request_id,
        "settlement_ref": row.settlement_ref,
        "settled_amount_usdc": row.settled_amount_usdc,
        "attributable_costs_usdc": row.attributable_costs_usdc,
        "net_profit_usdc": row.net_profit_usdc,
        "tax_rate": row.tax_rate,
        "tax_amount_usdc": row.tax_amount_usdc,
        "retained_earnings_usdc": row.retained_earnings_usdc,
        "tax_destination_wallet": row.tax_destination_wallet,
        "swap_task": revenue_swap_task_status(&state.db, &id)
            .map_err(|e| internal_err(&e))?,
        "settled_at": row.settled_at,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })))
}

fn revenue_swap_task_status(
    db: &ironclad_db::Database,
    opportunity_id: &str,
) -> Result<Option<Value>, ironclad_core::IroncladError> {
    let conn = db.conn();
    let task_id = format!("rev_swap:{opportunity_id}");
    match conn.query_row(
        "SELECT id, title, status, source, created_at, updated_at \
         FROM tasks WHERE id = ?1",
        [task_id.as_str()],
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "title": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "source": row.get::<_, Option<String>>(3)?,
                "created_at": row.get::<_, String>(4)?,
                "updated_at": row.get::<_, String>(5)?,
            }))
        },
    ) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(ironclad_core::IroncladError::Database(e.to_string())),
    }
}

pub async fn qualify_revenue_opportunity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RevenueOpportunityQualifyRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let row = ironclad_db::service_revenue::get_revenue_opportunity(&state.db, &id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| not_found(format!("revenue opportunity '{}' not found", id)))?;
    let approved = match req.approved {
        Some(v) => v,
        None => {
            // When scoring hasn't run (all scores at defaults), the system has no basis
            // for a recommendation.  Require the caller to make an explicit decision
            // rather than silently falling back to `recommended_approved = false`.
            let scores_are_default = row.confidence_score == 0.0
                && row.effort_score == 0.0
                && row.risk_score == 0.0
                && row.priority_score == 0.0;
            if scores_are_default && !row.recommended_approved {
                return Err(bad_request(
                    "scoring has not run for this opportunity; \
                     'approved' field is required for explicit qualification decision",
                ));
            }
            row.recommended_approved
        }
    };
    let reason = req.reason.trim();
    let updated = ironclad_db::service_revenue::qualify_revenue_opportunity(
        &state.db,
        &id,
        approved,
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
        "status": if approved {
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
    if plan_json.len() > 65_536 {
        return Err(bad_request("plan payload exceeds max size of 64KB"));
    }
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
    if evidence_json.len() > 65_536 {
        return Err(bad_request("evidence payload exceeds max size of 64KB"));
    }
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
