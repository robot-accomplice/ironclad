//! Process-mode tests: spawn the real `ironclad` binary as a child process.
//!
//! These tests are **feature-gated** behind `full-process` and are NOT run in CI.
//! They validate that the real binary boots, accepts requests, and shuts down
//! cleanly. This is the highest-fidelity test mode but contributes no coverage.
//!
//! Run: `cargo test -p ironclad-harness --features full-process --test process_mode`

#![cfg(feature = "full-process")]

use ironclad_harness::sandbox::{SandboxMode, SandboxedServer};

async fn spawn_process() -> SandboxedServer {
    SandboxedServer::spawn(SandboxMode::Process)
        .await
        .expect("process-mode spawn failed — is `ironclad` binary built?")
}

#[tokio::test]
async fn process_mode_health() {
    let s = spawn_process().await;
    let resp = s.client().get("/api/health").await.unwrap();
    assert!(resp.status().is_success(), "health check should pass");
}

#[tokio::test]
async fn process_mode_session_crud() {
    let s = spawn_process().await;
    let c = s.client();

    // Create session
    let created = c
        .post_ok("/api/sessions", &serde_json::json!({}))
        .await
        .unwrap();
    let session_id = created["id"].as_str().unwrap();

    // List sessions
    let list = c.get_ok("/api/sessions").await.unwrap();
    let sessions = list["sessions"].as_array().unwrap();
    assert!(
        sessions
            .iter()
            .any(|s| s["id"].as_str() == Some(session_id)),
        "created session should appear in list"
    );

    // Get session
    let fetched = c
        .get_ok(&format!("/api/sessions/{session_id}"))
        .await
        .unwrap();
    assert_eq!(fetched["id"].as_str(), Some(session_id));
}

#[tokio::test]
async fn process_mode_config() {
    let s = spawn_process().await;
    let v = s.client().get_ok("/api/config").await.unwrap();
    assert!(v.is_object(), "config should return JSON object");
}

#[tokio::test]
async fn process_mode_dashboard() {
    let s = spawn_process().await;
    let resp = s.client().get("/").await.unwrap();
    assert!(resp.status().is_success());
    let body = resp.text().await.unwrap();
    assert!(body.contains("Ironclad Dashboard"));
}
