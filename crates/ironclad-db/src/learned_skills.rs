//! CRUD operations for the `learned_skills` table.
//!
//! When sessions close, the agent detects successful multi-step tool sequences
//! and synthesizes reusable skill documents.  This module tracks those learned
//! skills and their reinforcement history (success/failure counts, priority).

use crate::Database;
use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;

// ── Record type ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LearnedSkillRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub trigger_tools: String,
    pub steps_json: String,
    pub source_session_id: Option<String>,
    pub success_count: i64,
    pub failure_count: i64,
    pub priority: i64,
    pub skill_md_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ── Store ──────────────────────────────────────────────────────

/// Insert a new learned skill.  Uses `ON CONFLICT(name)` to update if the
/// skill already exists (idempotent re-learning).
pub fn store_learned_skill(
    db: &Database,
    name: &str,
    description: &str,
    trigger_tools: &str,
    steps_json: &str,
    source_session_id: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO learned_skills \
             (id, name, description, trigger_tools, steps_json, source_session_id, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
         ON CONFLICT(name) DO UPDATE SET \
             description   = excluded.description, \
             trigger_tools = excluded.trigger_tools, \
             steps_json    = excluded.steps_json, \
             success_count = success_count + 1, \
             updated_at    = ?7",
        rusqlite::params![id, name, description, trigger_tools, steps_json, source_session_id, now],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

// ── Retrieve ───────────────────────────────────────────────────

pub fn get_learned_skill_by_name(db: &Database, name: &str) -> Result<Option<LearnedSkillRecord>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, name, description, trigger_tools, steps_json, source_session_id, \
                success_count, failure_count, priority, skill_md_path, created_at, updated_at \
         FROM learned_skills WHERE name = ?1",
        [name],
        row_to_record,
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

/// List learned skills ordered by priority descending.
pub fn list_learned_skills(db: &Database, limit: usize) -> Result<Vec<LearnedSkillRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, description, trigger_tools, steps_json, source_session_id, \
                    success_count, failure_count, priority, skill_md_path, created_at, updated_at \
             FROM learned_skills ORDER BY priority DESC LIMIT ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([limit as i64], row_to_record)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

// ── Reinforcement ──────────────────────────────────────────────

pub fn record_learned_skill_success(db: &Database, name: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE learned_skills SET success_count = success_count + 1, \
         updated_at = datetime('now') WHERE name = ?1",
        [name],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn record_learned_skill_failure(db: &Database, name: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE learned_skills SET failure_count = failure_count + 1, \
         updated_at = datetime('now') WHERE name = ?1",
        [name],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

// ── Priority ───────────────────────────────────────────────────

pub fn update_learned_skill_priority(db: &Database, name: &str, new_priority: i64) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE learned_skills SET priority = ?1, updated_at = datetime('now') WHERE name = ?2",
        rusqlite::params![new_priority, name],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

// ── Skill-md path ──────────────────────────────────────────────

pub fn set_learned_skill_md_path(db: &Database, name: &str, path: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE learned_skills SET skill_md_path = ?1, updated_at = datetime('now') WHERE name = ?2",
        rusqlite::params![path, name],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

// ── Count ──────────────────────────────────────────────────────

pub fn count_learned_skills(db: &Database) -> Result<usize> {
    let conn = db.conn();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM learned_skills", [], |r| r.get(0))
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(count as usize)
}

// ── Internal ───────────────────────────────────────────────────

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<LearnedSkillRecord> {
    Ok(LearnedSkillRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        trigger_tools: row.get(3)?,
        steps_json: row.get(4)?,
        source_session_id: row.get(5)?,
        success_count: row.get(6)?,
        failure_count: row.get(7)?,
        priority: row.get(8)?,
        skill_md_path: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn store_and_retrieve_learned_skill() {
        let db = test_db();
        let id = store_learned_skill(
            &db,
            "git-commit-push",
            "Commit and push changes",
            r#"["bash","git"]"#,
            r#"[{"tool":"bash","action":"git add ."},{"tool":"bash","action":"git commit -m msg"},{"tool":"bash","action":"git push"}]"#,
            Some("session-abc"),
        )
        .unwrap();
        assert!(!id.is_empty());

        let skill = get_learned_skill_by_name(&db, "git-commit-push")
            .unwrap()
            .expect("should exist");
        assert_eq!(skill.name, "git-commit-push");
        assert_eq!(skill.description, "Commit and push changes");
        assert_eq!(skill.success_count, 1);
        assert_eq!(skill.failure_count, 0);
        assert_eq!(skill.priority, 50);
        assert_eq!(skill.source_session_id.as_deref(), Some("session-abc"));
    }

    #[test]
    fn store_duplicate_name_increments_success() {
        let db = test_db();
        store_learned_skill(&db, "deploy", "Deploy app", "[]", "[]", None).unwrap();
        store_learned_skill(&db, "deploy", "Deploy v2", "[]", "[]", None).unwrap();

        let skill = get_learned_skill_by_name(&db, "deploy")
            .unwrap()
            .expect("should exist");
        // ON CONFLICT increments success_count
        assert_eq!(skill.success_count, 2);
        // Description updated to latest
        assert_eq!(skill.description, "Deploy v2");
    }

    #[test]
    fn list_learned_skills_ordered_by_priority() {
        let db = test_db();
        store_learned_skill(&db, "low-pri", "Low", "[]", "[]", None).unwrap();
        store_learned_skill(&db, "high-pri", "High", "[]", "[]", None).unwrap();
        update_learned_skill_priority(&db, "high-pri", 90).unwrap();
        update_learned_skill_priority(&db, "low-pri", 10).unwrap();

        let skills = list_learned_skills(&db, 10).unwrap();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "high-pri");
        assert_eq!(skills[1].name, "low-pri");
    }

    #[test]
    fn record_success_and_failure() {
        let db = test_db();
        store_learned_skill(&db, "test-skill", "Test", "[]", "[]", None).unwrap();

        record_learned_skill_success(&db, "test-skill").unwrap();
        record_learned_skill_success(&db, "test-skill").unwrap();
        record_learned_skill_failure(&db, "test-skill").unwrap();

        let skill = get_learned_skill_by_name(&db, "test-skill")
            .unwrap()
            .expect("should exist");
        // Initial 1 + 2 more successes
        assert_eq!(skill.success_count, 3);
        assert_eq!(skill.failure_count, 1);
    }

    #[test]
    fn count_learned_skills_empty_and_populated() {
        let db = test_db();
        assert_eq!(count_learned_skills(&db).unwrap(), 0);

        store_learned_skill(&db, "skill-a", "A", "[]", "[]", None).unwrap();
        store_learned_skill(&db, "skill-b", "B", "[]", "[]", None).unwrap();
        assert_eq!(count_learned_skills(&db).unwrap(), 2);
    }

    #[test]
    fn update_priority() {
        let db = test_db();
        store_learned_skill(&db, "pri-test", "Priority", "[]", "[]", None).unwrap();

        update_learned_skill_priority(&db, "pri-test", 75).unwrap();
        let skill = get_learned_skill_by_name(&db, "pri-test")
            .unwrap()
            .expect("should exist");
        assert_eq!(skill.priority, 75);
    }

    #[test]
    fn set_skill_md_path() {
        let db = test_db();
        store_learned_skill(&db, "md-test", "MD", "[]", "[]", None).unwrap();

        set_learned_skill_md_path(&db, "md-test", "/skills/learned/md-test.md").unwrap();
        let skill = get_learned_skill_by_name(&db, "md-test")
            .unwrap()
            .expect("should exist");
        assert_eq!(
            skill.skill_md_path.as_deref(),
            Some("/skills/learned/md-test.md")
        );
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let db = test_db();
        assert!(get_learned_skill_by_name(&db, "nope").unwrap().is_none());
    }

    #[test]
    fn list_respects_limit() {
        let db = test_db();
        for i in 0..5 {
            store_learned_skill(&db, &format!("s-{i}"), "desc", "[]", "[]", None).unwrap();
        }
        let skills = list_learned_skills(&db, 3).unwrap();
        assert_eq!(skills.len(), 3);
    }
}
