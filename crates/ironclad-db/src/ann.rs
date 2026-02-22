use std::sync::{Arc, RwLock};

use instant_distance::{Builder, HnswMap, Search};

use crate::Database;
use crate::embeddings::{blob_to_embedding, cosine_similarity};

/// A point wrapper for the instant-distance HNSW index.
#[derive(Clone)]
struct EmbeddingPoint(Vec<f32>);

impl instant_distance::Point for EmbeddingPoint {
    fn distance(&self, other: &Self) -> f32 {
        1.0 - cosine_sim_f32(&self.0, &other.0)
    }
}

fn cosine_sim_f32(a: &[f32], b: &[f32]) -> f32 {
    cosine_similarity(a, b) as f32
}

/// Metadata stored alongside each indexed embedding.
#[derive(Clone)]
struct IndexEntry {
    source_table: String,
    source_id: String,
    content_preview: String,
}

/// Optional in-memory HNSW index for O(log n) approximate nearest neighbor search.
pub struct AnnIndex {
    inner: Arc<RwLock<Option<IndexState>>>,
    enabled: bool,
    pub min_entries_for_index: usize,
}

struct IndexState {
    hnsw: HnswMap<EmbeddingPoint, usize>,
    entries: Vec<IndexEntry>,
}

pub struct AnnSearchResult {
    pub source_table: String,
    pub source_id: String,
    pub content_preview: String,
    pub similarity: f64,
}

const DEFAULT_MIN_ENTRIES: usize = 100;

impl AnnIndex {
    pub fn new(enabled: bool) -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
            enabled,
            min_entries_for_index: DEFAULT_MIN_ENTRIES,
        }
    }

    /// Load all embeddings from the database and build the HNSW index.
    pub fn build_from_db(&self, db: &Database) -> ironclad_core::Result<usize> {
        if !self.enabled {
            return Ok(0);
        }

        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT source_table, source_id, content_preview, embedding_blob, embedding_json \
                 FROM embeddings",
            )
            .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;

        let mut points = Vec::new();
        let mut values = Vec::new();
        let mut entries = Vec::new();

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
            .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;

        for row in rows {
            let (source_table, source_id, content_preview, blob, json_text) =
                row.map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;

            let embedding = if let Some(b) = blob {
                if !b.is_empty() {
                    blob_to_embedding(&b)
                } else if !json_text.is_empty() {
                    serde_json::from_str(&json_text).unwrap_or_default()
                } else {
                    continue;
                }
            } else if !json_text.is_empty() {
                serde_json::from_str(&json_text).unwrap_or_default()
            } else {
                continue;
            };

            if embedding.is_empty() {
                continue;
            }

            let idx = entries.len();
            points.push(EmbeddingPoint(embedding));
            values.push(idx);
            entries.push(IndexEntry {
                source_table,
                source_id,
                content_preview,
            });
        }

        let count = points.len();
        if count < self.min_entries_for_index {
            *self.inner.write().unwrap() = None;
            return Ok(count);
        }

        let hnsw = Builder::default().build(points, values);
        *self.inner.write().unwrap() = Some(IndexState { hnsw, entries });

        Ok(count)
    }

    /// Search for the k nearest neighbors of `query_embedding`.
    /// Returns None if the index is not built, signaling the caller to fall back.
    pub fn search(&self, query_embedding: &[f32], k: usize) -> Option<Vec<AnnSearchResult>> {
        let guard = self.inner.read().unwrap();
        let state = guard.as_ref()?;

        let query = EmbeddingPoint(query_embedding.to_vec());
        let mut search = Search::default();

        let results: Vec<AnnSearchResult> = state
            .hnsw
            .search(&query, &mut search)
            .take(k)
            .map(|item| {
                let idx = *item.value;
                let entry = &state.entries[idx];
                let similarity = 1.0 - item.distance as f64;
                AnnSearchResult {
                    source_table: entry.source_table.clone(),
                    source_id: entry.source_id.clone(),
                    content_preview: entry.content_preview.clone(),
                    similarity,
                }
            })
            .collect();

        Some(results)
    }

    pub fn is_built(&self) -> bool {
        self.inner.read().unwrap().is_some()
    }

    pub fn entry_count(&self) -> usize {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .map(|s| s.entries.len())
            .unwrap_or(0)
    }

    /// Rebuild the entire HNSW index from the database. Call periodically to
    /// incorporate embeddings stored since the last build.
    pub fn rebuild(&self, db: &Database) -> ironclad_core::Result<usize> {
        self.build_from_db(db)
    }
}

impl Clone for AnnIndex {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            enabled: self.enabled,
            min_entries_for_index: self.min_entries_for_index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::store_embedding;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn disabled_index_returns_none() {
        let index = AnnIndex::new(false);
        let db = test_db();
        let count = index.build_from_db(&db).unwrap();
        assert_eq!(count, 0);
        assert!(!index.is_built());
        assert!(index.search(&[1.0, 0.0], 5).is_none());
    }

    #[test]
    fn empty_db_no_index() {
        let index = AnnIndex::new(true);
        let db = test_db();
        let count = index.build_from_db(&db).unwrap();
        assert_eq!(count, 0);
        assert!(!index.is_built());
    }

    #[test]
    fn below_min_entries_no_index() {
        let db = test_db();
        for i in 0..10 {
            store_embedding(
                &db,
                &format!("e{i}"),
                "t",
                &format!("{i}"),
                "preview",
                &[1.0, 0.0],
            )
            .unwrap();
        }
        let index = AnnIndex::new(true);
        let count = index.build_from_db(&db).unwrap();
        assert_eq!(count, 10);
        assert!(!index.is_built());
    }

    #[test]
    fn builds_index_above_threshold() {
        let db = test_db();
        let mut index = AnnIndex::new(true);
        index.min_entries_for_index = 5;

        for i in 0..10 {
            let emb = vec![i as f32 / 10.0, 1.0 - i as f32 / 10.0];
            store_embedding(
                &db,
                &format!("e{i}"),
                "test",
                &format!("t{i}"),
                &format!("entry {i}"),
                &emb,
            )
            .unwrap();
        }

        let count = index.build_from_db(&db).unwrap();
        assert_eq!(count, 10);
        assert!(index.is_built());
        assert_eq!(index.entry_count(), 10);
    }

    #[test]
    fn search_returns_nearest() {
        let db = test_db();
        let mut index = AnnIndex::new(true);
        index.min_entries_for_index = 3;

        store_embedding(&db, "e1", "test", "t1", "near", &[1.0, 0.0, 0.0]).unwrap();
        store_embedding(&db, "e2", "test", "t2", "far", &[0.0, 1.0, 0.0]).unwrap();
        store_embedding(&db, "e3", "test", "t3", "medium", &[0.7, 0.3, 0.0]).unwrap();

        index.build_from_db(&db).unwrap();
        assert!(index.is_built());

        let results = index.search(&[1.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content_preview, "near");
        assert!(results[0].similarity > results[1].similarity);
    }

    #[test]
    fn clone_shares_state() {
        let index = AnnIndex::new(true);
        let clone = index.clone();
        assert_eq!(index.is_built(), clone.is_built());
    }

    #[test]
    fn build_from_json_fallback() {
        let db = test_db();
        let mut index = AnnIndex::new(true);
        index.min_entries_for_index = 3;

        {
            let conn = db.conn();
            for i in 0..5 {
                let emb = vec![i as f32 / 5.0, 1.0 - i as f32 / 5.0, 0.5];
                let json = serde_json::to_string(&emb).unwrap();
                conn.execute(
                    "INSERT INTO embeddings (id, source_table, source_id, content_preview, embedding_json, dimensions) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![format!("j{i}"), "legacy", format!("l{i}"), format!("legacy {i}"), json, 3],
                )
                .unwrap();
            }
        }

        let count = index.build_from_db(&db).unwrap();
        assert_eq!(count, 5);
        assert!(index.is_built());

        let results = index.search(&[1.0, 0.0, 0.5], 2).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn build_skips_empty_embeddings() {
        let db = test_db();
        let mut index = AnnIndex::new(true);
        index.min_entries_for_index = 3;

        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO embeddings (id, source_table, source_id, content_preview, embedding_json, dimensions) \
                 VALUES ('empty1', 'test', 's1', 'empty', '', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO embeddings (id, source_table, source_id, content_preview, embedding_json, dimensions) \
                 VALUES ('empty2', 'test', 's2', 'empty2', '', 0)",
                [],
            )
            .unwrap();
        }

        store_embedding(&db, "valid1", "test", "v1", "ok1", &[1.0, 0.0, 0.0]).unwrap();
        store_embedding(&db, "valid2", "test", "v2", "ok2", &[0.0, 1.0, 0.0]).unwrap();
        store_embedding(&db, "valid3", "test", "v3", "ok3", &[0.0, 0.0, 1.0]).unwrap();

        let count = index.build_from_db(&db).unwrap();
        assert_eq!(count, 3);
        assert!(index.is_built());
    }

    #[test]
    fn build_skips_blob_with_no_data_falls_back_to_json() {
        let db = test_db();
        let mut index = AnnIndex::new(true);
        index.min_entries_for_index = 3;

        {
            let conn = db.conn();
            for i in 0..4 {
                let emb = vec![i as f32 / 4.0, 1.0 - i as f32 / 4.0, 0.5];
                conn.execute(
                    "INSERT INTO embeddings (id, source_table, source_id, content_preview, embedding_json, embedding_blob, dimensions) \
                     VALUES (?1, 'test', ?2, ?3, ?4, X'', 3)",
                    rusqlite::params![format!("mixed{i}"), format!("m{i}"), format!("mixed {i}"), serde_json::to_string(&emb).unwrap()],
                )
                .unwrap();
            }
        }

        let count = index.build_from_db(&db).unwrap();
        assert_eq!(count, 4);
        assert!(index.is_built());
    }
}
