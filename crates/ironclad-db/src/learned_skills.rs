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

/// Insert a new learned skill.  Uses `ON CONFLICT(name)` to upsert if the
/// skill already exists (idempotent re-learning).
///
/// On conflict the description/tools/steps are updated but `success_count`
/// is NOT incremented here — the caller (governor) handles reinforcement
/// via [`record_learned_skill_success`] to avoid double-counting.
///
/// Returns the persisted row's `id` (which may differ from a freshly
/// generated UUID when the upsert takes the conflict path).
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
             (id, name, description, trigger_tools, steps_json, source_session_id, success_count, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7) \
         ON CONFLICT(name) DO UPDATE SET \
             description      = excluded.description, \
             trigger_tools    = excluded.trigger_tools, \
             steps_json       = excluded.steps_json, \
             source_session_id = excluded.source_session_id, \
             updated_at       = ?7",
        rusqlite::params![id, name, description, trigger_tools, steps_json, source_session_id, now],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    // Return the actual persisted id (the upsert may have kept the original row's id).
    let persisted_id: String = conn
        .query_row(
            "SELECT id FROM learned_skills WHERE name = ?1",
            [name],
            |r| r.get(0),
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(persisted_id)
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

// ── Hygiene ─────────────────────────────────────────────────────

/// Find learned skills whose priority has decayed to or below `threshold`.
///
/// Pass `threshold = 0` for the default behaviour (find only fully-dead
/// skills).  Raise the threshold to be more aggressive about culling
/// low-value procedures.
///
/// Returns the names and `skill_md_path`s of matching rows.  The caller
/// should delete the corresponding `.md` files **first**, then call
/// [`delete_learned_skills_by_names`] to remove the DB rows — this ordering
/// ensures a crash between the two steps never leaves orphan `.md` files
/// (the DB rows survive and will be pruned on the next cycle).
pub fn find_dead_learned_skills(
    db: &Database,
    threshold: i64,
) -> Result<Vec<(String, Option<String>)>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name, skill_md_path FROM learned_skills WHERE priority <= ?1")
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let dead: Vec<(String, Option<String>)> = stmt
        .query_map([threshold], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .filter_map(|r| {
            r.inspect_err(|e| tracing::warn!("skipping corrupted learned_skills row: {e}"))
                .ok()
        })
        .collect();

    Ok(dead)
}

/// Delete learned skills by name.  Intended to be called **after** the
/// caller has removed the corresponding `.md` files from disk.
pub fn delete_learned_skills_by_names(db: &Database, names: &[String]) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    let conn = db.conn();
    let placeholders: Vec<String> = (1..=names.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "DELETE FROM learned_skills WHERE name IN ({})",
        placeholders.join(", ")
    );
    let params: Vec<&dyn rusqlite::types::ToSql> = names
        .iter()
        .map(|n| n as &dyn rusqlite::types::ToSql)
        .collect();
    conn.execute(&sql, params.as_slice())
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

/// Convenience wrapper that finds and deletes dead skills in one call.
///
/// **WARNING:** This only removes DB rows. It does NOT delete the `.md` files
/// referenced by `skill_md_path`. Callers MUST iterate the returned paths and
/// remove the files themselves. Prefer [`find_dead_learned_skills`] +
/// file cleanup + [`delete_learned_skills_by_names`] for the two-phase pattern.
pub fn prune_dead_learned_skills(
    db: &Database,
    threshold: i64,
) -> Result<Vec<(String, Option<String>)>> {
    let dead = find_dead_learned_skills(db, threshold)?;
    if !dead.is_empty() {
        let names: Vec<String> = dead.iter().map(|(n, _)| n.clone()).collect();
        delete_learned_skills_by_names(db, &names)?;
    }
    Ok(dead)
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
        // I4 fix: new skills start at success_count = 0 to avoid phantom inflation
        assert_eq!(skill.success_count, 0);
        assert_eq!(skill.failure_count, 0);
        assert_eq!(skill.priority, 50);
        assert_eq!(skill.source_session_id.as_deref(), Some("session-abc"));
    }

    #[test]
    fn store_duplicate_name_upserts_without_double_counting() {
        let db = test_db();
        let id1 =
            store_learned_skill(&db, "deploy", "Deploy app", "[]", "[]", Some("sess-1")).unwrap();
        let id2 =
            store_learned_skill(&db, "deploy", "Deploy v2", "[]", "[]", Some("sess-2")).unwrap();

        let skill = get_learned_skill_by_name(&db, "deploy")
            .unwrap()
            .expect("should exist");
        // ON CONFLICT does NOT increment success_count — the governor handles
        // reinforcement separately via record_learned_skill_success.
        assert_eq!(skill.success_count, 0);
        // Description updated to latest
        assert_eq!(skill.description, "Deploy v2");
        // source_session_id updated to latest session
        assert_eq!(skill.source_session_id.as_deref(), Some("sess-2"));
        // Both calls return the same persisted row id
        assert_eq!(id1, id2);
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
        // Initial 0 + 2 explicit successes (I4 fix: no phantom +1 at creation)
        assert_eq!(skill.success_count, 2);
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

    #[test]
    fn prune_dead_learned_skills_removes_zero_priority() {
        let db = test_db();
        store_learned_skill(&db, "alive", "Alive", "[]", "[]", None).unwrap();
        store_learned_skill(&db, "dead", "Dead", "[]", "[]", None).unwrap();
        set_learned_skill_md_path(&db, "dead", "/tmp/dead.md").unwrap();
        update_learned_skill_priority(&db, "dead", 0).unwrap();

        let pruned = prune_dead_learned_skills(&db, 0).unwrap();
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].0, "dead");
        assert_eq!(pruned[0].1.as_deref(), Some("/tmp/dead.md"));

        // Only "alive" remains
        assert_eq!(count_learned_skills(&db).unwrap(), 1);
        assert!(get_learned_skill_by_name(&db, "alive").unwrap().is_some());
        assert!(get_learned_skill_by_name(&db, "dead").unwrap().is_none());
    }

    #[test]
    fn prune_dead_learned_skills_empty_is_noop() {
        let db = test_db();
        store_learned_skill(&db, "healthy", "OK", "[]", "[]", None).unwrap();
        let pruned = prune_dead_learned_skills(&db, 0).unwrap();
        assert!(pruned.is_empty());
        assert_eq!(count_learned_skills(&db).unwrap(), 1);
    }
}
