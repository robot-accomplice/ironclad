use ironclad_core::config::MemoryConfig;
use ironclad_db::Database;

use crate::context::{ComplexityLevel, token_budget};
use crate::memory::MemoryBudgetManager;

/// Retrieves and formats memories from all five tiers for injection into the LLM prompt.
pub struct MemoryRetriever {
    budget_manager: MemoryBudgetManager,
    hybrid_weight: f64,
}

impl MemoryRetriever {
    pub fn new(config: MemoryConfig) -> Self {
        let hybrid_weight = config.hybrid_weight;
        Self {
            budget_manager: MemoryBudgetManager::new(config),
            hybrid_weight,
        }
    }

    /// Retrieve memories from all tiers and format them into a single string
    /// for context injection. Token budgets are respected per-tier.
    pub fn retrieve(
        &self,
        db: &Database,
        session_id: &str,
        query: &str,
        query_embedding: Option<&[f32]>,
        complexity: ComplexityLevel,
    ) -> String {
        self.retrieve_with_ann(db, session_id, query, query_embedding, complexity, None)
    }

    /// Like `retrieve`, but optionally uses an ANN index for O(log n) nearest-neighbor
    /// search instead of brute-force cosine scan.
    pub fn retrieve_with_ann(
        &self,
        db: &Database,
        session_id: &str,
        query: &str,
        query_embedding: Option<&[f32]>,
        complexity: ComplexityLevel,
        ann_index: Option<&ironclad_db::ann::AnnIndex>,
    ) -> String {
        let total_budget = token_budget(complexity);
        let budgets = self.budget_manager.allocate_budgets(total_budget);

        let mut sections = Vec::new();

        if let Some(s) = self.retrieve_working(db, session_id, budgets.working) {
            sections.push(s);
        }

        // Try ANN index first for relevant memories; fall back to brute-force hybrid search
        let relevant = if let (Some(ann), Some(emb)) = (ann_index, query_embedding) {
            ann.search(emb, 10).map(|results| {
                results
                    .into_iter()
                    .map(|r| ironclad_db::embeddings::SearchResult {
                        source_table: r.source_table,
                        source_id: r.source_id,
                        content_preview: r.content_preview,
                        similarity: r.similarity,
                    })
                    .collect::<Vec<_>>()
            })
        } else {
            None
        };
        let relevant = relevant.unwrap_or_else(|| {
            ironclad_db::embeddings::hybrid_search(
                db,
                query,
                query_embedding,
                10,
                self.hybrid_weight,
            )
            .unwrap_or_default()
        });
        if let Some(s) = self.format_relevant(&relevant, budgets.episodic + budgets.semantic) {
            sections.push(s);
        }

        if let Some(s) = self.retrieve_procedural(db, budgets.procedural) {
            sections.push(s);
        }

        if let Some(s) = self.retrieve_relationships(db, query, budgets.relationship) {
            sections.push(s);
        }

        if sections.is_empty() {
            return String::new();
        }

        format!("[Active Memory]\n{}", sections.join("\n\n"))
    }

    fn retrieve_working(
        &self,
        db: &Database,
        session_id: &str,
        budget_tokens: usize,
    ) -> Option<String> {
        if budget_tokens == 0 {
            return None;
        }

        let entries = ironclad_db::memory::retrieve_working(db, session_id).ok()?;
        if entries.is_empty() {
            return None;
        }

        let mut text = String::from("[Working Memory]\n");
        let mut used = estimate_tokens(&text);

        for entry in &entries {
            // `turn_summary` mirrors prior assistant output and can cause
            // repetitive self-priming when injected into subsequent prompts.
            if entry.entry_type.eq_ignore_ascii_case("turn_summary") {
                continue;
            }
            let line = format!("- [{}] {}\n", entry.entry_type, entry.content);
            let line_tokens = estimate_tokens(&line);
            if used + line_tokens > budget_tokens {
                break;
            }
            text.push_str(&line);
            used += line_tokens;
        }

        if text.len() > "[Working Memory]\n".len() {
            Some(text)
        } else {
            None
        }
    }

    fn format_relevant(
        &self,
        results: &[ironclad_db::embeddings::SearchResult],
        budget_tokens: usize,
    ) -> Option<String> {
        if budget_tokens == 0 || results.is_empty() {
            return None;
        }

        let mut text = String::from("[Relevant Memories]\n");
        let mut used = estimate_tokens(&text);

        for result in results {
            let line = format!(
                "- [{} | sim={:.2}] {}\n",
                result.source_table, result.similarity, result.content_preview,
            );
            let line_tokens = estimate_tokens(&line);
            if used + line_tokens > budget_tokens {
                break;
            }
            text.push_str(&line);
            used += line_tokens;
        }

        if text.len() > "[Relevant Memories]\n".len() {
            Some(text)
        } else {
            None
        }
    }

    fn retrieve_procedural(&self, db: &Database, budget_tokens: usize) -> Option<String> {
        if budget_tokens == 0 {
            return None;
        }

        // Retrieve all procedural entries and present those with meaningful history
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT name, steps, success_count, failure_count FROM procedural_memory \
                 WHERE success_count > 0 OR failure_count > 0 \
                 ORDER BY success_count + failure_count DESC LIMIT 5",
            )
            .ok()?;

        let rows: Vec<(String, String, i64, i64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            return None;
        }

        let mut text = String::from("[Tool Experience]\n");
        let mut used = estimate_tokens(&text);

        for (name, _steps, successes, failures) in &rows {
            let total = *successes + *failures;
            let rate = if total > 0 {
                (*successes as f64 / total as f64 * 100.0) as u32
            } else {
                0
            };
            let line = format!("- {name}: {successes}/{total} success ({rate}%)\n");
            let line_tokens = estimate_tokens(&line);
            if used + line_tokens > budget_tokens {
                break;
            }
            text.push_str(&line);
            used += line_tokens;
        }

        if text.len() > "[Tool Experience]\n".len() {
            Some(text)
        } else {
            None
        }
    }

    fn retrieve_relationships(
        &self,
        db: &Database,
        query: &str,
        budget_tokens: usize,
    ) -> Option<String> {
        if budget_tokens == 0 {
            return None;
        }

        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT entity_id, entity_name, trust_score, interaction_count \
                 FROM relationship_memory ORDER BY interaction_count DESC LIMIT 5",
            )
            .ok()?;

        let rows: Vec<(String, Option<String>, f64, i64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            return None;
        }

        // Only include entities that might be relevant: name appears in query, or high interaction count
        let query_lower = query.to_lowercase();
        let relevant: Vec<_> = rows
            .into_iter()
            .filter(|(id, name, _, count)| {
                *count > 2
                    || query_lower.contains(&id.to_lowercase())
                    || name
                        .as_ref()
                        .is_some_and(|n| query_lower.contains(&n.to_lowercase()))
            })
            .collect();

        if relevant.is_empty() {
            return None;
        }

        let mut text = String::from("[Known Entities]\n");
        let mut used = estimate_tokens(&text);

        for (entity_id, name, trust, count) in &relevant {
            let display = name.as_deref().unwrap_or(entity_id);
            let line = format!("- {display}: trust={trust:.1}, interactions={count}\n");
            let line_tokens = estimate_tokens(&line);
            if used + line_tokens > budget_tokens {
                break;
            }
            text.push_str(&line);
            used += line_tokens;
        }

        if text.len() > "[Known Entities]\n".len() {
            Some(text)
        } else {
            None
        }
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

// ── Content chunking ────────────────────────────────────────────

pub struct ChunkConfig {
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            overlap_tokens: 64,
        }
    }
}

pub struct Chunk {
    pub text: String,
    pub index: usize,
    pub start_char: usize,
    pub end_char: usize,
}

/// Snap a byte offset to the nearest char boundary at or before `pos`.
fn floor_char_boundary(text: &str, pos: usize) -> usize {
    if pos >= text.len() {
        return text.len();
    }
    let mut p = pos;
    while p > 0 && !text.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Split text into overlapping chunks for embedding.
pub fn chunk_text(text: &str, config: &ChunkConfig) -> Vec<Chunk> {
    if text.is_empty() || config.max_tokens == 0 {
        return Vec::new();
    }

    let max_bytes = config.max_tokens * 4;
    let overlap_bytes = config.overlap_tokens * 4;

    if text.len() <= max_bytes {
        return vec![Chunk {
            text: text.to_string(),
            index: 0,
            start_char: 0,
            end_char: text.len(),
        }];
    }

    let step = max_bytes.saturating_sub(overlap_bytes).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let raw_end = floor_char_boundary(text, (start + max_bytes).min(text.len()));

        let end = find_break_point(text, start, raw_end);

        chunks.push(Chunk {
            text: text[start..end].to_string(),
            index: chunks.len(),
            start_char: start,
            end_char: end,
        });

        if end >= text.len() {
            break;
        }

        let advance = step.min(end - start).max(1);
        start = floor_char_boundary(text, start + advance);
    }

    chunks
}

fn find_break_point(text: &str, start: usize, raw_end: usize) -> usize {
    if raw_end >= text.len() {
        return text.len();
    }

    let search_start = floor_char_boundary(text, start + (raw_end - start) / 2);
    let window = &text[search_start..raw_end];

    if let Some(pos) = window.rfind("\n\n") {
        return search_start + pos + 2;
    }
    for delim in [". ", ".\n", "? ", "! "] {
        if let Some(pos) = window.rfind(delim) {
            return search_start + pos + delim.len();
        }
    }
    if let Some(pos) = window.rfind(' ') {
        return search_start + pos + 1;
    }

    raw_end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    fn default_config() -> MemoryConfig {
        MemoryConfig::default()
    }

    #[test]
    fn retriever_empty_db_returns_empty() {
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        let result = retriever.retrieve(&db, &session_id, "hello", None, ComplexityLevel::L1);
        assert!(result.is_empty());
    }

    #[test]
    fn retriever_returns_working_memory() {
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        ironclad_db::memory::store_working(&db, &session_id, "goal", "find documentation", 8)
            .unwrap();

        let result = retriever.retrieve(&db, &session_id, "hello", None, ComplexityLevel::L2);
        assert!(result.contains("Working Memory"));
        assert!(result.contains("find documentation"));
    }

    #[test]
    fn retriever_skips_turn_summary_working_entries() {
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        ironclad_db::memory::store_working(
            &db,
            &session_id,
            "turn_summary",
            "Good to be back on familiar ground.",
            9,
        )
        .unwrap();
        ironclad_db::memory::store_working(&db, &session_id, "goal", "fix Telegram loop", 8)
            .unwrap();

        let result = retriever.retrieve(&db, &session_id, "telegram", None, ComplexityLevel::L2);
        assert!(result.contains("Working Memory"));
        assert!(result.contains("fix Telegram loop"));
        assert!(!result.contains("Good to be back on familiar ground."));
    }

    #[test]
    fn retriever_returns_relevant_memories() {
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        ironclad_db::memory::store_semantic(&db, "facts", "sky", "the sky is blue", 0.9).unwrap();

        let result = retriever.retrieve(&db, &session_id, "sky", None, ComplexityLevel::L2);
        assert!(result.contains("Active Memory"));
    }

    #[test]
    fn retriever_returns_procedural_experience() {
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        ironclad_db::memory::store_procedural(&db, "web_search", "search the web").unwrap();
        ironclad_db::memory::record_procedural_success(&db, "web_search").unwrap();
        ironclad_db::memory::record_procedural_success(&db, "web_search").unwrap();

        let result = retriever.retrieve(&db, &session_id, "search", None, ComplexityLevel::L2);
        assert!(result.contains("Tool Experience"));
        assert!(result.contains("web_search"));
    }

    #[test]
    fn retriever_returns_relationships() {
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        ironclad_db::memory::store_relationship(&db, "user-1", "Jon", 0.9).unwrap();
        // Need > 2 interactions or name in query
        let result = retriever.retrieve(&db, &session_id, "Jon", None, ComplexityLevel::L2);
        assert!(result.contains("Known Entities") || result.contains("Jon"));
    }

    #[test]
    fn retriever_respects_zero_budget() {
        let config = MemoryConfig {
            working_budget_pct: 0.0,
            episodic_budget_pct: 0.0,
            semantic_budget_pct: 0.0,
            procedural_budget_pct: 0.0,
            relationship_budget_pct: 100.0,
            ..default_config()
        };
        let db = test_db();
        let retriever = MemoryRetriever::new(config);
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        ironclad_db::memory::store_working(&db, &session_id, "goal", "test", 5).unwrap();

        let result = retriever.retrieve(&db, &session_id, "test", None, ComplexityLevel::L0);
        assert!(!result.contains("Working Memory"));
    }

    // ── Chunker tests ───────────────────────────────────────────

    #[test]
    fn chunk_empty_text() {
        let chunks = chunk_text("", &ChunkConfig::default());
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_short_text() {
        let text = "This is a short sentence.";
        let chunks = chunk_text(text, &ChunkConfig::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, text);
        assert_eq!(chunks[0].index, 0);
    }

    #[test]
    fn chunk_long_text_produces_overlapping_chunks() {
        let text = "word ".repeat(1000);
        let config = ChunkConfig {
            max_tokens: 50,
            overlap_tokens: 10,
        };
        let chunks = chunk_text(&text, &config);
        assert!(chunks.len() > 1);

        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.index, i);
            assert!(!chunk.text.is_empty());
        }

        // Verify continuity: each chunk's start is before the previous chunk's end
        for i in 1..chunks.len() {
            assert!(chunks[i].start_char < chunks[i - 1].end_char);
        }
    }

    #[test]
    fn chunk_respects_sentence_boundaries() {
        let text = "First sentence. Second sentence. Third sentence. Fourth sentence. Fifth sentence. \
                    Sixth sentence. Seventh sentence. Eighth sentence. Ninth sentence. Tenth sentence.";
        let config = ChunkConfig {
            max_tokens: 20,
            overlap_tokens: 5,
        };
        let chunks = chunk_text(text, &config);
        // Chunks should end at sentence boundaries when possible
        for chunk in &chunks {
            if chunk.end_char < text.len() {
                let ends_at_boundary = chunk.text.ends_with(". ")
                    || chunk.text.ends_with('.')
                    || chunk.text.ends_with(' ');
                assert!(
                    ends_at_boundary,
                    "chunk should end at a boundary: {:?}",
                    &chunk.text[chunk.text.len().saturating_sub(10)..]
                );
            }
        }
    }

    #[test]
    fn chunk_covers_full_text() {
        let text = "a ".repeat(500);
        let config = ChunkConfig {
            max_tokens: 25,
            overlap_tokens: 5,
        };
        let chunks = chunk_text(&text, &config);

        assert_eq!(chunks.first().unwrap().start_char, 0);
        assert_eq!(chunks.last().unwrap().end_char, text.len());
    }

    #[test]
    fn chunk_zero_max_tokens() {
        let chunks = chunk_text(
            "some text",
            &ChunkConfig {
                max_tokens: 0,
                overlap_tokens: 0,
            },
        );
        assert!(chunks.is_empty());
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("hello world!"), 3);
    }

    #[test]
    fn chunk_multibyte_does_not_panic() {
        let text = "Hello \u{1F600} world. ".repeat(200);
        let config = ChunkConfig {
            max_tokens: 20,
            overlap_tokens: 5,
        };
        let chunks = chunk_text(&text, &config);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(!chunk.text.is_empty());
            // Verify each chunk is valid UTF-8 (would panic on slice if not)
            let _ = chunk.text.as_bytes();
        }
    }

    #[test]
    fn chunk_cjk_text() {
        let text = "\u{4F60}\u{597D}\u{4E16}\u{754C} ".repeat(300);
        let config = ChunkConfig {
            max_tokens: 15,
            overlap_tokens: 3,
        };
        let chunks = chunk_text(&text, &config);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.first().unwrap().start_char, 0);
        assert_eq!(chunks.last().unwrap().end_char, text.len());
    }

    #[test]
    fn floor_char_boundary_ascii() {
        let text = "hello world";
        assert_eq!(floor_char_boundary(text, 5), 5);
        assert_eq!(floor_char_boundary(text, 0), 0);
        assert_eq!(floor_char_boundary(text, 100), text.len());
    }

    #[test]
    fn floor_char_boundary_multibyte() {
        // "café" = c(1) a(1) f(1) é(2) = 5 bytes total
        let text = "caf\u{00E9}";
        assert_eq!(text.len(), 5);
        // Position 4 is inside the 2-byte é, should snap back to 3
        assert_eq!(floor_char_boundary(text, 4), 3);
        // Position 3 is a valid boundary (start of é)
        assert_eq!(floor_char_boundary(text, 3), 3);
        // Position 5 >= len, returns len
        assert_eq!(floor_char_boundary(text, 5), 5);
    }

    #[test]
    fn floor_char_boundary_emoji() {
        let text = "a\u{1F600}b"; // a(1) + emoji(4) + b(1) = 6 bytes
        assert_eq!(text.len(), 6);
        // Position 2 is inside the emoji
        assert_eq!(floor_char_boundary(text, 2), 1);
        // Position 5 is the start of 'b'
        assert_eq!(floor_char_boundary(text, 5), 5);
    }

    #[test]
    fn estimate_tokens_rounding() {
        // div_ceil(1, 4) = 1
        assert_eq!(estimate_tokens("a"), 1);
        // div_ceil(5, 4) = 2
        assert_eq!(estimate_tokens("abcde"), 2);
        // div_ceil(8, 4) = 2
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn retriever_with_procedural_no_history() {
        // Procedural with no success/failure counts should return None
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        ironclad_db::memory::store_procedural(&db, "unused_tool", "a tool").unwrap();

        let result = retriever.retrieve(&db, &session_id, "test", None, ComplexityLevel::L2);
        assert!(
            !result.contains("Tool Experience"),
            "tools with no success/failure should not appear"
        );
    }

    #[test]
    fn chunk_with_paragraph_breaks() {
        let text = "Paragraph one content.\n\nParagraph two content.\n\nParagraph three content.\n\n\
                    Paragraph four content.\n\nParagraph five content.";
        let config = ChunkConfig {
            max_tokens: 15,
            overlap_tokens: 3,
        };
        let chunks = chunk_text(text, &config);
        // Should prefer breaking at paragraph boundaries
        for chunk in &chunks {
            if chunk.end_char < text.len() {
                // Many chunks should end at paragraph breaks
                let last_few = &chunk.text[chunk.text.len().saturating_sub(5)..];
                let has_good_break =
                    last_few.contains('\n') || last_few.contains(". ") || last_few.ends_with(' ');
                assert!(has_good_break, "chunk should end at a reasonable boundary");
            }
        }
    }

    #[test]
    fn chunk_config_default() {
        let config = ChunkConfig::default();
        assert_eq!(config.max_tokens, 512);
        assert_eq!(config.overlap_tokens, 64);
    }

    #[test]
    fn find_break_point_at_end_of_text() {
        let text = "Hello world.";
        assert_eq!(find_break_point(text, 0, text.len()), text.len());
    }

    #[test]
    fn retriever_relationships_high_interaction_count() {
        let db = test_db();
        let retriever = MemoryRetriever::new(default_config());
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();

        // store_relationship uses ON CONFLICT to increment interaction_count
        // Calling it 4 times gives interaction_count > 2
        for _ in 0..4 {
            ironclad_db::memory::store_relationship(&db, "alice", "Alice Smith", 0.8).unwrap();
        }

        // Query that doesn't contain "alice" but high interaction count should still include it
        let result = retriever.retrieve(
            &db,
            &session_id,
            "some random query",
            None,
            ComplexityLevel::L2,
        );
        assert!(
            result.contains("Known Entities") && result.contains("Alice Smith"),
            "high interaction count entity should appear in results"
        );
    }
}
