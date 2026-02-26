use ironclad_agent::memory::MemoryBudgetManager;
use ironclad_agent::retrieval::MemoryRetriever;
use ironclad_agent::context::ComplexityLevel;
use ironclad_core::config::MemoryConfig;
use ironclad_db::Database;

#[test]
fn store_and_retrieve_all_memory_tiers() {
    let db = Database::new(":memory:").unwrap();
    let session_id = ironclad_db::sessions::find_or_create(&db, "memory-test-agent", None).unwrap();

    ironclad_db::memory::store_working(&db, &session_id, "goal", "complete integration tests", 9)
        .unwrap();
    let working = ironclad_db::memory::retrieve_working(&db, &session_id).unwrap();
    assert_eq!(working.len(), 1);
    assert_eq!(working[0].content, "complete integration tests");

    ironclad_db::memory::store_episodic(&db, "success", "first deployment succeeded", 8).unwrap();
    let episodic = ironclad_db::memory::retrieve_episodic(&db, 10).unwrap();
    assert_eq!(episodic.len(), 1);
    assert_eq!(episodic[0].classification, "success");

    ironclad_db::memory::store_semantic(&db, "facts", "language", "Rust", 0.95).unwrap();
    let semantic = ironclad_db::memory::retrieve_semantic(&db, "facts").unwrap();
    assert_eq!(semantic.len(), 1);
    assert_eq!(semantic[0].key, "language");
    assert_eq!(semantic[0].value, "Rust");

    ironclad_db::memory::store_procedural(
        &db,
        "git-workflow",
        r#"["branch","commit","push","pr"]"#,
    )
    .unwrap();
    let procedural = ironclad_db::memory::retrieve_procedural(&db, "git-workflow")
        .unwrap()
        .unwrap();
    assert_eq!(procedural.name, "git-workflow");

    ironclad_db::memory::store_relationship(&db, "user-jon", "Jon", 0.95).unwrap();
    let relationship = ironclad_db::memory::retrieve_relationship(&db, "user-jon")
        .unwrap()
        .unwrap();
    assert_eq!(relationship.entity_name.as_deref(), Some("Jon"));
    assert!((relationship.trust_score - 0.95).abs() < f64::EPSILON);
}

#[test]
fn budget_allocation_matches_config() {
    let config = MemoryConfig {
        working_budget_pct: 30.0,
        episodic_budget_pct: 25.0,
        semantic_budget_pct: 20.0,
        procedural_budget_pct: 15.0,
        relationship_budget_pct: 10.0,
        embedding_provider: None,
        embedding_model: None,
        hybrid_weight: 0.5,
        ann_index: false,
    };

    let manager = MemoryBudgetManager::new(config);
    let budgets = manager.allocate_budgets(10_000);

    assert_eq!(budgets.working, 3_000);
    assert_eq!(budgets.episodic, 2_500);
    assert_eq!(budgets.semantic, 2_000);
    assert_eq!(budgets.procedural, 1_500);
    assert_eq!(budgets.relationship, 1_000);

    let total = budgets.working
        + budgets.episodic
        + budgets.semantic
        + budgets.procedural
        + budgets.relationship;
    assert_eq!(total, 10_000);
}

#[test]
fn budget_rollover_assigned_to_working() {
    let config = MemoryConfig {
        working_budget_pct: 30.0,
        episodic_budget_pct: 25.0,
        semantic_budget_pct: 20.0,
        procedural_budget_pct: 15.0,
        relationship_budget_pct: 10.0,
        embedding_provider: None,
        embedding_model: None,
        hybrid_weight: 0.5,
        ann_index: false,
    };

    let manager = MemoryBudgetManager::new(config);
    let budgets = manager.allocate_budgets(99);

    let total = budgets.working
        + budgets.episodic
        + budgets.semantic
        + budgets.procedural
        + budgets.relationship;
    assert_eq!(total, 99, "all tokens distributed even with rounding");
}

#[test]
fn full_text_search_across_tiers() {
    let db = Database::new(":memory:").unwrap();
    let session_id = ironclad_db::sessions::find_or_create(&db, "fts-test-agent", None).unwrap();

    ironclad_db::memory::store_working(&db, &session_id, "note", "the quick brown fox", 5).unwrap();
    ironclad_db::memory::store_episodic(&db, "event", "a lazy dog appeared", 5).unwrap();
    ironclad_db::memory::store_semantic(&db, "facts", "animal", "foxes are quick", 0.8).unwrap();
    ironclad_db::memory::store_procedural(&db, "catch-fox", "run quickly after the fox").unwrap();

    let hits = ironclad_db::memory::fts_search(&db, "quick", 10).unwrap();
    assert!(
        hits.len() >= 2,
        "should match in working + semantic at minimum, got {}",
        hits.len()
    );

    let fox_hits = ironclad_db::memory::fts_search(&db, "fox", 10).unwrap();
    assert!(fox_hits.len() >= 2, "fox should appear in multiple tiers");
}

/// 9A: After ingest_turn, correct memories appear in working, episodic, semantic, procedural.
#[test]
fn memory_ingestion_after_ingest_turn_tiers() {
    let db = Database::new(":memory:").unwrap();
    let session_id = ironclad_db::sessions::find_or_create(&db, "ingest-test", None).unwrap();

    ironclad_agent::memory::ingest_turn(
        &db,
        &session_id,
        "What is Rust?",
        "Rust is a systems programming language focused on safety, concurrency, and zero-cost abstractions. It has a rich type system and memory safety without garbage collection.",
        &[],
    );

    let working = ironclad_db::memory::retrieve_working(&db, &session_id).unwrap();
    assert!(
        !working.is_empty(),
        "working memory should have turn summary"
    );
    assert!(working.iter().any(|e| e.entry_type == "turn_summary"));

    ironclad_db::memory::store_procedural(&db, "deploy", "run deploy").ok();
    ironclad_agent::memory::ingest_turn(
        &db,
        &session_id,
        "Run the deploy tool",
        "Deployment completed.",
        &[("deploy".into(), "ok".into())],
    );
    let episodic = ironclad_db::memory::retrieve_episodic(&db, 20).unwrap();
    assert!(
        episodic.iter().any(|e| e.classification == "tool_use"),
        "episodic should have tool_use event"
    );

    let procedural = ironclad_db::memory::retrieve_procedural(&db, "deploy").unwrap();
    assert!(procedural.is_some(), "procedural should record deploy");
}

#[test]
fn memory_retrieval_excludes_turn_summary_echoes() {
    let db = Database::new(":memory:").unwrap();
    let session_id = ironclad_db::sessions::find_or_create(&db, "echo-guard-agent", None).unwrap();

    ironclad_db::memory::store_working(
        &db,
        &session_id,
        "turn_summary",
        "Good to be back on familiar ground.",
        9,
    )
    .unwrap();
    ironclad_db::memory::store_working(&db, &session_id, "goal", "stabilize telegram replies", 8)
        .unwrap();

    let retriever = MemoryRetriever::new(MemoryConfig::default());
    let memories = retriever.retrieve(&db, &session_id, "telegram", None, ComplexityLevel::L2);

    assert!(memories.contains("stabilize telegram replies"));
    assert!(
        !memories.contains("Good to be back on familiar ground."),
        "turn summaries should not be re-injected into prompt context"
    );
}

#[test]
fn scoped_sessions_remain_isolated_between_peer_and_group() {
    let db = Database::new(":memory:").unwrap();
    let peer_scope = ironclad_db::sessions::SessionScope::Peer {
        peer_id: "user-1".into(),
        channel: "telegram".into(),
    };
    let group_scope = ironclad_db::sessions::SessionScope::Group {
        group_id: "chat-77".into(),
        channel: "telegram".into(),
    };

    let peer_session = ironclad_db::sessions::find_or_create(&db, "scope-agent", Some(&peer_scope)).unwrap();
    let group_session =
        ironclad_db::sessions::find_or_create(&db, "scope-agent", Some(&group_scope)).unwrap();
    assert_ne!(peer_session, group_session);

    ironclad_db::memory::store_working(&db, &peer_session, "note", "peer-memory", 5).unwrap();
    ironclad_db::memory::store_working(&db, &group_session, "note", "group-memory", 5).unwrap();

    let peer_entries = ironclad_db::memory::retrieve_working(&db, &peer_session).unwrap();
    let group_entries = ironclad_db::memory::retrieve_working(&db, &group_session).unwrap();
    assert!(peer_entries.iter().any(|e| e.content == "peer-memory"));
    assert!(!peer_entries.iter().any(|e| e.content == "group-memory"));
    assert!(group_entries.iter().any(|e| e.content == "group-memory"));
    assert!(!group_entries.iter().any(|e| e.content == "peer-memory"));
}
