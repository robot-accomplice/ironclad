#[derive(Deserialize)]
pub struct RevenueOpportunityFeedbackRequest {
    pub grade: f64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

pub async fn record_revenue_opportunity_feedback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RevenueOpportunityFeedbackRequest>,
) -> Result<impl IntoResponse, JsonError> {
    if !(0.0..=5.0).contains(&req.grade) {
        return Err(bad_request("grade must be between 0.0 and 5.0"));
    }
    let source = req
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("operator");
    let feedback_id = ironclad_db::revenue_feedback::record_revenue_feedback(
        &state.db,
        &id,
        req.grade,
        source,
        req.comment.as_deref(),
    )
    .map_err(|e| internal_err(&e))?;
    Ok(axum::Json(json!({
        "opportunity_id": id,
        "feedback_id": feedback_id,
        "grade": req.grade,
        "source": source,
        "recorded": true,
    })))
}
