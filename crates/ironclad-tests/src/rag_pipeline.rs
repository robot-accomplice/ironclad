use ironclad_agent::context::{ComplexityLevel, build_context};
use ironclad_agent::retrieval::{ChunkConfig, MemoryRetriever, chunk_text};
use ironclad_core::config::MemoryConfig;
use ironclad_db::Database;
use ironclad_db::embeddings;
use ironclad_llm::embedding::fallback_ngram;

fn test_db() -> Database {
    Database::new(":memory:").unwrap()
}

/// Full end-to-end: store memories, generate embeddings (n-gram fallback),
/// retrieve via hybrid search, verify context assembly.
#[test]
fn e2e_store_embed_retrieve_context() {
    let db = test_db();
    let session_id = ironclad_db::sessions::find_or_create(&db, "rag-test", None).unwrap();

    // 1. Store some working memory
    ironclad_db::memory::store_working(&db, &session_id, "goal", "analyze Rust code", 8).unwrap();

    // 2. Store semantic memory with FTS
    ironclad_db::memory::store_semantic(&db, "facts", "rust", "Rust is memory-safe", 0.9).unwrap();
    ironclad_db::memory::store_semantic(&db, "facts", "python", "Python is interpreted", 0.8)
        .unwrap();

    // 3. Generate and store embeddings using n-gram fallback
    let rust_emb = fallback_ngram("Rust is memory-safe", 128);
    let python_emb = fallback_ngram("Python is interpreted", 128);
    let query_emb = fallback_ngram("Tell me about Rust", 128);

    embeddings::store_embedding(
        &db,
        "emb-rust",
        "semantic",
        "rust",
        "Rust is memory-safe",
        &rust_emb,
    )
    .unwrap();
    embeddings::store_embedding(
        &db,
        "emb-python",
        "semantic",
        "python",
        "Python is interpreted",
        &python_emb,
    )
    .unwrap();

    // 4. Verify vector search finds correct results
    let results = embeddings::search_similar(&db, &query_emb, 10, 0.0).unwrap();
    assert!(!results.is_empty(), "vector search should return results");

    // The "Rust" embedding should be most similar to "Tell me about Rust"
    let rust_sim = embeddings::cosine_similarity(&query_emb, &rust_emb);
    let python_sim = embeddings::cosine_similarity(&query_emb, &python_emb);
    assert!(
        rust_sim > python_sim,
        "Rust embedding ({rust_sim:.4}) should be more similar than Python ({python_sim:.4})"
    );

    // 5. Use MemoryRetriever to retrieve formatted memories
    let retriever = MemoryRetriever::new(MemoryConfig::default());
    let memories = retriever.retrieve(
        &db,
        &session_id,
        "Tell me about Rust",
        Some(&query_emb),
        ComplexityLevel::L2,
    );
    assert!(
        !memories.is_empty(),
        "retriever should return non-empty memories"
    );
    assert!(
        memories.contains("Active Memory"),
        "memories should have header"
    );

    // 6. Build context with memories and history
    let history = vec![
        ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: "Hello".into(),
            parts: None,
        },
        ironclad_llm::format::UnifiedMessage {
            role: "assistant".into(),
            content: "Hi there! How can I help?".into(),
            parts: None,
        },
    ];

    let messages = build_context(
        ComplexityLevel::L2,
        "You are a test assistant.",
        &memories,
        &history,
    );
    assert!(
        messages.len() >= 2,
        "context should have at least system + history"
    );
    assert_eq!(messages[0].role, "system");
    assert!(
        messages[0].content.contains("You are a test assistant"),
        "system prompt should be present"
    );
}

/// Verify ingest_turn generates content that can later be retrieved.
#[test]
fn ingest_turn_produces_retrievable_memories() {
    let db = test_db();
    let session_id = ironclad_db::sessions::find_or_create(&db, "ingest-rag", None).unwrap();

    // Ingest a conversation turn
    ironclad_agent::memory::ingest_turn(
        &db,
        &session_id,
        "What is the capital of France?",
        "The capital of France is Paris.",
        &[],
    );

    // Generate and store an embedding for the response
    let emb = fallback_ngram("The capital of France is Paris.", 128);
    embeddings::store_embedding(
        &db,
        "turn-emb-1",
        "turn",
        &session_id,
        "The capital of France is Pari",
        &emb,
    )
    .unwrap();

    // Retrieve using a related query
    let query_emb = fallback_ngram("What about Paris?", 128);
    let retriever = MemoryRetriever::new(MemoryConfig::default());
    let _memories = retriever.retrieve(
        &db,
        &session_id,
        "Paris",
        Some(&query_emb),
        ComplexityLevel::L1,
    );

    // At minimum, working memory from ingest_turn should be present
    let working = ironclad_db::memory::retrieve_working(&db, &session_id).unwrap();
    assert!(
        !working.is_empty(),
        "ingest_turn should have stored working memory"
    );
}

/// Binary embedding storage roundtrip through the full stack.
#[test]
fn binary_embedding_storage_roundtrip() {
    let db = test_db();

    let original = vec![0.1f32, -0.5, 1.23, 0.0, f32::MAX, f32::MIN_POSITIVE];
    embeddings::store_embedding(
        &db,
        "bin-test",
        "episodic_memory",
        "source",
        "preview",
        &original,
    )
    .unwrap();

    // Search should find it and the similarity should be 1.0
    let results = embeddings::search_similar(&db, &original, 10, 0.99).unwrap();
    assert_eq!(results.len(), 1);
    assert!((results[0].similarity - 1.0).abs() < 1e-6);

    // Verify blob serialization at the low level
    let blob = embeddings::embedding_to_blob(&original);
    let restored = embeddings::blob_to_embedding(&blob);
    assert_eq!(original, restored, "blob roundtrip should be lossless");
}

/// Verify build_context respects token budgets.
#[test]
fn build_context_respects_token_budget() {
    let system = "You are a helpful assistant.";
    let memories = "[Active Memory]\n[Working Memory]\n- Important fact";
    // Each message ~200 chars = ~50 tokens. 100 messages = ~5000 tokens, exceeding L0's 2000 budget
    let history: Vec<ironclad_llm::format::UnifiedMessage> = (0..100)
        .map(|i| ironclad_llm::format::UnifiedMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
            content: format!("Message number {i}: {}", "x".repeat(200)),
            parts: None,
        })
        .collect();

    let messages = build_context(ComplexityLevel::L0, system, memories, &history);
    // L0 has 2000 token budget; should truncate history significantly
    assert!(
        messages.len() < history.len(),
        "L0 should truncate long history, got {} messages from {}",
        messages.len(),
        history.len()
    );
    assert_eq!(messages[0].role, "system");
}

/// Chunker integration: chunk a document, embed each chunk, search for relevant chunks.
#[test]
fn chunk_embed_and_search() {
    let db = test_db();

    let document = "Rust is a systems programming language. It provides memory safety without garbage collection. \
                    The borrow checker prevents data races at compile time. Cargo is the build system and package manager. \
                    Crates.io is the community registry for Rust libraries. Async/await enables efficient concurrent programming.";

    let config = ChunkConfig {
        max_tokens: 20,
        overlap_tokens: 5,
    };

    let chunks = chunk_text(document, &config);
    assert!(
        chunks.len() >= 2,
        "long document should produce multiple chunks"
    );

    // Embed each chunk and store
    for chunk in &chunks {
        let emb = fallback_ngram(&chunk.text, 128);
        embeddings::store_embedding(
            &db,
            &format!("chunk-{}", chunk.index),
            "document",
            "doc-1",
            &chunk.text[..chunk.text.len().min(200)],
            &emb,
        )
        .unwrap();
    }

    // Search for a specific topic
    let query_emb = fallback_ngram("borrow checker memory safety", 128);
    let results = embeddings::search_similar(&db, &query_emb, 3, 0.0).unwrap();
    assert!(!results.is_empty(), "should find chunks matching query");

    // The most relevant chunk should mention borrow checker or memory safety
    let top = &results[0];
    let lower = top.content_preview.to_lowercase();
    assert!(
        lower.contains("borrow") || lower.contains("memory") || lower.contains("safety"),
        "top result should be relevant: {lower}"
    );
}

/// Hybrid search combines FTS and vector results.
#[test]
fn hybrid_search_combines_fts_and_vector() {
    let db = test_db();
    let session_id = ironclad_db::sessions::find_or_create(&db, "hybrid-test", None).unwrap();

    // FTS content
    ironclad_db::memory::store_working(
        &db,
        &session_id,
        "note",
        "quantum computing is the future",
        5,
    )
    .unwrap();

    // Vector content
    let emb = fallback_ngram("quantum computing applications", 128);
    embeddings::store_embedding(
        &db,
        "q-emb",
        "working",
        &session_id,
        "quantum computing is the future",
        &emb,
    )
    .unwrap();

    let query_emb = fallback_ngram("quantum computing", 128);
    let results = embeddings::hybrid_search(&db, "quantum", Some(&query_emb), 10, 0.5).unwrap();

    // Should have results from both FTS and vector paths
    assert!(!results.is_empty(), "hybrid search should return results");
}

/// ANN index build + search (with low threshold).
#[test]
fn ann_index_build_and_search() {
    let db = test_db();
    let mut ann = ironclad_db::ann::AnnIndex::new(true);
    ann.min_entries_for_index = 5; // lower threshold for testing

    for i in 0..10 {
        let angle = i as f32 * std::f32::consts::PI / 10.0;
        let emb = vec![angle.cos(), angle.sin(), 0.0];
        embeddings::store_embedding(
            &db,
            &format!("ann-{i}"),
            "test",
            &format!("t{i}"),
            &format!("entry at angle {}", i * 18),
            &emb,
        )
        .unwrap();
    }

    let count = ann.build_from_db(&db).unwrap();
    assert_eq!(count, 10);
    assert!(ann.is_built());

    let query = vec![1.0, 0.0, 0.0]; // cos(0), sin(0) -- should match entry 0
    let results = ann.search(&query, 3).unwrap();
    assert_eq!(results.len(), 3);
    assert!(
        results[0].similarity > results[2].similarity,
        "results should be sorted by similarity"
    );
}

/// Cache persistence roundtrip through DB.
#[test]
fn cache_persistence_roundtrip() {
    let db = test_db();

    let entry = ironclad_db::cache::PersistedCacheEntry {
        prompt_hash: "test-hash".into(),
        response: "cached response".into(),
        model: "test-model".into(),
        tokens_saved: 42,
        hit_count: 7,
        embedding: Some(vec![0.1, 0.2, 0.3]),
        created_at: "2025-06-15T12:00:00".into(),
        expires_at: Some("2030-12-31T23:59:59".into()),
    };

    ironclad_db::cache::save_cache_entry(&db, "c1", &entry).unwrap();

    let loaded = ironclad_db::cache::load_cache_entries(&db).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].1.response, "cached response");
    assert_eq!(loaded[0].1.tokens_saved, 42);
    assert_eq!(loaded[0].1.hit_count, 7);

    let emb = loaded[0].1.embedding.as_ref().unwrap();
    assert_eq!(emb.len(), 3);
    assert!((emb[0] - 0.1).abs() < 1e-6);
}
