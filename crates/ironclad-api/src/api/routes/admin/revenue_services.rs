struct RevenueServiceSpec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    price_usdc: f64,
}

const GEO_SITREP_VERIFIED: RevenueServiceSpec = RevenueServiceSpec {
    id: "geopolitical-sitrep-verified",
    name: "Geopolitical Sitrep (Verified)",
    description: "Collects current web sources, applies veracity checks, and returns a concise sitrep.",
    price_usdc: 0.25,
};

fn revenue_catalog() -> &'static [RevenueServiceSpec] {
    &[GEO_SITREP_VERIFIED]
}

fn find_revenue_service(id: &str) -> Option<&'static RevenueServiceSpec> {
    revenue_catalog().iter().find(|svc| svc.id == id)
}

#[derive(Deserialize)]
pub struct ServiceQuoteRequest {
    pub service_id: String,
    pub requester: String,
    #[serde(default)]
    pub parameters: Value,
}

#[derive(Deserialize)]
pub struct ServicePaymentVerifyRequest {
    pub tx_hash: String,
    pub amount_usdc: f64,
    pub recipient: String,
}

#[derive(Deserialize)]
pub struct ServiceFulfillRequest {
    pub fulfillment_output: String,
}

#[derive(Deserialize)]
pub struct RevenueOpportunityIntakeRequest {
    pub source: String,
    pub strategy: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub expected_revenue_usdc: f64,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Deserialize)]
pub struct RevenueOpportunityQualifyRequest {
    #[serde(default)]
    pub approved: Option<bool>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Deserialize)]
pub struct RevenueOpportunityPlanRequest {
    #[serde(default)]
    pub plan: Value,
}

#[derive(Deserialize)]
pub struct RevenueOpportunityFulfillRequest {
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Deserialize)]
pub struct RevenueOpportunitySettleRequest {
    pub settlement_ref: String,
    pub amount_usdc: f64,
    #[serde(default)]
    pub attributable_costs_usdc: Option<f64>,
    #[serde(default = "default_settlement_currency")]
    pub currency: String,
    #[serde(default)]
    pub target_chain: Option<String>,
    #[serde(default)]
    pub auto_swap: Option<bool>,
    #[serde(default)]
    pub target_symbol: Option<String>,
    #[serde(default)]
    pub target_contract_address: Option<String>,
    #[serde(default)]
    pub swap_contract_address: Option<String>,
}

fn default_settlement_currency() -> String {
    "USDC".to_string()
}

#[derive(Deserialize)]
pub struct MicroBountyIntakeRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    pub expected_revenue_usdc: f64,
    #[serde(default)]
    pub payload: Value,
}

pub async fn list_services_catalog() -> impl IntoResponse {
    let items: Vec<Value> = revenue_catalog()
        .iter()
        .map(|svc| {
            json!({
                "id": svc.id,
                "name": svc.name,
                "description": svc.description,
                "price_usdc": svc.price_usdc,
                "currency": "USDC",
            })
        })
        .collect();
    axum::Json(json!({ "services": items }))
}

pub async fn create_service_quote(
    State(state): State<AppState>,
    Json(req): Json<ServiceQuoteRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let svc = find_revenue_service(req.service_id.trim())
        .ok_or_else(|| not_found(format!("unknown service '{}'", req.service_id)))?;
    let requester = req.requester.trim();
    validate_field("requester", requester, 64)?;

    let request_id = format!("sr_{}", uuid::Uuid::new_v4().simple());
    let recipient = state.wallet.wallet.address().to_string();
    let quote_expires_at = (Utc::now() + Duration::hours(24)).to_rfc3339();
    let parameters_json = serde_json::to_string(&req.parameters)
        .map_err(|e| bad_request(format!("invalid parameters: {e}")))?;
    if parameters_json.len() > 65_536 {
        return Err(bad_request(
            "parameters payload exceeds max size of 64KB",
        ));
    }

    let new_req = ironclad_db::service_revenue::NewServiceRequest {
        id: &request_id,
        service_id: svc.id,
        requester,
        parameters_json: &parameters_json,
        quoted_amount: svc.price_usdc,
        currency: "USDC",
        recipient: &recipient,
        quote_expires_at: &quote_expires_at,
    };
    ironclad_db::service_revenue::create_service_request(&state.db, &new_req)
        .map_err(|e| internal_err(&e))?;

    Ok(axum::Json(json!({
        "request_id": request_id,
        "service_id": svc.id,
        "status": ironclad_db::service_revenue::STATUS_QUOTED,
        "amount_usdc": svc.price_usdc,
        "currency": "USDC",
        "recipient": recipient,
        "quote_expires_at": quote_expires_at,
    })))
}

pub async fn get_service_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let req = ironclad_db::service_revenue::get_service_request(&state.db, &id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| not_found(format!("service request '{}' not found", id)))?;
    Ok(axum::Json(json!({
        "id": req.id,
        "service_id": req.service_id,
        "requester": req.requester,
        "parameters": serde_json::from_str::<Value>(&req.parameters_json).unwrap_or_else(|e| {
            tracing::warn!(request_id = %id, error = %e, "parameters_json contains invalid JSON");
            json!({ "raw": req.parameters_json })
        }),
        "status": req.status,
        "quoted_amount": req.quoted_amount,
        "currency": req.currency,
        "recipient": req.recipient,
        "quote_expires_at": req.quote_expires_at,
        "payment_tx_hash": req.payment_tx_hash,
        "paid_amount": req.paid_amount,
        "payment_verified_at": req.payment_verified_at,
        "fulfillment_output": req.fulfillment_output,
        "fulfilled_at": req.fulfilled_at,
        "failure_reason": req.failure_reason,
        "created_at": req.created_at,
        "updated_at": req.updated_at,
    })))
}

pub async fn verify_service_payment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ServicePaymentVerifyRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let tx_hash = req.tx_hash.trim();
    let recipient = req.recipient.trim();
    if tx_hash.is_empty() || tx_hash.len() > 128 {
        return Err(bad_request("tx_hash must be non-empty and <= 128 chars"));
    }
    if !req.amount_usdc.is_finite() || req.amount_usdc <= 0.0 {
        return Err(bad_request("amount_usdc must be a finite positive number"));
    }

    let existing = ironclad_db::service_revenue::get_service_request(&state.db, &id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| not_found(format!("service request '{}' not found", id)))?;

    if !existing
        .status
        .eq_ignore_ascii_case(ironclad_db::service_revenue::STATUS_QUOTED)
    {
        return Err(bad_request(format!(
            "service request '{}' is not in quoted state",
            id
        )));
    }
    let now = chrono::Utc::now().to_rfc3339();
    if existing.quote_expires_at < now {
        return Err(bad_request(format!(
            "service quote '{}' has expired (expired at {})",
            id, existing.quote_expires_at
        )));
    }
    if !existing.recipient.eq_ignore_ascii_case(recipient) {
        return Err(bad_request("recipient does not match quoted recipient"));
    }
    if (existing.quoted_amount - req.amount_usdc).abs() > 0.000001 {
        return Err(bad_request(format!(
            "amount_usdc mismatch: expected {} got {}",
            existing.quoted_amount, req.amount_usdc
        )));
    }

    let updated = ironclad_db::service_revenue::mark_payment_verified(
        &state.db,
        &id,
        tx_hash,
        req.amount_usdc,
    )
    .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request("service request state transition rejected"));
    }

    ironclad_db::metrics::record_transaction(
        &state.db,
        "service_revenue",
        req.amount_usdc,
        "USDC",
        Some(existing.requester.as_str()),
        Some(tx_hash),
    )
    .map_err(|e| internal_err(&e))?;

    Ok(axum::Json(json!({
        "request_id": id,
        "status": ironclad_db::service_revenue::STATUS_PAYMENT_VERIFIED,
        "verified": true,
    })))
}

#[derive(Deserialize)]
pub struct ServiceFailRequest {
    pub reason: String,
}

pub async fn fail_service_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ServiceFailRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let reason = req.reason.trim();
    if reason.is_empty() {
        return Err(bad_request("reason must be non-empty"));
    }
    let updated = ironclad_db::service_revenue::mark_service_request_failed(&state.db, &id, reason)
        .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(not_found(format!(
            "service request '{}' not found or already completed/failed",
            id
        )));
    }
    Ok(axum::Json(json!({
        "request_id": id,
        "status": ironclad_db::service_revenue::STATUS_FAILED,
        "reason": reason,
    })))
}

pub async fn fulfill_service_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ServiceFulfillRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let output = req.fulfillment_output.trim();
    if output.is_empty() {
        return Err(bad_request("fulfillment_output cannot be empty"));
    }
    if output.len() > 8000 {
        return Err(bad_request(
            "fulfillment_output exceeds max length of 8000 characters",
        ));
    }

    let updated = ironclad_db::service_revenue::mark_fulfilled(&state.db, &id, output)
        .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request(
            "service request must be payment_verified before fulfillment",
        ));
    }

    ironclad_db::metrics::record_transaction(
        &state.db,
        "service_delivery",
        0.0,
        "USDC",
        Some("service_engine"),
        None,
    )
    .map_err(|e| internal_err(&e))?;

    Ok(axum::Json(json!({
        "request_id": id,
        "status": ironclad_db::service_revenue::STATUS_COMPLETED,
        "fulfilled": true,
    })))
}
