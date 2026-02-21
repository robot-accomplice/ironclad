use crate::Database;
use ironclad_core::{IroncladError, Result};

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
    conn.execute(
        "INSERT INTO working_memory (id, session_id, entry_type, content, importance) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, session_id, entry_type, content, importance],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn retrieve_working(db: &Database, session_id: &str) -> Result<Vec<WorkingEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, entry_type, content, importance, created_at \
             FROM working_memory WHERE session_id = ?1 ORDER BY importance DESC, created_at DESC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

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
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
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
    conn.execute(
        "INSERT INTO episodic_memory (id, classification, content, importance) \
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, classification, content, importance],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn retrieve_episodic(db: &Database, limit: i64) -> Result<Vec<EpisodicEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, classification, content, importance, created_at \
             FROM episodic_memory ORDER BY importance DESC, created_at DESC LIMIT ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

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
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
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
    conn.execute(
        "INSERT INTO semantic_memory (id, category, key, value, confidence) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(category, key) DO UPDATE SET value = excluded.value, \
         confidence = excluded.confidence, updated_at = datetime('now')",
        rusqlite::params![id, category, key, value, confidence],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn retrieve_semantic(db: &Database, category: &str) -> Result<Vec<SemanticEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, category, key, value, confidence, created_at, updated_at \
             FROM semantic_memory WHERE category = ?1 ORDER BY confidence DESC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

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
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
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
    conn.execute(
        "INSERT INTO procedural_memory (id, name, steps) VALUES (?1, ?2, ?3) \
         ON CONFLICT(name) DO UPDATE SET steps = excluded.steps, updated_at = datetime('now')",
        rusqlite::params![id, name, steps],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
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
    .map_err(|e| IroncladError::Database(e.to_string()))
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
    conn.execute(
        "INSERT INTO relationship_memory (id, entity_id, entity_name, trust_score) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(entity_id) DO UPDATE SET entity_name = excluded.entity_name, \
         trust_score = excluded.trust_score, interaction_count = interaction_count + 1, \
         last_interaction = datetime('now')",
        rusqlite::params![id, entity_id, entity_name, trust_score],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
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
    .map_err(|e| IroncladError::Database(e.to_string()))
}

// ── Full-text search across memory tiers ────────────────────────

/// Simple LIKE-based search across all memory content columns.
/// Returns matching content strings up to `limit`.
pub fn fts_search(db: &Database, query: &str, limit: i64) -> Result<Vec<String>> {
    let conn = db.conn();
    let pattern = format!("%{query}%");
    let mut results = Vec::new();

    let tables_and_cols: &[(&str, &str)] = &[
        ("working_memory", "content"),
        ("episodic_memory", "content"),
        ("semantic_memory", "value"),
        ("procedural_memory", "steps"),
        ("relationship_memory", "interaction_summary"),
    ];

    for &(table, col) in tables_and_cols {
        let sql = format!("SELECT {col} FROM {table} WHERE {col} LIKE ?1 LIMIT ?2");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![pattern, limit], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        for row in rows {
            results.push(row.map_err(|e| IroncladError::Database(e.to_string()))?);
            if results.len() as i64 >= limit {
                return Ok(results);
            }
        }
    }

    Ok(results)
}

trait Optional<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> Optional<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
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
}
