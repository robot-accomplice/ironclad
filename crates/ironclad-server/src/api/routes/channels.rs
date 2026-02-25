//! Webhooks (Telegram, WhatsApp) and channel status.

use subtle::ConstantTimeEq;

use axum::{
    Json,
    body::to_bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde_json::{Value, json};

use super::AppState;
use super::agent::{
    CHANNEL_PROCESSING_ERROR_REPLY, channel_chat_id_for_inbound, process_channel_message,
};

pub async fn webhook_telegram(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> impl IntoResponse {
    let adapter = match state.telegram.as_ref() {
        Some(a) => a,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok": false, "error": "Telegram not configured"})),
            )
                .into_response();
        }
    };
    if adapter.webhook_secret.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ok": false, "error": "Webhook secret not configured"})),
        )
            .into_response();
    }
    if let Some(ref secret) = adapter.webhook_secret {
        let header_value = headers
            .get("X-Telegram-Bot-Api-Secret-Token")
            .and_then(|v| v.to_str().ok());
        let matches = header_value
            .map(|v| bool::from(v.as_bytes().ct_eq(secret.as_bytes())))
            .unwrap_or(false);
        if !matches {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok": false, "error": "missing or invalid webhook secret"})),
            )
                .into_response();
        }
    }
    tracing::debug!("received Telegram webhook");
    {
        match adapter.process_webhook_update(&body) {
            Ok(Some(inbound)) => {
                let state = state.clone();
                state.channel_router.record_received("telegram").await;
                let inbound_for_error = inbound.clone();
                tokio::spawn(async move {
                    if let Err(e) = process_channel_message(&state, inbound).await {
                        state
                            .channel_router
                            .record_processing_error("telegram", e.clone())
                            .await;
                        let chat_id = channel_chat_id_for_inbound(&inbound_for_error);
                        if let Err(send_err) = state
                            .channel_router
                            .send_reply(
                                "telegram",
                                &chat_id,
                                CHANNEL_PROCESSING_ERROR_REPLY.to_string(),
                            )
                            .await
                        {
                            tracing::warn!(
                                error = %send_err,
                                "failed to send Telegram webhook processing failure reply"
                            );
                        }
                        tracing::error!(error = %e, "Telegram message processing failed");
                    }
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse Telegram webhook update");
            }
        }
    }
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

pub async fn webhook_whatsapp_verify(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let mode = params.get("hub.mode").map(String::as_str).unwrap_or("");
    let token = params
        .get("hub.verify_token")
        .map(String::as_str)
        .unwrap_or("");
    let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

    match state.whatsapp.as_ref() {
        Some(adapter) => match adapter.verify_webhook_challenge(mode, token, &challenge) {
            Ok(verified) => (StatusCode::OK, verified).into_response(),
            Err(_) => StatusCode::FORBIDDEN.into_response(),
        },
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub async fn webhook_whatsapp(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> impl IntoResponse {
    let adapter = match state.whatsapp.as_ref() {
        Some(a) => a,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok": false, "error": "WhatsApp not configured"})),
            )
                .into_response();
        }
    };
    let secret = match adapter.app_secret.as_ref() {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"ok": false, "error": "Webhook secret not configured"})),
            )
                .into_response();
        }
    };
    const WEBHOOK_BODY_LIMIT: usize = 1024 * 1024;
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, WEBHOOK_BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok": false, "error": "body too large or invalid"})),
            )
                .into_response();
        }
    };
    let sig_header = parts
        .headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let expected = match &sig_header {
        Some(s) if s.starts_with("sha256=") => &s[7..],
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok": false, "error": "missing or invalid X-Hub-Signature-256"})),
            )
                .into_response();
        }
    };
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key size");
    mac.update(&bytes);
    let computed = hex::encode(mac.finalize().into_bytes());
    if !bool::from(computed.as_bytes().ct_eq(expected.as_bytes())) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error": "invalid webhook signature"})),
        )
            .into_response();
    }

    let body_json: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok": false, "error": "invalid JSON"})),
            )
                .into_response();
        }
    };

    tracing::debug!("received WhatsApp webhook");
    match adapter.process_webhook(&body_json) {
        Ok(Some(inbound)) => {
            let state = state.clone();
            state.channel_router.record_received("whatsapp").await;
            tokio::spawn(async move {
                if let Err(e) = process_channel_message(&state, inbound).await {
                    state
                        .channel_router
                        .record_processing_error("whatsapp", e.clone())
                        .await;
                    tracing::error!(error = %e, "WhatsApp message processing failed");
                }
            });
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse WhatsApp webhook");
        }
    }
    Json(json!({"ok": true})).into_response()
}

pub async fn get_channels_status(State(state): State<AppState>) -> impl IntoResponse {
    let statuses = state.channel_router.channel_status().await;
    let mut result: Vec<Value> = vec![json!({
        "name": "web",
        "connected": true,
        "messages_received": 0,
        "messages_sent": 0,
    })];
    for s in statuses {
        result.push(json!({
            "name": s.name,
            "connected": s.connected,
            "messages_received": s.messages_received,
            "messages_sent": s.messages_sent,
            "last_error": s.last_error,
            "last_activity": s.last_activity,
        }));
    }
    Json(json!(result))
}
