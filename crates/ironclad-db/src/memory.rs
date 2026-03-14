use crate::{Database, DbResultExt};
use chrono::Utc;
use ironclad_core::Result;
use rusqlite::OptionalExtension;

// ── Working memory ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WorkingEntry {
    pub id: String,
    pub session_id: String,
    pub entry_type: String,
    pub content: String,
    pub importance: i32,
    pub created_at: String,
}

pub fn store_working(
    db: &Database,
    session_id: &str,
    entry_type: &str,
    content: &str,
    importance: i32,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let tx = conn.unchecked_transaction().db_err()?;
    tx.execute(
        "INSERT INTO working_memory (id, session_id, entry_type, content, importance, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, session_id, entry_type, content, importance, now],
    )
    .db_err()?;
    // Remove any existing FTS row before inserting to avoid duplicates.
    tx.execute(
        "DELETE FROM memory_fts WHERE source_table = 'working' AND source_id = ?1",
        rusqlite::params![id],
    )
    .db_err()?;
    tx.execute(
        "INSERT INTO memory_fts (content, category, source_table, source_id) VALUES (?1, ?2, 'working', ?3)",
        rusqlite::params![content, entry_type, id],
    )
    .db_err()?;
    tx.commit().db_err()?;
    Ok(id)
}

pub fn retrieve_working(db: &Database, session_id: &str) -> Result<Vec<WorkingEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, entry_type, content, importance, created_at \
             FROM working_memory WHERE session_id = ?1 ORDER BY importance DESC, created_at DESC",
        )
        .db_err()?;

    let rows = stmt
        .query_map([session_id], |row| {
            Ok(WorkingEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                entry_type: row.get(2)?,
                content: row.get(3)?,
                importance: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

pub fn retrieve_working_all(db: &Database, limit: i64) -> Result<Vec<WorkingEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, entry_type, content, importance, created_at \
             FROM working_memory ORDER BY importance DESC, created_at DESC LIMIT ?1",
        )
        .db_err()?;

    let rows = stmt
        .query_map([limit], |row| {
            Ok(WorkingEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                entry_type: row.get(2)?,
                content: row.get(3)?,
                importance: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

// ── Episodic memory ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EpisodicEntry {
    pub id: String,
    pub classification: String,
    pub content: String,
    pub importance: i32,
    pub created_at: String,
}

pub fn store_episodic(
    db: &Database,
    classification: &str,
    content: &str,
    importance: i32,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO episodic_memory (id, classification, content, importance, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, classification, content, importance, now],
    )
    .db_err()?;

    // FTS insert handled by episodic_ai trigger

    Ok(id)
}

pub fn retrieve_episodic(db: &Database, limit: i64) -> Result<Vec<EpisodicEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, classification, content, importance, created_at \
             FROM episodic_memory ORDER BY importance DESC, created_at DESC LIMIT ?1",
        )
        .db_err()?;

    let rows = stmt
        .query_map([limit], |row| {
            Ok(EpisodicEntry {
                id: row.get(0)?,
                classification: row.get(1)?,
                content: row.get(2)?,
                importance: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

// ── Semantic memory ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SemanticEntry {
    pub id: String,
    pub category: String,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub created_at: String,
    pub updated_at: String,
}

pub fn store_semantic(
    db: &Database,
    category: &str,
    key: &str,
    value: &str,
    confidence: f64,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let tx = conn.unchecked_transaction().db_err()?;
    tx.execute(
        "INSERT INTO semantic_memory (id, category, key, value, confidence, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(category, key) DO UPDATE SET value = excluded.value, \
         confidence = excluded.confidence, updated_at = ?6",
        rusqlite::params![id, category, key, value, confidence, now],
    )
    .db_err()?;

    let actual_id: String = tx
        .query_row(
            "SELECT id FROM semantic_memory WHERE category = ?1 AND key = ?2",
            rusqlite::params![category, key],
            |row| row.get(0),
        )
        .db_err()?;

    // Remove any existing FTS row before re-inserting to avoid duplicates on upsert.
    tx.execute(
        "DELETE FROM memory_fts WHERE source_table = 'semantic' AND source_id = ?1",
        rusqlite::params![actual_id],
    )
    .db_err()?;
    tx.execute(
        "INSERT INTO memory_fts (content, category, source_table, source_id) VALUES (?1, ?2, 'semantic', ?3)",
        rusqlite::params![value, category, actual_id],
    )
    .db_err()?;
    tx.commit().db_err()?;

    Ok(actual_id)
}

pub fn retrieve_semantic(db: &Database, category: &str) -> Result<Vec<SemanticEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, category, key, value, confidence, created_at, updated_at \
             FROM semantic_memory WHERE category = ?1 ORDER BY confidence DESC",
        )
        .db_err()?;

    let rows = stmt
        .query_map([category], |row| {
            Ok(SemanticEntry {
                id: row.get(0)?,
                category: row.get(1)?,
                key: row.get(2)?,
                value: row.get(3)?,
                confidence: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

pub fn list_semantic_categories(db: &Database) -> Result<Vec<(String, i64)>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT category, COUNT(*) as cnt FROM semantic_memory \
             GROUP BY category ORDER BY cnt DESC",
        )
        .db_err()?;

    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

pub fn retrieve_semantic_all(db: &Database, limit: i64) -> Result<Vec<SemanticEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, category, key, value, confidence, created_at, updated_at \
             FROM semantic_memory ORDER BY confidence DESC, updated_at DESC LIMIT ?1",
        )
        .db_err()?;

    let rows = stmt
        .query_map([limit], |row| {
            Ok(SemanticEntry {
                id: row.get(0)?,
                category: row.get(1)?,
                key: row.get(2)?,
                value: row.get(3)?,
                confidence: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

// ── Procedural memory ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProceduralEntry {
    pub id: String,
    pub name: String,
    pub steps: String,
    pub success_count: i64,
    pub failure_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

pub fn store_procedural(db: &Database, name: &str, steps: &str) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO procedural_memory (id, name, steps, created_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(name) DO UPDATE SET steps = excluded.steps, updated_at = ?4",
        rusqlite::params![id, name, steps, now],
    )
    .db_err()?;
    Ok(id)
}

pub fn retrieve_procedural(db: &Database, name: &str) -> Result<Option<ProceduralEntry>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, name, steps, success_count, failure_count, created_at, updated_at \
         FROM procedural_memory WHERE name = ?1",
        [name],
        |row| {
            Ok(ProceduralEntry {
                id: row.get(0)?,
                name: row.get(1)?,
                steps: row.get(2)?,
                success_count: row.get(3)?,
                failure_count: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    )
    .optional()
    .db_err()
}

pub fn record_procedural_success(db: &Database, name: &str) -> Result<()> {
    let conn = db.conn();
    // Auto-register the tool if it hasn't been seen before, then increment.
    // Must include `steps` (NOT NULL) — SQLite evaluates NOT NULL before the
    // ON CONFLICT(name) upsert path, so omitting it causes a hard failure
    // even when the row already exists.
    conn.execute(
        "INSERT INTO procedural_memory (id, name, steps, success_count, failure_count, created_at, updated_at) \
         VALUES (lower(hex(randomblob(16))), ?1, '', 0, 0, datetime('now'), datetime('now')) \
         ON CONFLICT(name) DO NOTHING",
        [name],
    )
    .db_err()?;
    conn.execute(
        "UPDATE procedural_memory SET success_count = success_count + 1, updated_at = datetime('now') WHERE name = ?1",
        [name],
    )
    .db_err()?;
    Ok(())
}

pub fn record_procedural_failure(db: &Database, name: &str) -> Result<()> {
    let conn = db.conn();
    // Auto-register the tool if it hasn't been seen before, then increment.
    // Must include `steps` (NOT NULL) — see record_procedural_success comment.
    conn.execute(
        "INSERT INTO procedural_memory (id, name, steps, success_count, failure_count, created_at, updated_at) \
         VALUES (lower(hex(randomblob(16))), ?1, '', 0, 0, datetime('now'), datetime('now')) \
         ON CONFLICT(name) DO NOTHING",
        [name],
    )
    .db_err()?;
    conn.execute(
        "UPDATE procedural_memory SET failure_count = failure_count + 1, updated_at = datetime('now') WHERE name = ?1",
        [name],
    )
    .db_err()?;
    Ok(())
}

/// Delete procedural entries with zero activity (no successes AND no failures)
/// that haven't been updated in at least `stale_days` days.
///
/// Returns the number of rows deleted.
pub fn prune_stale_procedural(db: &Database, stale_days: u32) -> Result<usize> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM procedural_memory \
             WHERE success_count = 0 AND failure_count = 0 \
               AND updated_at < datetime('now', ?1)",
            [format!("-{stale_days} days")],
        )
        .db_err()?;
    Ok(deleted)
}

// ── Relationship memory ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RelationshipEntry {
    pub id: String,
    pub entity_id: String,
    pub entity_name: Option<String>,
    pub trust_score: f64,
    pub interaction_summary: Option<String>,
    pub interaction_count: i64,
    pub last_interaction: Option<String>,
    pub created_at: String,
}

pub fn store_relationship(
    db: &Database,
    entity_id: &str,
    entity_name: &str,
    trust_score: f64,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO relationship_memory (id, entity_id, entity_name, trust_score, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(entity_id) DO UPDATE SET entity_name = excluded.entity_name, \
         trust_score = excluded.trust_score, interaction_count = interaction_count + 1, \
         last_interaction = ?5",
        rusqlite::params![id, entity_id, entity_name, trust_score, now],
    )
    .db_err()?;
    Ok(id)
}

pub fn retrieve_relationship(db: &Database, entity_id: &str) -> Result<Option<RelationshipEntry>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, entity_id, entity_name, trust_score, interaction_summary, \
         interaction_count, last_interaction, created_at \
         FROM relationship_memory WHERE entity_id = ?1",
        [entity_id],
        |row| {
            Ok(RelationshipEntry {
                id: row.get(0)?,
                entity_id: row.get(1)?,
                entity_name: row.get(2)?,
                trust_score: row.get(3)?,
                interaction_summary: row.get(4)?,
                interaction_count: row.get(5)?,
                last_interaction: row.get(6)?,
                created_at: row.get(7)?,
            })
        },
    )
    .optional()
    .db_err()
}

// ── Full-text search across memory tiers ────────────────────────

// ── Search results ──────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemorySearchResult {
    pub content: String,
    pub category: String,
    pub source: String,
}

/// Sanitize user input for FTS5: keep only alphanumeric and whitespace, wrap in double quotes
/// (phrase query), and escape any remaining double quotes so FTS5 operators (AND, OR, NOT, etc.)
/// cannot be injected.
pub(crate) fn sanitize_fts_query(query: &str) -> String {
    let stripped: String = query
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    format!("\"{}\"", stripped.replace('"', "\"\""))
}

/// Search memory: FTS5 MATCH on memory_fts (working, episodic, semantic), LIKE fallback for others.
/// Returns matching structured entries (content + category + source) up to `limit`, deduplicated.
pub fn fts_search(db: &Database, query: &str, limit: i64) -> Result<Vec<MemorySearchResult>> {
    let conn = db.conn();
    let mut results: Vec<MemorySearchResult> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // FTS5 MATCH on memory_fts (populated from working_memory, episodic_memory, semantic_memory)
    let fts_query = sanitize_fts_query(query);
    match conn.prepare(
        "SELECT content, category, source_table FROM memory_fts WHERE memory_fts MATCH ?1 LIMIT ?2",
    ) {
        Ok(mut stmt) => {
            match stmt.query_map(rusqlite::params![fts_query, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            }) {
                Ok(rows) => {
                    for row in rows.flatten() {
                        let key = format!("{}|{}", row.2, row.0);
                        if seen.insert(key) {
                            results.push(MemorySearchResult {
                                content: row.0,
                                category: row.1,
                                source: row.2,
                            });
                            if results.len() as i64 >= limit {
                                return Ok(results);
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "FTS5 query_map failed"),
            }
        }
        Err(e) => tracing::warn!(error = %e, "FTS5 query preparation failed"),
    }

    // LIKE fallback for tables not in FTS: procedural_memory.steps, relationship_memory.interaction_summary.
    // Safety: table and column names below are hardcoded constants, not user input,
    // so the string interpolation into SQL is safe from injection.
    // Escape % and _ so they are literal, and use ESCAPE '\\'.
    let escaped_query = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped_query}%");
    let tables_and_cols: &[(&str, &str)] = &[
        ("procedural_memory", "steps"),
        ("relationship_memory", "interaction_summary"),
    ];

    for &(table, col) in tables_and_cols {
        let sql = format!("SELECT {col} FROM {table} WHERE {col} LIKE ?1 ESCAPE '\\' LIMIT ?2");
        match conn.prepare(&sql) {
            Ok(mut stmt) => {
                match stmt.query_map(rusqlite::params![pattern, limit], |row| {
                    row.get::<_, String>(0)
                }) {
                    Ok(rows) => {
                        for row in rows.flatten() {
                            let key = format!("{table}|{row}");
                            if seen.insert(key) {
                                results.push(MemorySearchResult {
                                    content: row,
                                    category: table.replace("_memory", ""),
                                    source: table.to_string(),
                                });
                                if results.len() as i64 >= limit {
                                    return Ok(results);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, table, col, "LIKE fallback query_map failed")
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, table, col, "LIKE fallback query preparation failed")
            }
        }
    }

    Ok(results)
}

// ── Episodic dead-entry cleanup ────────────────────────────────

/// Delete episodic entries with `importance <= 1` that are older than
/// `stale_days` days.  These low-signal entries accumulate over time and
/// bloat the episodic tier without contributing useful retrieval context.
///
/// Returns the number of rows deleted.
pub fn prune_dead_episodic(db: &Database, stale_days: u32) -> Result<usize> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM episodic_memory \
             WHERE importance <= 1 \
               AND created_at < datetime('now', ?1)",
            [format!("-{stale_days} days")],
        )
        .db_err()?;
    Ok(deleted)
}

// ── Orphan cleanup ─────────────────────────────────────────────

/// Delete working_memory rows whose `session_id` no longer exists in `sessions`.
///
/// Returns the number of orphaned rows removed.
pub fn cleanup_orphaned_working_memory(db: &Database) -> Result<usize> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM working_memory \
             WHERE session_id NOT IN (SELECT id FROM sessions)",
            [],
        )
        .db_err()?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn working_memory_roundtrip() {
        let db = test_db();
        store_working(&db, "sess-1", "goal", "find food", 8).unwrap();
        store_working(&db, "sess-1", "observation", "sun is up", 3).unwrap();

        let entries = retrieve_working(&db, "sess-1").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].importance, 8, "higher importance first");
    }

    #[test]
    fn episodic_memory_roundtrip() {
        let db = test_db();
        store_episodic(&db, "success", "deployed v1.0", 9).unwrap();
        store_episodic(&db, "failure", "ran out of credits", 7).unwrap();

        let entries = retrieve_episodic(&db, 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].classification, "success");
    }

    #[test]
    fn semantic_memory_upsert() {
        let db = test_db();
        store_semantic(&db, "facts", "sky_color", "blue", 0.9).unwrap();
        store_semantic(&db, "facts", "sky_color", "grey", 0.7).unwrap();

        let entries = retrieve_semantic(&db, "facts").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, "grey");
    }

    #[test]
    fn procedural_memory_roundtrip() {
        let db = test_db();
        store_procedural(&db, "deploy", r#"["build","push","verify"]"#).unwrap();
        let entry = retrieve_procedural(&db, "deploy").unwrap().unwrap();
        assert_eq!(entry.name, "deploy");
    }

    #[test]
    fn relationship_memory_roundtrip() {
        let db = test_db();
        store_relationship(&db, "user-42", "Jon", 0.9).unwrap();
        let entry = retrieve_relationship(&db, "user-42").unwrap().unwrap();
        assert_eq!(entry.entity_name.as_deref(), Some("Jon"));
        assert!((entry.trust_score - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn fts_search_finds_across_tiers() {
        let db = test_db();
        store_working(&db, "s1", "note", "the quick brown fox", 5).unwrap();
        store_episodic(&db, "event", "a lazy dog appeared", 5).unwrap();
        store_semantic(&db, "facts", "animal", "fox is quick", 0.8).unwrap();

        let hits = fts_search(&db, "quick", 10).unwrap();
        assert_eq!(hits.len(), 2, "should match working + semantic");
    }

    #[test]
    fn fts_search_finds_episodic_via_trigger() {
        let db = test_db();
        store_episodic(&db, "discovery", "the quantum engine hummed", 9).unwrap();

        let hits = fts_search(&db, "quantum", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("quantum"));
    }

    #[test]
    fn fts_respects_limit() {
        let db = test_db();
        for i in 0..5 {
            store_working(&db, "s1", "note", &format!("alpha item {i}"), 1).unwrap();
        }
        let hits = fts_search(&db, "alpha", 3).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn semantic_upsert_returns_existing_id() {
        let db = test_db();
        let id1 = store_semantic(&db, "prefs", "color", "blue", 0.9).unwrap();
        let id2 = store_semantic(&db, "prefs", "color", "red", 0.8).unwrap();
        assert_eq!(id1, id2, "upsert should return the original row id");
    }

    #[test]
    fn procedural_failure_tracking() {
        let db = test_db();
        store_procedural(&db, "deploy", r#"["build","push"]"#).unwrap();
        let entry = retrieve_procedural(&db, "deploy").unwrap().unwrap();
        assert_eq!(entry.failure_count, 0);

        record_procedural_failure(&db, "deploy").unwrap();
        record_procedural_failure(&db, "deploy").unwrap();
        let entry = retrieve_procedural(&db, "deploy").unwrap().unwrap();
        assert_eq!(entry.failure_count, 2);
    }

    #[test]
    fn store_working_writes_both_tables() {
        let db = test_db();
        let id = store_working(&db, "sess-1", "fact", "the sky is blue", 5).unwrap();

        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM working_memory WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_fts WHERE source_id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn record_procedural_success_tracking() {
        let db = test_db();
        store_procedural(&db, "deploy", r#"["build","push"]"#).unwrap();
        record_procedural_success(&db, "deploy").unwrap();
        record_procedural_success(&db, "deploy").unwrap();
        record_procedural_success(&db, "deploy").unwrap();
        let entry = retrieve_procedural(&db, "deploy").unwrap().unwrap();
        assert_eq!(entry.success_count, 3);
    }

    #[test]
    fn retrieve_working_empty_session() {
        let db = test_db();
        let entries = retrieve_working(&db, "nonexistent-session").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn retrieve_working_is_session_isolated() {
        let db = test_db();
        store_working(&db, "sess-a", "note", "alpha", 5).unwrap();
        store_working(&db, "sess-b", "note", "beta", 5).unwrap();

        let a = retrieve_working(&db, "sess-a").unwrap();
        let b = retrieve_working(&db, "sess-b").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].content, "alpha");
        assert_eq!(b[0].content, "beta");
    }

    #[test]
    fn retrieve_episodic_limit_zero() {
        let db = test_db();
        store_episodic(&db, "event", "something happened", 5).unwrap();
        let entries = retrieve_episodic(&db, 0).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn retrieve_semantic_empty_category() {
        let db = test_db();
        let entries = retrieve_semantic(&db, "no-such-category").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn retrieve_procedural_nonexistent() {
        let db = test_db();
        let entry = retrieve_procedural(&db, "nonexistent").unwrap();
        assert!(entry.is_none());
    }

    #[test]
    fn retrieve_relationship_nonexistent() {
        let db = test_db();
        let entry = retrieve_relationship(&db, "no-such-entity").unwrap();
        assert!(entry.is_none());
    }

    #[test]
    fn store_relationship_upsert_increments_interaction() {
        let db = test_db();
        store_relationship(&db, "user-1", "Alice", 0.5).unwrap();
        store_relationship(&db, "user-1", "Alice Updated", 0.8).unwrap();
        let entry = retrieve_relationship(&db, "user-1").unwrap().unwrap();
        assert_eq!(entry.interaction_count, 1);
    }

    #[test]
    fn store_procedural_upsert_updates_steps() {
        let db = test_db();
        store_procedural(&db, "deploy", r#"["build"]"#).unwrap();
        store_procedural(&db, "deploy", r#"["build","push","verify"]"#).unwrap();
        let entry = retrieve_procedural(&db, "deploy").unwrap().unwrap();
        assert_eq!(entry.steps, r#"["build","push","verify"]"#);
    }

    #[test]
    fn fts_search_no_matches() {
        let db = test_db();
        store_working(&db, "s1", "note", "hello world", 5).unwrap();
        let hits = fts_search(&db, "zzzznotfound", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn fts_search_like_fallback_procedural() {
        let db = test_db();
        store_procedural(&db, "backup", "step one: tar the archive and compress").unwrap();
        let hits = fts_search(&db, "tar the archive", 10).unwrap();
        assert!(!hits.is_empty());
    }

    // ── retrieve_working_all tests ────────────────────────────

    #[test]
    fn retrieve_working_all_returns_across_sessions() {
        let db = test_db();
        store_working(&db, "sess-a", "note", "alpha entry", 5).unwrap();
        store_working(&db, "sess-b", "note", "beta entry", 8).unwrap();
        store_working(&db, "sess-c", "note", "gamma entry", 3).unwrap();

        let entries = retrieve_working_all(&db, 100).unwrap();
        assert_eq!(entries.len(), 3);
        // Ordered by importance DESC
        assert_eq!(entries[0].importance, 8);
        assert_eq!(entries[1].importance, 5);
        assert_eq!(entries[2].importance, 3);
    }

    #[test]
    fn retrieve_working_all_respects_limit() {
        let db = test_db();
        for i in 0..5 {
            store_working(&db, "sess", "note", &format!("entry {i}"), i).unwrap();
        }
        let entries = retrieve_working_all(&db, 2).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn retrieve_working_all_empty_db() {
        let db = test_db();
        let entries = retrieve_working_all(&db, 10).unwrap();
        assert!(entries.is_empty());
    }

    // ── list_semantic_categories tests ────────────────────────

    #[test]
    fn list_semantic_categories_returns_grouped() {
        let db = test_db();
        store_semantic(&db, "facts", "sky_color", "blue", 0.9).unwrap();
        store_semantic(&db, "facts", "grass_color", "green", 0.8).unwrap();
        store_semantic(&db, "prefs", "theme", "dark", 0.7).unwrap();

        let categories = list_semantic_categories(&db).unwrap();
        assert_eq!(categories.len(), 2);
        // Ordered by count DESC
        assert_eq!(categories[0].0, "facts");
        assert_eq!(categories[0].1, 2);
        assert_eq!(categories[1].0, "prefs");
        assert_eq!(categories[1].1, 1);
    }

    #[test]
    fn list_semantic_categories_empty() {
        let db = test_db();
        let categories = list_semantic_categories(&db).unwrap();
        assert!(categories.is_empty());
    }

    // ── retrieve_semantic_all tests ──────────────────────────

    #[test]
    fn retrieve_semantic_all_returns_across_categories() {
        let db = test_db();
        store_semantic(&db, "facts", "sky", "blue", 0.9).unwrap();
        store_semantic(&db, "prefs", "theme", "dark", 0.7).unwrap();
        store_semantic(&db, "facts", "grass", "green", 0.8).unwrap();

        let entries = retrieve_semantic_all(&db, 100).unwrap();
        assert_eq!(entries.len(), 3);
        // Ordered by confidence DESC
        assert!((entries[0].confidence - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn retrieve_semantic_all_respects_limit() {
        let db = test_db();
        for i in 0..5 {
            store_semantic(
                &db,
                "cat",
                &format!("key{i}"),
                &format!("val{i}"),
                0.5 + i as f64 * 0.1,
            )
            .unwrap();
        }
        let entries = retrieve_semantic_all(&db, 2).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn retrieve_semantic_all_empty() {
        let db = test_db();
        let entries = retrieve_semantic_all(&db, 10).unwrap();
        assert!(entries.is_empty());
    }

    // ── fts_search LIKE fallback additional paths ────────────

    #[test]
    fn fts_search_like_fallback_relationship() {
        let db = test_db();
        // Store a relationship with an interaction_summary that can be found via LIKE fallback
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO relationship_memory (id, entity_id, entity_name, trust_score, interaction_summary) \
                 VALUES ('r1', 'user-99', 'TestUser', 0.8, 'discussed the quantum physics experiment')",
                [],
            ).unwrap();
        }

        let hits = fts_search(&db, "quantum physics", 10).unwrap();
        assert!(
            !hits.is_empty(),
            "LIKE fallback should find relationship interaction_summary"
        );
    }

    #[test]
    fn fts_search_limit_reached_in_fts_phase() {
        let db = test_db();
        // Create enough FTS entries so the limit is reached during the FTS phase
        for i in 0..5 {
            store_working(
                &db,
                "sess",
                "note",
                &format!("searchable keyword item {i}"),
                5,
            )
            .unwrap();
        }
        let hits = fts_search(&db, "keyword", 2).unwrap();
        assert_eq!(hits.len(), 2, "should stop at limit during FTS phase");
    }

    #[test]
    fn fts_search_limit_reached_in_like_phase() {
        let db = test_db();
        // Store items in procedural memory (LIKE fallback) with a common pattern
        for i in 0..5 {
            store_procedural(
                &db,
                &format!("proc_{i}"),
                &format!("step: run the xyzzy command {i}"),
            )
            .unwrap();
        }
        let hits = fts_search(&db, "xyzzy command", 2).unwrap();
        assert_eq!(
            hits.len(),
            2,
            "should stop at limit during LIKE fallback phase"
        );
    }

    #[test]
    fn fts_search_special_chars_in_query() {
        let db = test_db();
        store_working(
            &db,
            "sess",
            "note",
            "test with percent % and underscore _",
            5,
        )
        .unwrap();
        // This tests the sanitize_fts_query and the LIKE escape logic
        let hits = fts_search(&db, "percent", 10).unwrap();
        assert!(!hits.is_empty());
    }

    #[test]
    fn sanitize_fts_query_strips_operators() {
        // FTS5 operators like AND, OR, NOT should be neutralized by the sanitizer
        let result = sanitize_fts_query("hello AND world");
        // Should wrap in quotes, stripping non-alnum/space
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }

    #[test]
    fn sanitize_fts_query_empty() {
        let result = sanitize_fts_query("");
        assert_eq!(result, "\"\"");
    }

    #[test]
    fn sanitize_fts_query_special_chars_stripped() {
        let result = sanitize_fts_query("hello* OR world");
        // * and OR should be kept as alphanumeric/space
        assert!(!result.contains('*'));
    }

    #[test]
    fn prune_stale_procedural_removes_zero_activity_entries() {
        let db = test_db();
        // Create a procedural entry via store (will have success_count=0, failure_count=0)
        store_procedural(&db, "stale-tool", "do something").unwrap();

        // Backdate its updated_at to 60 days ago
        db.conn()
            .execute(
                "UPDATE procedural_memory SET updated_at = datetime('now', '-60 days') WHERE name = ?1",
                ["stale-tool"],
            )
            .unwrap();

        // Also create one with activity — should NOT be pruned
        store_procedural(&db, "active-tool", "steps").unwrap();
        record_procedural_success(&db, "active-tool").unwrap();
        db.conn()
            .execute(
                "UPDATE procedural_memory SET updated_at = datetime('now', '-60 days') WHERE name = ?1",
                ["active-tool"],
            )
            .unwrap();

        let pruned = prune_stale_procedural(&db, 30).unwrap();
        assert_eq!(pruned, 1);

        // stale-tool gone, active-tool remains
        assert!(retrieve_procedural(&db, "stale-tool").unwrap().is_none());
        assert!(retrieve_procedural(&db, "active-tool").unwrap().is_some());
    }

    #[test]
    fn prune_stale_procedural_ignores_recent_entries() {
        let db = test_db();
        store_procedural(&db, "fresh-tool", "steps").unwrap();
        // Don't backdate — should not be pruned
        let pruned = prune_stale_procedural(&db, 30).unwrap();
        assert_eq!(pruned, 0);
        assert!(retrieve_procedural(&db, "fresh-tool").unwrap().is_some());
    }

    // ── Episodic dead-entry cleanup tests ─────────────────────

    #[test]
    fn prune_dead_episodic_removes_low_importance_old() {
        let db = test_db();
        store_episodic(&db, "noise", "irrelevant chatter", 1).unwrap();
        store_episodic(&db, "signal", "critical event", 8).unwrap();

        // Backdate the low-importance entry
        db.conn()
            .execute(
                "UPDATE episodic_memory SET created_at = datetime('now', '-60 days') \
                 WHERE importance <= 1",
                [],
            )
            .unwrap();

        let pruned = prune_dead_episodic(&db, 30).unwrap();
        assert_eq!(pruned, 1);

        let remaining = retrieve_episodic(&db, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].content, "critical event");
    }

    #[test]
    fn prune_dead_episodic_keeps_recent_low_importance() {
        let db = test_db();
        store_episodic(&db, "recent-noise", "just happened", 1).unwrap();
        // Don't backdate — should not be pruned
        let pruned = prune_dead_episodic(&db, 30).unwrap();
        assert_eq!(pruned, 0);
    }

    #[test]
    fn prune_dead_episodic_keeps_old_high_importance() {
        let db = test_db();
        store_episodic(&db, "important", "old but critical", 5).unwrap();
        db.conn()
            .execute(
                "UPDATE episodic_memory SET created_at = datetime('now', '-90 days')",
                [],
            )
            .unwrap();
        let pruned = prune_dead_episodic(&db, 30).unwrap();
        assert_eq!(pruned, 0);
    }

    // ── Orphan cleanup tests ─────────────────────────────────

    #[test]
    fn cleanup_orphaned_working_memory_removes_dangling() {
        let db = test_db();
        // Create a real session so its working_memory survives.
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, agent_id) VALUES ('live-sess', 'a')",
            [],
        )
        .unwrap();
        drop(conn);

        store_working(&db, "live-sess", "note", "survives", 5).unwrap();
        store_working(&db, "dead-sess", "note", "orphaned", 5).unwrap();

        let deleted = cleanup_orphaned_working_memory(&db).unwrap();
        assert_eq!(deleted, 1);

        let remaining = retrieve_working(&db, "live-sess").unwrap();
        assert_eq!(remaining.len(), 1);
        let gone = retrieve_working(&db, "dead-sess").unwrap();
        assert!(gone.is_empty());
    }

    #[test]
    fn cleanup_orphaned_working_memory_noop_when_clean() {
        let db = test_db();
        let conn = db.conn();
        conn.execute("INSERT INTO sessions (id, agent_id) VALUES ('s1', 'a')", [])
            .unwrap();
        drop(conn);

        store_working(&db, "s1", "note", "ok", 5).unwrap();
        let deleted = cleanup_orphaned_working_memory(&db).unwrap();
        assert_eq!(deleted, 0);
    }
}
