//! Proof-of-concept harness tests: health, sessions, config.
//!
//! Each test spawns its own `SandboxedServer` on a unique port.
//! Running with `--test-threads=8` validates parallel execution.

use ironclad_harness::sandbox::{SandboxMode, SandboxedServer};
use serde_json::json;

// ── Health endpoint ──────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .expect("sandbox spawn failed");

    let resp = server.client().get("/api/health").await.unwrap();
    assert!(
        resp.status().is_success(),
        "health returned {}",
        resp.status()
    );
}

#[tokio::test]
async fn health_body_is_json() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let body = server.client().get_ok("/api/health").await.unwrap();
    // Health endpoint returns JSON with at least a status field
    assert!(body.is_object(), "expected JSON object, got: {body}");
}

// ── Session CRUD ─────────────────────────────────────────────

#[tokio::test]
async fn session_create_and_list() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let c = server.client();

    // List sessions — should be empty initially
    let list = c.get_ok("/api/sessions").await.unwrap();
    let sessions = list["sessions"]
        .as_array()
        .expect("sessions should be array");
    let initial_count = sessions.len();

    // Create a session
    let created = c
        .post_ok("/api/sessions", &json!({"agent_id": "harness-test"}))
        .await
        .unwrap();
    assert!(
        created["id"].is_string(),
        "created session should have an id: {created}"
    );
    let session_id = created["id"].as_str().unwrap();

    // List again — count should increase
    let list2 = c.get_ok("/api/sessions").await.unwrap();
    let sessions2 = list2["sessions"].as_array().unwrap();
    assert_eq!(
        sessions2.len(),
        initial_count + 1,
        "session count should increase by 1"
    );

    // Fetch the specific session
    let fetched = c
        .get_ok(&format!("/api/sessions/{session_id}"))
        .await
        .unwrap();
    assert_eq!(fetched["id"].as_str(), Some(session_id));
}

// ── Config endpoint ──────────────────────────────────────────

#[tokio::test]
async fn config_returns_agent_name() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let config = server.client().get_ok("/api/config").await.unwrap();
    // The config should reflect the generated agent name
    let agent_name = config["agent"]["name"].as_str();
    assert_eq!(
        agent_name,
        Some("HarnessBot"),
        "agent name should be HarnessBot from config_gen defaults"
    );
}

#[tokio::test]
async fn config_capabilities_is_json() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let caps = server
        .client()
        .get_ok("/api/config/capabilities")
        .await
        .unwrap();
    assert!(caps.is_object(), "capabilities should be JSON object");
}

// ── Parallel isolation proof ─────────────────────────────────

#[tokio::test]
async fn parallel_servers_have_different_ports() {
    let s1 = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let s2 = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let s3 = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    assert_ne!(s1.port, s2.port);
    assert_ne!(s2.port, s3.port);
    assert_ne!(s1.port, s3.port);

    // All three should be healthy simultaneously
    for s in [&s1, &s2, &s3] {
        let resp = s.client().get("/api/health").await.unwrap();
        assert!(resp.status().is_success());
    }
}

#[tokio::test]
async fn sessions_are_isolated_across_sandboxes() {
    let s1 = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();
    let s2 = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    // Create session in s1
    s1.client()
        .post_ok("/api/sessions", &json!({"agent_id": "harness-test"}))
        .await
        .unwrap();

    // s2 should still have 0 sessions
    let list = s2.client().get_ok("/api/sessions").await.unwrap();
    let sessions = list["sessions"].as_array().unwrap();
    assert!(
        sessions.is_empty(),
        "s2 should have no sessions, but has {}",
        sessions.len()
    );
}
