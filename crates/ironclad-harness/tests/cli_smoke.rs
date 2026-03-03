//! CLI smoke tests: invoke the real `ironclad` binary against a sandboxed server.
//!
//! Uses `assert_cmd` to locate the binary and `tokio::process::Command` to run
//! CLI subcommands with `--url` pointing at each test's sandboxed server.
//! Validates exit codes and output.
//!
//! KEY: We use `tokio::process::Command` (not `std::process::Command`) because
//! the server runs in-process on the tokio runtime. A blocking `output()` call
//! would deadlock — the CLI waits for the server, but the server can't accept
//! because the thread is blocked.
//!
//! NOTE: These are heavyweight tests — each spawns a real server + CLI
//! binary process. Run with `--test-threads=4` to avoid resource exhaustion.

use ironclad_harness::sandbox::{SandboxMode, SandboxedServer};
use serde_json::json;
use tokio::process::Command;

#[allow(deprecated)] // cargo_bin! macro requires CARGO_BIN_EXE_ env var not available here
fn ironclad_bin() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin("ironclad"))
}

/// Build a CLI command pointing at the given sandbox.
fn cli(server: &SandboxedServer) -> Command {
    let mut cmd = ironclad_bin();
    cmd.arg("--url").arg(&server.base_url);
    cmd.arg("--json");
    cmd.arg("--quiet");
    if let Some(ref key) = server.api_key {
        cmd.arg("--api-key").arg(key);
    }
    cmd
}

// ── Version (offline — no server needed) ────────────────────

#[tokio::test]
async fn cli_version_succeeds() {
    let output = ironclad_bin()
        .arg("version")
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Status ──────────────────────────────────────────────────

#[tokio::test]
async fn cli_status_succeeds() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .arg("status")
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Sessions ────────────────────────────────────────────────

#[tokio::test]
async fn cli_sessions_list() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["sessions", "list"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "sessions list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn cli_sessions_show() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    // Create session via API
    let created = server
        .client()
        .post_ok("/api/sessions", &json!({"agent_id": "harness-test"}))
        .await
        .unwrap();
    let session_id = created["id"].as_str().unwrap();

    let output = cli(&server)
        .args(["sessions", "show", session_id])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "sessions show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Memory ──────────────────────────────────────────────────

#[tokio::test]
async fn cli_memory_list_episodic() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["memory", "list", "episodic"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "memory list episodic failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn cli_memory_list_semantic() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["memory", "list", "semantic"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "memory list semantic failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Skills ──────────────────────────────────────────────────

#[tokio::test]
async fn cli_skills_list() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["skills", "list"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "skills list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Config ──────────────────────────────────────────────────

#[tokio::test]
async fn cli_config_show() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["config", "show"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "config show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Metrics ─────────────────────────────────────────────────

#[tokio::test]
async fn cli_metrics_costs() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["metrics", "costs"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "metrics costs failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Schedule ────────────────────────────────────────────────

#[tokio::test]
async fn cli_schedule_list() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["schedule", "list"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "schedule list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Wallet ──────────────────────────────────────────────────

#[tokio::test]
async fn cli_wallet_balance() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["wallet", "balance"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "wallet balance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── Circuit breaker ─────────────────────────────────────────

#[tokio::test]
async fn cli_circuit_status() {
    let server = SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .unwrap();

    let output = cli(&server)
        .args(["circuit", "status"])
        .output()
        .await
        .expect("failed to run");
    assert!(
        output.status.success(),
        "circuit status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
