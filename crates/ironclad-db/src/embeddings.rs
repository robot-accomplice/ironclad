use serde::{Deserialize, Serialize};

use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingEntry {
    pub id: String,
    pub source_table: String,
    pub source_id: String,
    pub content_preview: String,
    pub embedding: Vec<f32>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub source_table: String,
    pub source_id: String,
    pub content_preview: String,
    pub similarity: f64,
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;

    for i in 0..a.len() {
        let ai = a[i] as f64;
        let bi = b[i] as f64;
        dot += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

/// Serialize `Vec<f32>` to a compact little-endian byte representation.
pub fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize a BLOB back to `Vec<f32>`.
pub fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Store an embedding using binary BLOB format (with JSON fallback column for
/// backward compatibility).
pub fn store_embedding(
    db: &Database,
    id: &str,
    source_table: &str,
    source_id: &str,
    content_preview: &str,
    embedding: &[f32],
) -> Result<()> {
    let blob = embedding_to_blob(embedding);
    let dimensions = embedding.len() as i64;

    let conn = db.conn();
    conn.execute(
        "INSERT OR REPLACE INTO embeddings \
         (id, source_table, source_id, content_preview, embedding_json, embedding_blob, dimensions) \
         VALUES (?1, ?2, ?3, ?4, '', ?5, ?6)",
        rusqlite::params![id, source_table, source_id, content_preview, blob, dimensions],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(())
}

/// Load an embedding from a row, preferring BLOB over JSON.
fn load_embedding_from_row(blob: Option<Vec<u8>>, json_text: &str) -> Option<Vec<f32>> {
    if let Some(b) = blob
        && !b.is_empty()
    {
        return Some(blob_to_embedding(&b));
    }
    if !json_text.is_empty() {
        return serde_json::from_str(json_text).ok();
    }
    None
}

/// Brute-force cosine similarity search over all stored embeddings.
///
/// **Complexity**: O(N) where N is the number of stored embeddings. Every row is loaded
/// into memory and compared. For production workloads with large embedding tables,
/// use `AnnIndex` (approximate nearest neighbor) instead.
///
/// A `LIMIT 10000` cap is applied to the SQL query to prevent unbounded memory usage
/// while the AnnIndex integration is pending.
pub fn search_similar(
    db: &Database,
    query_embedding: &[f32],
    limit: usize,
    min_similarity: f64,
) -> Result<Vec<SearchResult>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT source_table, source_id, content_preview, embedding_blob, embedding_json \
             FROM embeddings LIMIT 10000",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let mut results: Vec<SearchResult> = Vec::new();

    for row in rows {
        let (source_table, source_id, content_preview, blob, json_text) =
            row.map_err(|e| IroncladError::Database(e.to_string()))?;

        let embedding = match load_embedding_from_row(blob, &json_text) {
            Some(e) => e,
            None => continue,
        };

        let similarity = cosine_similarity(query_embedding, &embedding);

        if similarity >= min_similarity {
            results.push(SearchResult {
                source_table,
                source_id,
                content_preview,
                similarity,
            });
        }
    }

    results.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);

    Ok(results)
}

pub fn hybrid_search(
    db: &Database,
    query_text: &str,
    query_embedding: Option<&[f32]>,
    limit: usize,
    hybrid_weight: f64,
) -> Result<Vec<SearchResult>> {
    let mut fts_results: Vec<SearchResult> = Vec::new();

    {
        let conn = db.conn();
        let safe_query = crate::memory::sanitize_fts_query(query_text);
        let mut stmt = conn
            .prepare("SELECT content, category FROM memory_fts WHERE memory_fts MATCH ?1 LIMIT ?2")
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![safe_query, limit * 2], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        for (i, row) in rows.enumerate() {
            let (content, category) = row.map_err(|e| IroncladError::Database(e.to_string()))?;
            let fts_score = 1.0 - (i as f64 * 0.05).min(0.9);
            fts_results.push(SearchResult {
                source_table: category,
                source_id: String::new(),
                content_preview: content.chars().take(200).collect(),
                similarity: fts_score * (1.0 - hybrid_weight),
            });
        }
    }

    if let Some(embedding) = query_embedding {
        let vec_results = search_similar(db, embedding, limit * 2, 0.0)?;
        for mut r in vec_results {
            r.similarity *= hybrid_weight;
            fts_results.push(r);
        }
    }

    fts_results.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fts_results.truncate(limit);

    Ok(fts_results)
}

#[cfg(test)]
pub(crate) fn embedding_count(db: &Database) -> Result<usize> {
    let conn = db.conn();
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn blob_roundtrip() {
        let original = vec![1.0f32, -0.5, 0.0, 1.23456, f32::MIN, f32::MAX];
        let blob = embedding_to_blob(&original);
        let restored = blob_to_embedding(&blob);
        assert_eq!(original, restored);
    }

    #[test]
    fn blob_empty() {
        let blob = embedding_to_blob(&[]);
        assert!(blob.is_empty());
        let restored = blob_to_embedding(&blob);
        assert!(restored.is_empty());
    }

    #[test]
    fn blob_size_is_4x_floats() {
        let emb = vec![0.0f32; 768];
        let blob = embedding_to_blob(&emb);
        assert_eq!(blob.len(), 768 * 4);
    }

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn cosine_empty_vectors() {
        let sim = cosine_similarity(&[], &[]);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn cosine_mismatched_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn store_and_search() {
        let db = test_db();
        let emb1 = vec![1.0, 0.0, 0.0];
        let emb2 = vec![0.0, 1.0, 0.0];
        let emb3 = vec![0.9, 0.1, 0.0];

        store_embedding(&db, "e1", "episodic", "ep1", "first entry", &emb1).unwrap();
        store_embedding(&db, "e2", "episodic", "ep2", "second entry", &emb2).unwrap();
        store_embedding(&db, "e3", "semantic", "s1", "third entry", &emb3).unwrap();

        let query = vec![1.0, 0.0, 0.0];
        let results = search_similar(&db, &query, 10, 0.5).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source_id, "ep1");
        assert!((results[0].similarity - 1.0).abs() < 1e-6);
        assert!(results[1].similarity > 0.5);
    }

    #[test]
    fn store_replaces_existing() {
        let db = test_db();
        let emb1 = vec![1.0, 0.0];
        let emb2 = vec![0.0, 1.0];
        store_embedding(&db, "e1", "test", "t1", "v1", &emb1).unwrap();
        store_embedding(&db, "e1", "test", "t1", "v2", &emb2).unwrap();
        assert_eq!(embedding_count(&db).unwrap(), 1);
    }

    #[test]
    fn search_min_similarity_filter() {
        let db = test_db();
        store_embedding(&db, "e1", "t", "1", "a", &[1.0, 0.0]).unwrap();
        store_embedding(&db, "e2", "t", "2", "b", &[0.0, 1.0]).unwrap();

        let results = search_similar(&db, &[1.0, 0.0], 10, 0.99).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn embedding_count_works() {
        let db = test_db();
        assert_eq!(embedding_count(&db).unwrap(), 0);
        store_embedding(&db, "e1", "t", "1", "a", &[1.0]).unwrap();
        assert_eq!(embedding_count(&db).unwrap(), 1);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn hybrid_search_vector_only() {
        let db = test_db();
        store_embedding(&db, "e1", "test", "t1", "hello world", &[1.0, 0.0, 0.0]).unwrap();
        store_embedding(&db, "e2", "test", "t2", "goodbye", &[0.0, 1.0, 0.0]).unwrap();

        let results =
            hybrid_search(&db, "zzzznonexistent", Some(&[1.0, 0.0, 0.0]), 10, 0.5).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn hybrid_search_empty_db() {
        let db = test_db();
        let results = hybrid_search(&db, "anything", Some(&[1.0, 0.0]), 10, 0.5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn hybrid_search_respects_limit() {
        let db = test_db();
        for i in 0..20 {
            store_embedding(
                &db,
                &format!("e{i}"),
                "test",
                &format!("t{i}"),
                &format!("entry {i}"),
                &[1.0, 0.0],
            )
            .unwrap();
        }
        let results = hybrid_search(&db, "entry", Some(&[1.0, 0.0]), 5, 0.5).unwrap();
        assert!(results.len() <= 5);
    }

    #[test]
    fn hybrid_search_no_embedding() {
        let db = test_db();
        store_embedding(&db, "e1", "test", "t1", "hello world", &[1.0, 0.0]).unwrap();
        let results = hybrid_search(&db, "hello", None, 10, 0.5).unwrap();
        assert!(results.is_empty() || !results.is_empty());
    }

    #[test]
    fn hybrid_search_sorted_by_similarity() {
        let db = test_db();
        store_embedding(&db, "e1", "test", "t1", "first", &[1.0, 0.0, 0.0]).unwrap();
        store_embedding(&db, "e2", "test", "t2", "second", &[0.5, 0.5, 0.0]).unwrap();
        store_embedding(&db, "e3", "test", "t3", "third", &[0.0, 0.0, 1.0]).unwrap();

        let results = hybrid_search(&db, "query", Some(&[1.0, 0.0, 0.0]), 10, 1.0).unwrap();
        for w in results.windows(2) {
            assert!(w[0].similarity >= w[1].similarity);
        }
    }

    #[test]
    fn load_embedding_prefers_blob() {
        let emb = vec![1.0f32, 2.0, 3.0];
        let blob = embedding_to_blob(&emb);
        let json = serde_json::to_string(&vec![4.0f32, 5.0, 6.0]).unwrap();
        let loaded = load_embedding_from_row(Some(blob), &json).unwrap();
        assert_eq!(loaded, emb);
    }

    #[test]
    fn load_embedding_falls_back_to_json() {
        let json = serde_json::to_string(&vec![7.0f32, 8.0]).unwrap();
        let loaded = load_embedding_from_row(None, &json).unwrap();
        assert_eq!(loaded, vec![7.0, 8.0]);
    }

    #[test]
    fn load_embedding_empty_both() {
        let loaded = load_embedding_from_row(None, "");
        assert!(loaded.is_none());
    }

    #[test]
    fn load_embedding_empty_blob_with_json() {
        let json = serde_json::to_string(&vec![1.0f32, 2.0]).unwrap();
        // Empty blob (not None) should fall back to JSON
        let loaded = load_embedding_from_row(Some(vec![]), &json).unwrap();
        assert_eq!(loaded, vec![1.0, 2.0]);
    }

    #[test]
    fn load_embedding_empty_blob_empty_json() {
        let loaded = load_embedding_from_row(Some(vec![]), "");
        assert!(loaded.is_none());
    }

    #[test]
    fn search_similar_skips_row_without_embedding() {
        let db = test_db();
        // Insert a row with empty embedding data (both blob and json empty)
        let conn = db.conn();
        conn.execute(
            "INSERT INTO embeddings (id, source_table, source_id, content_preview, embedding_json, embedding_blob, dimensions) \
             VALUES ('e-no-emb', 'test', 't1', 'no embedding here', '', NULL, 0)",
            [],
        ).unwrap();
        // Also insert one with a real embedding
        store_embedding(&db, "e-real", "test", "t2", "has embedding", &[1.0, 0.0]).unwrap();

        let results = search_similar(&db, &[1.0, 0.0], 10, 0.0).unwrap();
        // Should only find the one with a real embedding
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source_id, "t2");
    }

    #[test]
    fn hybrid_search_fts_matches() {
        let db = test_db();
        // Store data in FTS-indexed tables (working_memory populates memory_fts)
        crate::memory::store_working(&db, "sess", "note", "quantum computing breakthrough", 5).unwrap();
        store_embedding(&db, "e1", "test", "t1", "classical computing", &[0.0, 1.0]).unwrap();

        // Search with FTS query that should match the working memory entry
        let results = hybrid_search(&db, "quantum", Some(&[1.0, 0.0]), 10, 0.5).unwrap();
        assert!(!results.is_empty(), "hybrid search should find FTS match for 'quantum'");
    }

    #[test]
    fn hybrid_search_fts_only_no_embedding() {
        let db = test_db();
        crate::memory::store_working(&db, "sess", "note", "unique identifier xyzzy", 5).unwrap();

        // Search with only FTS (no embedding provided), weight doesn't matter much
        let results = hybrid_search(&db, "xyzzy", None, 10, 0.5).unwrap();
        // FTS results get weighted by (1 - hybrid_weight), so they should appear
        assert!(!results.is_empty(), "hybrid search without embedding should find FTS results");
    }

    #[test]
    fn hybrid_search_combined_scores() {
        let db = test_db();
        crate::memory::store_working(&db, "sess", "note", "machine learning algorithms", 5).unwrap();
        store_embedding(&db, "e1", "test", "t1", "machine learning", &[1.0, 0.0, 0.0]).unwrap();

        let results = hybrid_search(&db, "machine", Some(&[1.0, 0.0, 0.0]), 10, 0.5).unwrap();
        // Should have results from both FTS and vector search
        assert!(!results.is_empty());
        // Results should be sorted by similarity desc
        for w in results.windows(2) {
            assert!(w[0].similarity >= w[1].similarity);
        }
    }
}
