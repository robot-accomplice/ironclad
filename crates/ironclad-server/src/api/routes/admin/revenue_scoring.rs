#[derive(Deserialize)]
pub struct OracleFeedIntakeRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    pub expected_revenue_usdc: f64,
    #[serde(default)]
    pub payload: Value,
}

fn score_input_from_request<'a>(
    source: &'a str,
    strategy: &'a str,
    payload_json: &'a str,
    expected_revenue_usdc: f64,
    request_id: Option<&'a str>,
) -> ironclad_db::revenue_scoring::RevenueOpportunityScoreInput<'a> {
    ironclad_db::revenue_scoring::RevenueOpportunityScoreInput {
        source,
        strategy,
        payload_json,
        expected_revenue_usdc,
        request_id,
    }
}

pub(super) fn score_response_json(
    score: &ironclad_db::revenue_scoring::RevenueOpportunityScore,
) -> Value {
    json!({
        "confidence_score": score.confidence_score,
        "effort_score": score.effort_score,
        "risk_score": score.risk_score,
        "priority_score": score.priority_score,
        "recommended_approved": score.recommended_approved,
        "score_reason": score.score_reason,
    })
}

pub(super) fn score_revenue_payload(
    db: &ironclad_db::Database,
    id: &str,
    source: &str,
    strategy: &str,
    payload_json: &str,
    expected_revenue_usdc: f64,
    request_id: Option<&str>,
) -> Result<ironclad_db::revenue_scoring::RevenueOpportunityScore, JsonError> {
    let score = ironclad_db::revenue_scoring::score_revenue_opportunity(&score_input_from_request(
        source,
        strategy,
        payload_json,
        expected_revenue_usdc,
        request_id,
    ));
    ironclad_db::revenue_scoring::persist_revenue_opportunity_score(db, id, &score)
        .map_err(|e| internal_err(&e))?;
    Ok(score)
}

pub async fn intake_oracle_feed_opportunity(
    State(state): State<AppState>,
    Json(req): Json<OracleFeedIntakeRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let adapted = RevenueOpportunityIntakeRequest {
        source: "trusted_feed_registry".to_string(),
        strategy: "oracle_feed".to_string(),
        request_id: req.request_id,
        expected_revenue_usdc: req.expected_revenue_usdc,
        payload: req.payload,
    };
    intake_revenue_opportunity(State(state), Json(adapted)).await
}

pub async fn score_revenue_opportunity(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let row = ironclad_db::service_revenue::get_revenue_opportunity(&state.db, &id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| not_found(format!("revenue opportunity '{}' not found", id)))?;
    let score = score_revenue_payload(
        &state.db,
        &id,
        &row.source,
        &row.strategy,
        &row.payload_json,
        row.expected_revenue_usdc,
        row.request_id.as_deref(),
    )?;
    Ok(axum::Json(json!({
        "opportunity_id": id,
        "status": row.status,
        "score": score_response_json(&score),
    })))
}
