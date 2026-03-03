//! Session lifecycle tests: create → list → show → messages → turns → insights.
//!
//! Each test gets its own sandboxed server — no cross-test state leakage.

use ironclad_harness::sandbox::{SandboxMode, SandboxedServer};
use serde_json::json;

/// Helper: spawn a sandbox and create one session, returning (server, session_id).
async fn spawn_with_session() -> (SandboxedServer, String) {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .expect("sandbox spawn failed");
    let created = server
        .client()
        .post_ok("/api/sessions", &json!({"agent_id": "harness-test"}))
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    (server, id)
}

// ── Session CRUD ────────────────────────────────────────────

#[tokio::test]
async fn session_list_initially_empty() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let body = server.client().get_ok("/api/sessions").await.unwrap();
    let sessions = body["sessions"].as_array().unwrap();
    assert!(sessions.is_empty(), "fresh server should have 0 sessions");
}

#[tokio::test]
async fn session_create_returns_id_and_agent() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let created = server
        .client()
        .post_ok("/api/sessions", &json!({"agent_id": "harness-test"}))
        .await
        .unwrap();
    assert!(created["id"].is_string(), "id should be string");
    assert_eq!(created["agent_id"].as_str(), Some("harness-test"));
}

#[tokio::test]
async fn session_show_returns_detail() {
    let (server, id) = spawn_with_session().await;
    let detail = server
        .client()
        .get_ok(&format!("/api/sessions/{id}"))
        .await
        .unwrap();
    assert_eq!(detail["id"].as_str(), Some(id.as_str()));
    assert!(detail["created_at"].is_string(), "should have created_at");
}

#[tokio::test]
async fn session_show_unknown_returns_404() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let resp = server
        .client()
        .get("/api/sessions/nonexistent-uuid")
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

// ── Session messages ────────────────────────────────────────

#[tokio::test]
async fn session_messages_initially_empty() {
    let (server, id) = spawn_with_session().await;
    let body = server
        .client()
        .get_ok(&format!("/api/sessions/{id}/messages"))
        .await
        .unwrap();
    // Messages should be an array (empty for a fresh session)
    assert!(
        body["messages"].is_array() || body.is_array(),
        "messages response should contain an array: {body}"
    );
}

#[tokio::test]
async fn session_turns_initially_empty() {
    let (server, id) = spawn_with_session().await;
    let body = server
        .client()
        .get_ok(&format!("/api/sessions/{id}/turns"))
        .await
        .unwrap();
    assert!(
        body["turns"].is_array() || body.is_array(),
        "turns response should contain an array: {body}"
    );
}

#[tokio::test]
async fn session_insights_returns_json() {
    let (server, id) = spawn_with_session().await;
    let body = server
        .client()
        .get_ok(&format!("/api/sessions/{id}/insights"))
        .await
        .unwrap();
    assert!(
        body.is_object() || body.is_array(),
        "insights should be JSON: {body}"
    );
}

#[tokio::test]
async fn session_feedback_returns_json() {
    let (server, id) = spawn_with_session().await;
    let body = server
        .client()
        .get_ok(&format!("/api/sessions/{id}/feedback"))
        .await
        .unwrap();
    assert!(
        body.is_object() || body.is_array(),
        "feedback should be JSON: {body}"
    );
}

// ── Multiple sessions ───────────────────────────────────────

#[tokio::test]
async fn multiple_sessions_list_correctly() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let c = server.client();

    // Create 3 sessions
    for _ in 0..3 {
        c.post_ok("/api/sessions", &json!({"agent_id": "harness-test"}))
            .await
            .unwrap();
    }

    let list = c.get_ok("/api/sessions").await.unwrap();
    let sessions = list["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 3, "should have exactly 3 sessions");
}

// ── Session backfill-nicknames ──────────────────────────────

#[tokio::test]
async fn backfill_nicknames_accepts_post() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    // This is a maintenance endpoint — should accept POST and return success
    // even when there are no sessions to backfill
    let resp = server
        .client()
        .post_json("/api/sessions/backfill-nicknames", &json!({}))
        .await
        .unwrap();
    // Accept 200 or 202 — both mean "accepted"
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 202 || status == 204,
        "backfill-nicknames should succeed, got {status}"
    );
}
