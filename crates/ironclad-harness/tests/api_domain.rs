//! Domain endpoint tests: stats, skills, cron, wallet, browser,
//! agents, subagents, breaker, runtime, channels, approvals, audit.
//!
//! These validate that each route group is wired correctly and
//! returns well-formed JSON. Most return empty/zeroed data on a
//! fresh server — that's fine; we're proving the plumbing works.

use ironclad_harness::sandbox::{SandboxMode, SandboxedServer};
use serde_json::json;

async fn spawn() -> SandboxedServer {
    SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .expect("sandbox spawn failed")
}

// ── Statistics & Analytics ──────────────────────────────────

#[tokio::test]
async fn stats_costs_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/stats/costs").await.unwrap();
    assert!(body.is_object() || body.is_array(), "costs: {body}");
}

#[tokio::test]
async fn stats_timeseries_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/stats/timeseries").await.unwrap();
    assert!(body.is_object() || body.is_array(), "timeseries: {body}");
}

#[tokio::test]
async fn stats_efficiency_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/stats/efficiency").await.unwrap();
    assert!(body.is_object() || body.is_array(), "efficiency: {body}");
}

#[tokio::test]
async fn stats_transactions_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/stats/transactions").await.unwrap();
    assert!(body.is_object() || body.is_array(), "transactions: {body}");
}

#[tokio::test]
async fn stats_cache_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/stats/cache").await.unwrap();
    assert!(body.is_object() || body.is_array(), "cache: {body}");
}

#[tokio::test]
async fn stats_capacity_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/stats/capacity").await.unwrap();
    assert!(body.is_object() || body.is_array(), "capacity: {body}");
}

#[tokio::test]
async fn recommendations_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/recommendations").await.unwrap();
    assert!(
        body.is_object() || body.is_array(),
        "recommendations: {body}"
    );
}

// ── Skills ──────────────────────────────────────────────────

#[tokio::test]
async fn skills_list_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/skills").await.unwrap();
    assert!(body.is_object() || body.is_array(), "skills: {body}");
}

#[tokio::test]
async fn skills_catalog_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/skills/catalog").await.unwrap();
    assert!(body.is_object() || body.is_array(), "catalog: {body}");
}

#[tokio::test]
async fn skills_audit_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/skills/audit").await.unwrap();
    assert!(body.is_object() || body.is_array(), "skills audit: {body}");
}

#[tokio::test]
async fn skills_reload_accepts_post() {
    let s = spawn().await;
    let resp = s
        .client()
        .post_json("/api/skills/reload", &json!({}))
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "skills reload: {}",
        resp.status()
    );
}

// ── Plugins ─────────────────────────────────────────────────

#[tokio::test]
async fn plugins_list_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/plugins").await.unwrap();
    assert!(body.is_object() || body.is_array(), "plugins: {body}");
}

// ── Cron ────────────────────────────────────────────────────

#[tokio::test]
async fn cron_jobs_list_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/cron/jobs").await.unwrap();
    assert!(body.is_object() || body.is_array(), "cron jobs: {body}");
}

#[tokio::test]
async fn cron_runs_list_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/cron/runs").await.unwrap();
    assert!(body.is_object() || body.is_array(), "cron runs: {body}");
}

#[tokio::test]
async fn cron_job_crud_lifecycle() {
    let s = spawn().await;
    let c = s.client();

    // Create a cron job
    let created = c
        .post_ok(
            "/api/cron/jobs",
            &json!({
                "name": "test-job",
                "schedule_kind": "cron",
                "schedule_expr": "0 */6 * * *"
            }),
        )
        .await
        .unwrap();
    let job_id = created["job_id"]
        .as_str()
        .expect("cron create should return job_id");

    // Get the job
    let fetched = c.get_ok(&format!("/api/cron/jobs/{job_id}")).await.unwrap();
    assert_eq!(fetched["name"].as_str(), Some("test-job"));

    // Update the job
    let resp = c
        .put_json(
            &format!("/api/cron/jobs/{job_id}"),
            &json!({"name": "updated-job"}),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success(), "update: {}", resp.status());

    // Delete the job
    let resp = c.delete(&format!("/api/cron/jobs/{job_id}")).await.unwrap();
    assert!(resp.status().is_success(), "delete: {}", resp.status());

    // Verify deleted
    let resp = c.get(&format!("/api/cron/jobs/{job_id}")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

// ── Wallet ──────────────────────────────────────────────────

#[tokio::test]
async fn wallet_balance_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/wallet/balance").await.unwrap();
    assert!(body.is_object(), "wallet balance: {body}");
}

#[tokio::test]
async fn wallet_address_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/wallet/address").await.unwrap();
    assert!(
        body.is_object() || body.is_string(),
        "wallet address: {body}"
    );
}

// ── Browser ─────────────────────────────────────────────────

#[tokio::test]
async fn browser_status_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/browser/status").await.unwrap();
    assert!(body.is_object(), "browser status: {body}");
}

// ── Agent (status only — message requires LLM) ─────────────

#[tokio::test]
async fn agent_status_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/agent/status").await.unwrap();
    assert!(body.is_object(), "agent status: {body}");
}

// ── Agents & Roster ─────────────────────────────────────────

#[tokio::test]
async fn agents_list_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/agents").await.unwrap();
    assert!(body.is_object() || body.is_array(), "agents: {body}");
}

#[tokio::test]
async fn roster_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/roster").await.unwrap();
    assert!(body.is_object() || body.is_array(), "roster: {body}");
}

// ── Subagents ───────────────────────────────────────────────

#[tokio::test]
async fn subagents_list_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/subagents").await.unwrap();
    assert!(body.is_object() || body.is_array(), "subagents: {body}");
}

#[tokio::test]
async fn subagent_crud_lifecycle() {
    let s = spawn().await;
    let c = s.client();

    // Create a subagent
    let created = c
        .post_ok(
            "/api/subagents",
            &json!({
                "name": "test-sub",
                "model": "openai/gpt-4o-mini",
                "system_prompt": "You are a test helper."
            }),
        )
        .await
        .unwrap();
    assert!(
        created["name"].as_str() == Some("test-sub") || created["name"].is_string(),
        "subagent should have name: {created}"
    );

    // Update it
    let resp = c
        .put_json(
            "/api/subagents/test-sub",
            &json!({"system_prompt": "Updated prompt."}),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success(), "update: {}", resp.status());

    // Toggle it
    let resp = c
        .put_json("/api/subagents/test-sub/toggle", &json!({}))
        .await
        .unwrap();
    assert!(resp.status().is_success(), "toggle: {}", resp.status());

    // Delete it
    let resp = c.delete("/api/subagents/test-sub").await.unwrap();
    assert!(resp.status().is_success(), "delete: {}", resp.status());
}

// ── Circuit Breaker ─────────────────────────────────────────

#[tokio::test]
async fn breaker_status_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/breaker/status").await.unwrap();
    assert!(body.is_object() || body.is_array(), "breaker: {body}");
}

// ── Workspace & Runtime ─────────────────────────────────────

#[tokio::test]
async fn workspace_state_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/workspace/state").await.unwrap();
    assert!(body.is_object(), "workspace state: {body}");
}

#[tokio::test]
async fn runtime_surfaces_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/runtime/surfaces").await.unwrap();
    assert!(body.is_object() || body.is_array(), "surfaces: {body}");
}

#[tokio::test]
async fn runtime_discovery_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/runtime/discovery").await.unwrap();
    assert!(body.is_object() || body.is_array(), "discovery: {body}");
}

#[tokio::test]
async fn runtime_devices_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/runtime/devices").await.unwrap();
    assert!(body.is_object() || body.is_array(), "devices: {body}");
}

#[tokio::test]
async fn runtime_mcp_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/runtime/mcp").await.unwrap();
    assert!(body.is_object() || body.is_array(), "mcp: {body}");
}

// ── Channels ────────────────────────────────────────────────

#[tokio::test]
async fn channels_status_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/channels/status").await.unwrap();
    assert!(body.is_object() || body.is_array(), "channels: {body}");
}

#[tokio::test]
async fn channels_dead_letter_returns_json() {
    let s = spawn().await;
    let body = s
        .client()
        .get_ok("/api/channels/dead-letter")
        .await
        .unwrap();
    assert!(body.is_object() || body.is_array(), "dead-letter: {body}");
}

// ── Approvals ───────────────────────────────────────────────

#[tokio::test]
async fn approvals_list_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/approvals").await.unwrap();
    assert!(body.is_object() || body.is_array(), "approvals: {body}");
}

// ── Models ──────────────────────────────────────────────────

#[tokio::test]
async fn models_selections_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/models/selections").await.unwrap();
    assert!(body.is_object() || body.is_array(), "selections: {body}");
}

#[tokio::test]
async fn models_available_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/models/available").await.unwrap();
    assert!(body.is_object() || body.is_array(), "available: {body}");
}

// ── Logs ────────────────────────────────────────────────────

#[tokio::test]
async fn logs_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/logs").await.unwrap();
    assert!(body.is_object() || body.is_array(), "logs: {body}");
}

// ── WebSocket ticket ────────────────────────────────────────

#[tokio::test]
async fn ws_ticket_returns_token() {
    let s = spawn().await;
    let body = s
        .client()
        .post_ok("/api/ws-ticket", &json!({}))
        .await
        .unwrap();
    assert!(
        body["ticket"].is_string() || body["token"].is_string(),
        "ws-ticket should return a ticket/token: {body}"
    );
}

// ── Public endpoints (no auth) ──────────────────────────────

#[tokio::test]
async fn well_known_agent_json_returns_json() {
    let s = spawn().await;
    // This is a public endpoint — use raw reqwest without auth header
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/.well-known/agent.json", s.base_url))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "agent.json: {}", resp.status());
}

#[tokio::test]
async fn dashboard_returns_html() {
    let s = spawn().await;
    let client = reqwest::Client::new();
    let resp = client.get(&s.base_url).send().await.unwrap();
    assert!(resp.status().is_success(), "dashboard: {}", resp.status());
    let content_type = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        content_type.contains("text/html"),
        "dashboard should be HTML, got: {content_type}"
    );
}
