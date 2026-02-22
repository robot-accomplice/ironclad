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
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

pub fn store_embedding(
    db: &Database,
    id: &str,
    source_table: &str,
    source_id: &str,
    content_preview: &str,
    embedding: &[f32],
) -> Result<()> {
    let embedding_json = serde_json::to_string(embedding)
        .map_err(|e| IroncladError::Database(format!("embedding serialize error: {e}")))?;

    let conn = db.conn();
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (id, source_table, source_id, content_preview, embedding_json) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, source_table, source_id, content_preview, embedding_json],
    ).map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(())
}

pub fn search_similar(
    db: &Database,
    query_embedding: &[f32],
    limit: usize,
    min_similarity: f64,
) -> Result<Vec<SearchResult>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT source_table, source_id, content_preview, embedding_json FROM embeddings"
    ).map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    }).map_err(|e| IroncladError::Database(e.to_string()))?;

    let mut results: Vec<SearchResult> = Vec::new();

    for row in rows {
        let (source_table, source_id, content_preview, embedding_json) = row
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        let embedding: Vec<f32> = serde_json::from_str(&embedding_json)
            .map_err(|e| IroncladError::Database(format!("embedding parse error: {e}")))?;

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

    results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
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
        let mut stmt = conn.prepare(
            "SELECT content, category FROM memory_fts WHERE memory_fts MATCH ?1 LIMIT ?2"
        ).map_err(|e| IroncladError::Database(e.to_string()))?;

        let rows = stmt.query_map(rusqlite::params![safe_query, limit * 2], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
            ))
        }).map_err(|e| IroncladError::Database(e.to_string()))?;

        for (i, row) in rows.enumerate() {
            let (content, category) = row
                .map_err(|e| IroncladError::Database(e.to_string()))?;
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

    fts_results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
    fts_results.truncate(limit);

    Ok(fts_results)
}

pub fn embedding_count(db: &Database) -> Result<usize> {
    let conn = db.conn();
    let count: usize = conn.query_row(
        "SELECT COUNT(*) FROM embeddings",
        [],
        |row| row.get(0),
    ).map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
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
}
