//! Dashboard tests: SPA rendering and data-source API validation.
//!
//! Validates that the single-page dashboard:
//! - Returns well-formed HTML with expected structure
//! - All API endpoints the dashboard fetches on load return valid JSON
//! - Content markers (title, navigation, themes) are present

use ironclad_harness::sandbox::{SandboxMode, SandboxedServer};

async fn spawn() -> SandboxedServer {
    SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .expect("sandbox spawn failed")
}

// ── SPA structure ─────────────────────────────────────────────

#[tokio::test]
async fn dashboard_serves_html_with_doctype() {
    let s = spawn().await;
    let resp = s.client().get("/").await.unwrap();
    assert!(resp.status().is_success());
    let body = resp.text().await.unwrap();
    assert!(
        body.starts_with("<!DOCTYPE html>"),
        "dashboard should start with DOCTYPE"
    );
}

#[tokio::test]
async fn dashboard_has_title() {
    let s = spawn().await;
    let body = s.client().get("/").await.unwrap().text().await.unwrap();
    assert!(
        body.contains("<title>Ironclad Dashboard</title>"),
        "should contain page title"
    );
}

#[tokio::test]
async fn dashboard_has_sidebar_navigation() {
    let s = spawn().await;
    let body = s.client().get("/").await.unwrap().text().await.unwrap();
    // The dashboard has a sidebar with navigation links
    assert!(body.contains("sidebar-nav"), "should have sidebar nav");
    // Key sections should exist in the HTML
    for section in &["Overview", "Sessions", "Memory", "Skills"] {
        assert!(
            body.contains(section),
            "dashboard should contain '{section}' section"
        );
    }
}

#[tokio::test]
async fn dashboard_has_theme_support() {
    let s = spawn().await;
    let body = s.client().get("/").await.unwrap().text().await.unwrap();
    // Multiple theme definitions exist
    for theme in &["ai-purple", "crt-orange", "crt-green"] {
        assert!(
            body.contains(theme),
            "dashboard should contain theme '{theme}'"
        );
    }
}

#[tokio::test]
async fn dashboard_has_websocket_code() {
    let s = spawn().await;
    let body = s.client().get("/").await.unwrap().text().await.unwrap();
    assert!(
        body.contains("WebSocket") || body.contains("ws://") || body.contains("wss://"),
        "dashboard should have WebSocket connectivity code"
    );
}

// ── Dashboard data-source APIs ────────────────────────────────
//
// These are the endpoints the dashboard fetches on initial load.
// Each must return valid JSON (even if empty/zeroed on a fresh server).

#[tokio::test]
async fn dashboard_api_health() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/health").await.unwrap();
    assert!(v["status"].is_string(), "health should have status field");
}

#[tokio::test]
async fn dashboard_api_agent_status() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/agent/status").await.unwrap();
    // Agent status should have at least an id or name
    assert!(v.is_object(), "agent status should return JSON object: {v}");
}

#[tokio::test]
async fn dashboard_api_sessions() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/sessions").await.unwrap();
    assert!(
        v["sessions"].is_array(),
        "sessions should have sessions array"
    );
}

#[tokio::test]
async fn dashboard_api_skills() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/skills").await.unwrap();
    assert!(v["skills"].is_array(), "skills should have skills array");
}

#[tokio::test]
async fn dashboard_api_cron_jobs() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/cron/jobs").await.unwrap();
    assert!(v["jobs"].is_array(), "cron should have jobs array");
}

#[tokio::test]
async fn dashboard_api_cache_stats() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/stats/cache").await.unwrap();
    assert!(v.is_object(), "cache stats should return JSON object");
}

#[tokio::test]
async fn dashboard_api_wallet_balance() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/wallet/balance").await.unwrap();
    assert!(v.is_object(), "wallet balance should return JSON object");
}

#[tokio::test]
async fn dashboard_api_breaker_status() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/breaker/status").await.unwrap();
    assert!(v.is_object(), "breaker status should return JSON object");
}

#[tokio::test]
async fn dashboard_api_cost_stats() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/stats/costs").await.unwrap();
    assert!(v.is_object(), "cost stats should return JSON object");
}

#[tokio::test]
async fn dashboard_api_workspace_state() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/workspace/state").await.unwrap();
    assert!(v.is_object(), "workspace state should return JSON object");
}

#[tokio::test]
async fn dashboard_api_config() {
    let s = spawn().await;
    let v = s.client().get_ok("/api/config").await.unwrap();
    assert!(v.is_object(), "config should return JSON object");
}
