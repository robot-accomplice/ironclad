//! CLI / Web API parity tests (Task 35 -- v0.8.0 stabilization).
//!
//! The ironclad CLI commands are thin HTTP clients that call the same API
//! endpoints served to the web dashboard.  Both paths ultimately call
//! into the shared `ironclad_db` layer.  These tests verify that:
//!
//! 1. Data written through the DB layer is faithfully surfaced by the API.
//! 2. The API JSON shapes match the fields the CLI commands parse.
//! 3. Operations that exist in both paths produce equivalent results when
//!    given the same underlying state.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironclad_agent::subagents::SubagentRegistry;
use ironclad_browser::Browser;
use ironclad_channels::a2a::A2aProtocol;
use ironclad_core::IroncladConfig;
use ironclad_db::Database;
use ironclad_llm::{LlmService, OAuthManager};
use ironclad_plugin_sdk::registry::PluginRegistry;
use ironclad_server::config_runtime::ConfigApplyStatus;
use ironclad_server::{AppState, EventBus, PersonalityState, build_router};
use ironclad_wallet::{TreasuryPolicy, WalletService, YieldEngine};
use tokio::sync::RwLock;
use tower::ServiceExt;

// ── Helpers ────────────────────────────────────────────────────────

const TEST_CONFIG_TOML: &str = r#"
[agent]
name = "ParityTestBot"
id = "parity-test"

[server]
port = 0

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"
fallbacks = ["ollama/llama3:8b"]

[models.routing]
mode = "rule"
confidence_threshold = 0.85
local_first = true
"#;

fn test_state() -> AppState {
    let db = Database::new(":memory:").unwrap();
    let config = IroncladConfig::from_str(TEST_CONFIG_TOML).unwrap();
    let llm = LlmService::new(&config).unwrap();
    let a2a = A2aProtocol::new(config.a2a.clone());
    let wallet = ironclad_wallet::Wallet::test_mock();
    let treasury = TreasuryPolicy::new(&config.treasury);
    let yield_engine = YieldEngine::new(&config.r#yield);
    let wallet_svc = WalletService {
        wallet,
        treasury,
        yield_engine,
    };
    let plugins = Arc::new(PluginRegistry::new(vec![], vec![]));
    let browser = Arc::new(Browser::new(ironclad_core::config::BrowserConfig::default()));
    let registry = Arc::new(SubagentRegistry::new(4, vec![]));
    let event_bus = EventBus::new(16);
    let channel_router = Arc::new(ironclad_channels::router::ChannelRouter::new());
    let retriever = Arc::new(ironclad_agent::retrieval::MemoryRetriever::new(
        config.memory.clone(),
    ));
    let config_path = std::env::temp_dir().join(format!(
        "ironclad-parity-config-{}.toml",
        uuid::Uuid::new_v4()
    ));
    AppState {
        db,
        config: Arc::new(RwLock::new(config)),
        llm: Arc::new(RwLock::new(llm)),
        wallet: Arc::new(wallet_svc),
        a2a: Arc::new(RwLock::new(a2a)),
        personality: Arc::new(RwLock::new(PersonalityState::empty())),
        hmac_secret: Arc::new(b"test-hmac-secret-key-for-tests!!".to_vec()),
        interviews: Arc::new(RwLock::new(std::collections::HashMap::new())),
        plugins,
        browser,
        registry,
        event_bus,
        channel_router,
        telegram: None,
        whatsapp: None,
        retriever,
        ann_index: ironclad_db::ann::AnnIndex::new(false),
        tools: Arc::new(ironclad_agent::tools::ToolRegistry::new()),
        approvals: Arc::new(ironclad_agent::approvals::ApprovalManager::new(
            ironclad_core::config::ApprovalsConfig::default(),
        )),
        discord: None,
        signal: None,
        email: None,
        voice: None,
        discovery: Arc::new(RwLock::new(
            ironclad_agent::discovery::DiscoveryRegistry::new(),
        )),
        devices: Arc::new(RwLock::new(ironclad_agent::device::DeviceManager::new(
            ironclad_agent::device::DeviceIdentity::generate("parity-test-device"),
            5,
        ))),
        mcp_clients: Arc::new(RwLock::new(ironclad_agent::mcp::McpClientManager::new())),
        mcp_server: Arc::new(RwLock::new(ironclad_agent::mcp::McpServerRegistry::new())),
        oauth: Arc::new(OAuthManager::new().unwrap()),
        keystore: Arc::new(ironclad_core::keystore::Keystore::new(
            std::env::temp_dir().join(format!("ironclad-parity-ks-{}.enc", uuid::Uuid::new_v4())),
        )),
        obsidian: None,
        started_at: std::time::Instant::now(),
        config_path: Arc::new(config_path.clone()),
        config_apply_status: Arc::new(RwLock::new(ConfigApplyStatus::new(&config_path))),
        pending_specialist_proposals: Arc::new(RwLock::new(std::collections::HashMap::new())),
        policy_engine: {
            let mut engine = ironclad_agent::policy::PolicyEngine::new();
            engine.add_rule(Box::new(ironclad_agent::policy::AuthorityRule));
            engine.add_rule(Box::new(ironclad_agent::policy::CommandSafetyRule));
            Arc::new(engine)
        },
    }
}

async fn json_body(resp: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ── (a) Session listing parity ────────────────────────────────────
//
// Both the CLI (`ironclad sessions`) and the API (`GET /api/sessions`)
// ultimately read from the same `sessions` table.  This test writes
// sessions through the DB layer, then asserts the API returns the same
// set with the field names the CLI parses.

#[tokio::test]
async fn session_list_parity_db_vs_api() {
    let state = test_state();

    // Write two sessions directly through the DB layer.
    let sid1 = ironclad_db::sessions::find_or_create(&state.db, "agent-alpha", None).unwrap();
    let sid2 = ironclad_db::sessions::find_or_create(&state.db, "agent-beta", None).unwrap();

    // Read via the API (the same endpoint the CLI calls).
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/sessions")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = json_body(resp).await;
    let sessions = body["sessions"].as_array().expect("sessions array");

    // Both sessions must appear.
    assert_eq!(sessions.len(), 2);
    let api_ids: Vec<&str> = sessions.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(api_ids.contains(&sid1.as_str()));
    assert!(api_ids.contains(&sid2.as_str()));

    // Verify field names the CLI parses are present in every session.
    for s in sessions {
        assert!(s["id"].is_string(), "CLI parses 'id'");
        assert!(s["agent_id"].is_string(), "CLI parses 'agent_id'");
        assert!(s["updated_at"].is_string(), "CLI parses 'updated_at'");
        // nickname may be null but the key must exist.
        assert!(s.get("nickname").is_some(), "CLI parses 'nickname'");
    }
}

// ── (a') Session creation parity ──────────────────────────────────
//
// CLI `ironclad session new <agent>` calls `POST /api/sessions` which
// calls `rotate_agent_session`.  Verify the DB layer and the API layer
// agree on what was created.

#[tokio::test]
async fn session_create_parity_api_then_db_lookup() {
    let state = test_state();

    // Create via API (as CLI would).
    let app = build_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/sessions")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"agent_id":"parity-agent"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let api_session_id = body["id"].as_str().unwrap().to_string();

    // The DB must know about this session.
    let db_session = ironclad_db::sessions::get_session(&state.db, &api_session_id).unwrap();
    assert!(db_session.is_some(), "API-created session visible in DB");
    let db_session = db_session.unwrap();
    assert_eq!(db_session.agent_id, "parity-agent");
    assert_eq!(db_session.status, "active");

    // And the API GET must return the same session.
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri(format!("/api/sessions/{api_session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let get_body = json_body(resp).await;
    assert_eq!(get_body["id"].as_str().unwrap(), api_session_id);
    assert_eq!(get_body["agent_id"].as_str().unwrap(), "parity-agent");
}

// ── (a'') Messages parity ─────────────────────────────────────────
//
// Messages written through the DB layer must be visible through the API
// (which is what the CLI reads for `ironclad session <id>`).

#[tokio::test]
async fn messages_parity_db_write_api_read() {
    let state = test_state();

    let sid = ironclad_db::sessions::find_or_create(&state.db, "msg-parity", None).unwrap();

    // Write messages via the DB layer.
    let mid1 =
        ironclad_db::sessions::append_message(&state.db, &sid, "user", "Hello from DB").unwrap();
    let mid2 =
        ironclad_db::sessions::append_message(&state.db, &sid, "assistant", "Hello back from DB")
            .unwrap();

    // Read via the API (the path the CLI uses).
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri(format!("/api/sessions/{sid}/messages"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);

    // Verify content parity.
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "Hello from DB");
    assert_eq!(messages[0]["id"].as_str().unwrap(), mid1);
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"], "Hello back from DB");
    assert_eq!(messages[1]["id"].as_str().unwrap(), mid2);

    // And vice versa: write via API, read from DB.
    let app = build_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{sid}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"role":"user","content":"Hello from API"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let post_body = json_body(resp).await;
    let api_mid = post_body["message_id"].as_str().unwrap().to_string();

    let db_msgs = ironclad_db::sessions::list_messages(&state.db, &sid, Some(200)).unwrap();
    assert_eq!(db_msgs.len(), 3);
    let api_msg = db_msgs.iter().find(|m| m.id == api_mid).unwrap();
    assert_eq!(api_msg.role, "user");
    assert_eq!(api_msg.content, "Hello from API");
}

// ── (b) Config display parity ─────────────────────────────────────
//
// The CLI (`ironclad config`) calls `GET /api/config` and renders named
// sections.  The API serializes the in-memory `IroncladConfig`.  This
// test verifies the JSON contains the sections the CLI iterates and that
// the model fields are populated as expected.

#[tokio::test]
async fn config_display_parity() {
    let state = test_state();

    // The config is set in memory by `test_state()`.  The CLI reads
    // it from the API endpoint.
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/config")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = json_body(resp).await;

    // The CLI iterates these section names -- they must exist.
    let cli_sections = ["agent", "server", "database", "models", "memory", "cache"];
    for section in &cli_sections {
        assert!(
            body.get(section).is_some(),
            "config JSON must contain section '{section}'"
        );
    }

    // Model fields the CLI extracts.
    assert_eq!(
        body["models"]["primary"].as_str().unwrap(),
        "ollama/qwen3:8b"
    );
    let fallbacks = body["models"]["fallbacks"].as_array().unwrap();
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(fallbacks[0].as_str().unwrap(), "ollama/llama3:8b");

    // Agent fields the CLI status command reads.
    assert_eq!(body["agent"]["name"].as_str().unwrap(), "ParityTestBot");
    assert_eq!(body["agent"]["id"].as_str().unwrap(), "parity-test");
}

// ── (b') Config determinism ───────────────────────────────────────
//
// Two calls to `GET /api/config` against the same state must produce
// byte-identical JSON (deterministic serialization).

#[tokio::test]
async fn config_serialization_is_deterministic() {
    let state = test_state();

    let fetch = |s: AppState| async move {
        let app = build_router(s);
        let req = Request::builder()
            .uri("/api/config")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        bytes.to_vec()
    };

    let first = fetch(state.clone()).await;
    let second = fetch(state.clone()).await;
    assert_eq!(first, second, "config serialization must be deterministic");
}

// ── (c) Health / status parity ────────────────────────────────────
//
// The CLI (`ironclad status`) reads `/api/health` to get version and
// model info, plus `/api/agent/status` for the running state.  Both
// must be well-formed and consistent with `AppState`.

#[tokio::test]
async fn health_status_parity() {
    let state = test_state();

    // /api/health
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let health = json_body(resp).await;

    assert_eq!(health["status"], "ok");
    assert!(health["version"].is_string());
    assert_eq!(health["agent"].as_str().unwrap(), "ParityTestBot");
    assert!(health["uptime_seconds"].is_number());

    // Model info the CLI reads from health.
    let models = &health["models"];
    assert_eq!(models["primary"].as_str().unwrap(), "ollama/qwen3:8b");
    assert!(models["current"].is_string());
    assert!(models["fallbacks"].is_array());

    // /api/agent/status
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/agent/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let agent = json_body(resp).await;

    assert_eq!(agent["state"].as_str().unwrap(), "running");
    assert_eq!(agent["agent_name"].as_str().unwrap(), "ParityTestBot");
    assert_eq!(agent["agent_id"].as_str().unwrap(), "parity-test");
}

// ── (c') Health and config agree on agent name ────────────────────
//
// The CLI `status` command reads the agent name from `/api/config` and
// the health agent field from `/api/health`.  They must match.

#[tokio::test]
async fn health_and_config_agent_name_agree() {
    let state = test_state();

    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let health = json_body(resp).await;

    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/config")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let config = json_body(resp).await;

    assert_eq!(
        health["agent"].as_str().unwrap(),
        config["agent"]["name"].as_str().unwrap(),
        "health.agent must match config.agent.name (CLI reads both)"
    );
}

// ── (d) Model listing parity ──────────────────────────────────────
//
// CLI `ironclad models` reads `/api/config` and extracts model fields.
// Verify the JSON shape matches what the CLI parser expects.

#[tokio::test]
async fn model_listing_parity() {
    let state = test_state();

    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/config")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let config = json_body(resp).await;

    // CLI `cmd_models_list` reads these exact JSON pointer paths.
    let primary = config
        .pointer("/models/primary")
        .and_then(|v| v.as_str())
        .expect("primary model must be present");
    assert_eq!(primary, "ollama/qwen3:8b");

    let fallbacks = config
        .pointer("/models/fallbacks")
        .and_then(|v| v.as_array())
        .expect("fallbacks must be array");
    assert_eq!(fallbacks.len(), 1);

    let mode = config
        .pointer("/models/routing/mode")
        .and_then(|v| v.as_str())
        .expect("routing mode must be present");
    assert_eq!(mode, "rule");

    let threshold = config
        .pointer("/models/routing/confidence_threshold")
        .and_then(|v| v.as_f64())
        .expect("confidence_threshold must be present");
    assert!((threshold - 0.85).abs() < f64::EPSILON);

    let local_first = config
        .pointer("/models/routing/local_first")
        .and_then(|v| v.as_bool())
        .expect("local_first must be present");
    assert!(local_first);
}

// ── (d') Health model fields match config model fields ────────────
//
// The health endpoint includes a `models.primary` field that should
// agree with the config endpoint.

#[tokio::test]
async fn health_models_match_config_models() {
    let state = test_state();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let health = json_body(resp).await;

    let app = build_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let config = json_body(resp).await;

    assert_eq!(
        health["models"]["primary"].as_str().unwrap(),
        config["models"]["primary"].as_str().unwrap(),
        "health.models.primary must equal config.models.primary"
    );

    let health_fallbacks = health["models"]["fallbacks"]
        .as_array()
        .expect("health fallbacks");
    let config_fallbacks = config["models"]["fallbacks"]
        .as_array()
        .expect("config fallbacks");
    assert_eq!(
        health_fallbacks, config_fallbacks,
        "fallback lists must be identical across health and config endpoints"
    );
}

// ── (e) Session count parity ──────────────────────────────────────
//
// The CLI `status` command reads session count from `/api/sessions`.
// Verify the count matches after DB writes.

#[tokio::test]
async fn session_count_parity() {
    let state = test_state();

    // No sessions initially.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = json_body(resp).await;
    let count_before = body["sessions"].as_array().unwrap().len();
    assert_eq!(count_before, 0);

    // Create 3 sessions via DB.
    for i in 0..3 {
        ironclad_db::sessions::find_or_create(&state.db, &format!("count-agent-{i}"), None)
            .unwrap();
    }

    // API count must reflect all 3.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = json_body(resp).await;
    let count_after = body["sessions"].as_array().unwrap().len();
    assert_eq!(count_after, 3);
}

// ── (f) Backfill nicknames parity ─────────────────────────────────
//
// Both CLI and API call `ironclad_db::sessions::backfill_nicknames`.
// The API response must reflect the DB return value.

#[tokio::test]
async fn backfill_nicknames_parity() {
    let state = test_state();

    // Create a session without a nickname.
    ironclad_db::sessions::find_or_create(&state.db, "nickname-test", None).unwrap();

    // Call through API (as CLI does).
    let app = build_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/sessions/backfill-nicknames")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = json_body(resp).await;
    // The count should be a non-negative integer (the CLI reads this).
    assert!(
        body["backfilled"].is_number(),
        "backfilled count must be a number"
    );
    let count = body["backfilled"].as_u64().unwrap();

    // Calling again should return 0 (already backfilled).
    let app = build_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/sessions/backfill-nicknames")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body2 = json_body(resp).await;
    let count2 = body2["backfilled"].as_u64().unwrap();

    // Second call should backfill fewer or equal sessions.
    assert!(
        count2 <= count,
        "second backfill should not exceed the first"
    );
}

// ── (g) Error response parity ─────────────────────────────────────
//
// Both CLI and API paths should handle invalid requests gracefully.
// The CLI handles 404s from the API; verify the API returns proper
// error status codes so the CLI can detect them.

#[tokio::test]
async fn error_response_404_for_missing_session() {
    let state = test_state();

    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/sessions/nonexistent-session-id")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "missing session returns 404 (CLI checks this)"
    );
}

#[tokio::test]
async fn error_response_400_for_invalid_message_role() {
    let state = test_state();
    let sid = ironclad_db::sessions::find_or_create(&state.db, "err-test", None).unwrap();

    let app = build_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{sid}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"role":"invalid","content":"test"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "invalid role returns 400"
    );
}

// ── (h) Skills listing field parity ───────────────────────────────
//
// The CLI `ironclad status` reads skills count from `/api/skills`.
// Verify the endpoint returns the expected shape.

#[tokio::test]
async fn skills_list_shape_for_cli() {
    let state = test_state();

    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/skills")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = json_body(resp).await;
    let skills = body["skills"].as_array().expect("skills must be an array");

    // CLI status reads `.len()` -- we just need the array to exist.
    // Each skill must have at least the `name` and `enabled` fields.
    for skill in skills {
        assert!(skill["name"].is_string(), "skill must have 'name'");
        assert!(skill["enabled"].is_boolean(), "skill must have 'enabled'");
    }
}

// ── (i) Cron jobs listing field parity ────────────────────────────
//
// The CLI `ironclad status` reads job count from `/api/cron/jobs`.

#[tokio::test]
async fn cron_jobs_list_shape_for_cli() {
    let state = test_state();

    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/cron/jobs")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = json_body(resp).await;
    assert!(
        body["jobs"].is_array(),
        "cron jobs response must have 'jobs' array"
    );
}
