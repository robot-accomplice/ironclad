//! Memory endpoint tests: working, episodic, semantic, search.
//!
//! On a fresh server these return empty collections — we validate
//! the status codes and JSON structure are correct.

use ironclad_harness::sandbox::{SandboxMode, SandboxedServer};

async fn spawn() -> SandboxedServer {
    SandboxedServer::spawn(SandboxMode::InProcess)
        .await
        .expect("sandbox spawn failed")
}

// ── Working memory ──────────────────────────────────────────

#[tokio::test]
async fn working_memory_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/memory/working").await.unwrap();
    assert!(
        body.is_object() || body.is_array(),
        "working memory should be JSON: {body}"
    );
}

#[tokio::test]
async fn working_memory_by_session_returns_404_or_empty() {
    let s = spawn().await;
    let resp = s
        .client()
        .get("/api/memory/working/nonexistent-session")
        .await
        .unwrap();
    // Either 404 (session doesn't exist) or 200 with empty result
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 404,
        "expected 200 or 404 for unknown session, got {status}"
    );
}

// ── Episodic memory ─────────────────────────────────────────

#[tokio::test]
async fn episodic_memory_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/memory/episodic").await.unwrap();
    assert!(
        body.is_object() || body.is_array(),
        "episodic memory should be JSON: {body}"
    );
}

// ── Semantic memory ─────────────────────────────────────────

#[tokio::test]
async fn semantic_memory_all_returns_json() {
    let s = spawn().await;
    let body = s.client().get_ok("/api/memory/semantic").await.unwrap();
    assert!(
        body.is_object() || body.is_array(),
        "semantic memory should be JSON: {body}"
    );
}

#[tokio::test]
async fn semantic_memory_categories_returns_json() {
    let s = spawn().await;
    let body = s
        .client()
        .get_ok("/api/memory/semantic/categories")
        .await
        .unwrap();
    assert!(
        body.is_object() || body.is_array(),
        "semantic categories should be JSON: {body}"
    );
}

#[tokio::test]
async fn semantic_memory_by_category_returns_json() {
    let s = spawn().await;
    // Even a nonexistent category should return 200 with empty results
    let resp = s
        .client()
        .get("/api/memory/semantic/test-category")
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 404,
        "semantic by category: expected 200 or 404, got {status}"
    );
}

// ── Memory search ───────────────────────────────────────────

#[tokio::test]
async fn memory_search_returns_json() {
    let s = spawn().await;
    let resp = s.client().get("/api/memory/search?q=test").await.unwrap();
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 400,
        "memory search: expected 200 or 400, got {status}"
    );
}
