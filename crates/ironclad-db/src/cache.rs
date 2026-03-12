use crate::Database;
use crate::embeddings::{blob_to_embedding, embedding_to_blob};
use ironclad_core::{IroncladError, Result};

/// A single persisted cache entry.
#[derive(Debug, Clone)]
pub struct PersistedCacheEntry {
    pub prompt_hash: String,
    pub response: String,
    pub model: String,
    pub tokens_saved: u32,
    pub hit_count: u32,
    pub embedding: Option<Vec<f32>>,
    pub created_at: String,
    pub expires_at: Option<String>,
}

/// Save a cache entry to the `semantic_cache` table.
pub fn save_cache_entry(db: &Database, id: &str, entry: &PersistedCacheEntry) -> Result<()> {
    let embedding_blob = entry.embedding.as_ref().map(|e| embedding_to_blob(e));

    let conn = db.conn();
    conn.execute(
        "INSERT OR REPLACE INTO semantic_cache \
         (id, prompt_hash, embedding, response, model, tokens_saved, hit_count, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id,
            entry.prompt_hash,
            embedding_blob,
            entry.response,
            entry.model,
            entry.tokens_saved,
            entry.hit_count,
            entry.created_at,
            entry.expires_at,
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(())
}

/// Load all non-expired cache entries from the database.
pub fn load_cache_entries(db: &Database) -> Result<Vec<(String, PersistedCacheEntry)>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, prompt_hash, embedding, response, model, tokens_saved, hit_count, \
             created_at, expires_at \
             FROM semantic_cache \
             WHERE expires_at IS NULL OR expires_at > datetime('now')",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let prompt_hash: String = row.get(1)?;
            let blob: Option<Vec<u8>> = row.get(2)?;
            let response: String = row.get(3)?;
            let model: String = row.get(4)?;
            let tokens_saved: u32 = row.get(5)?;
            let hit_count: u32 = row.get(6)?;
            let created_at: String = row.get(7)?;
            let expires_at: Option<String> = row.get(8)?;

            let embedding = blob.and_then(|b| {
                if b.is_empty() {
                    None
                } else {
                    Some(blob_to_embedding(&b))
                }
            });

            Ok((
                id,
                PersistedCacheEntry {
                    prompt_hash,
                    response,
                    model,
                    tokens_saved,
                    hit_count,
                    embedding,
                    created_at,
                    expires_at,
                },
            ))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

/// Maximum age (days) for cache entries that lack an explicit `expires_at`.
/// Prevents NULL-expiry rows from accumulating indefinitely.
const NULL_EXPIRY_MAX_AGE_DAYS: u32 = 7;

/// Remove expired entries from the semantic_cache table.
///
/// Evicts rows where:
/// 1. `expires_at` has passed, OR
/// 2. `expires_at IS NULL` and the row is older than [`NULL_EXPIRY_MAX_AGE_DAYS`].
pub fn evict_expired_cache(db: &Database) -> Result<usize> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            &format!(
                "DELETE FROM semantic_cache WHERE \
                 (expires_at IS NOT NULL AND expires_at <= datetime('now')) \
                 OR (expires_at IS NULL AND created_at <= datetime('now', '-{NULL_EXPIRY_MAX_AGE_DAYS} days'))"
            ),
            [],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(deleted)
}

/// Count of cached entries.
pub fn cache_count(db: &Database) -> Result<usize> {
    let conn = db.conn();
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM semantic_cache", [], |row| row.get(0))
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
    fn save_and_load_roundtrip() {
        let db = test_db();

        let entry = PersistedCacheEntry {
            prompt_hash: "abc123".into(),
            response: "Hello world".into(),
            model: "test-model".into(),
            tokens_saved: 50,
            hit_count: 3,
            embedding: Some(vec![0.1, 0.2, 0.3]),
            created_at: "2025-01-01T00:00:00".into(),
            expires_at: Some("2030-12-31T23:59:59".into()),
        };

        save_cache_entry(&db, "cache-1", &entry).unwrap();

        let loaded = load_cache_entries(&db).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "cache-1");
        assert_eq!(loaded[0].1.prompt_hash, "abc123");
        assert_eq!(loaded[0].1.response, "Hello world");
        assert_eq!(loaded[0].1.tokens_saved, 50);
        assert_eq!(loaded[0].1.hit_count, 3);
        assert!(loaded[0].1.embedding.is_some());
        assert_eq!(loaded[0].1.embedding.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn save_without_embedding() {
        let db = test_db();

        let entry = PersistedCacheEntry {
            prompt_hash: "def456".into(),
            response: "No embedding".into(),
            model: "test-model".into(),
            tokens_saved: 10,
            hit_count: 0,
            embedding: None,
            created_at: "2025-01-01T00:00:00".into(),
            expires_at: None,
        };

        save_cache_entry(&db, "cache-2", &entry).unwrap();

        let loaded = load_cache_entries(&db).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].1.embedding.is_none());
        assert!(loaded[0].1.expires_at.is_none());
    }

    #[test]
    fn evict_expired() {
        let db = test_db();

        let expired = PersistedCacheEntry {
            prompt_hash: "expired".into(),
            response: "old".into(),
            model: "m".into(),
            tokens_saved: 0,
            hit_count: 0,
            embedding: None,
            created_at: "2020-01-01T00:00:00".into(),
            expires_at: Some("2020-01-02T00:00:00".into()),
        };
        let fresh = PersistedCacheEntry {
            prompt_hash: "fresh".into(),
            response: "new".into(),
            model: "m".into(),
            tokens_saved: 0,
            hit_count: 0,
            embedding: None,
            created_at: "2025-01-01T00:00:00".into(),
            expires_at: Some("2030-12-31T23:59:59".into()),
        };

        save_cache_entry(&db, "c1", &expired).unwrap();
        save_cache_entry(&db, "c2", &fresh).unwrap();

        let evicted = evict_expired_cache(&db).unwrap();
        assert_eq!(evicted, 1);
        assert_eq!(cache_count(&db).unwrap(), 1);
    }

    #[test]
    fn evict_null_expiry_after_max_age() {
        let db = test_db();

        // Old entry with NULL expires_at — should be evicted.
        let old_null = PersistedCacheEntry {
            prompt_hash: "old_null".into(),
            response: "ancient".into(),
            model: "m".into(),
            tokens_saved: 0,
            hit_count: 0,
            embedding: None,
            created_at: "2020-01-01T00:00:00".into(),
            expires_at: None,
        };
        // Recent entry with NULL expires_at — should survive.
        let recent_null = PersistedCacheEntry {
            prompt_hash: "recent_null".into(),
            response: "fresh".into(),
            model: "m".into(),
            tokens_saved: 0,
            hit_count: 0,
            embedding: None,
            created_at: "2099-01-01T00:00:00".into(),
            expires_at: None,
        };

        save_cache_entry(&db, "c1", &old_null).unwrap();
        save_cache_entry(&db, "c2", &recent_null).unwrap();
        assert_eq!(cache_count(&db).unwrap(), 2);

        let evicted = evict_expired_cache(&db).unwrap();
        assert_eq!(evicted, 1);
        assert_eq!(cache_count(&db).unwrap(), 1);

        let remaining = load_cache_entries(&db).unwrap();
        assert_eq!(remaining[0].1.prompt_hash, "recent_null");
    }

    #[test]
    fn cache_count_empty() {
        let db = test_db();
        assert_eq!(cache_count(&db).unwrap(), 0);
    }

    #[test]
    fn replace_existing_entry() {
        let db = test_db();

        let entry1 = PersistedCacheEntry {
            prompt_hash: "hash".into(),
            response: "first".into(),
            model: "m".into(),
            tokens_saved: 10,
            hit_count: 1,
            embedding: None,
            created_at: "2025-01-01T00:00:00".into(),
            expires_at: None,
        };
        let entry2 = PersistedCacheEntry {
            prompt_hash: "hash".into(),
            response: "second".into(),
            model: "m".into(),
            tokens_saved: 20,
            hit_count: 5,
            embedding: None,
            created_at: "2025-01-02T00:00:00".into(),
            expires_at: None,
        };

        save_cache_entry(&db, "c1", &entry1).unwrap();
        save_cache_entry(&db, "c1", &entry2).unwrap();

        assert_eq!(cache_count(&db).unwrap(), 1);
        let loaded = load_cache_entries(&db).unwrap();
        assert_eq!(loaded[0].1.response, "second");
    }
}
