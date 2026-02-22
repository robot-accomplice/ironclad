use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub source_path: String,
    pub content_hash: String,
    pub triggers_json: Option<String>,
    pub tool_chain_json: Option<String>,
    pub policy_overrides_json: Option<String>,
    pub script_path: Option<String>,
    pub enabled: bool,
    pub last_loaded_at: Option<String>,
    pub created_at: String,
}

#[allow(clippy::too_many_arguments)]
pub fn register_skill(
    db: &Database,
    name: &str,
    kind: &str,
    description: Option<&str>,
    source_path: &str,
    content_hash: &str,
    triggers_json: Option<&str>,
    tool_chain_json: Option<&str>,
    policy_overrides_json: Option<&str>,
    script_path: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO skills (id, name, kind, description, source_path, content_hash, \
         triggers_json, tool_chain_json, policy_overrides_json, script_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id,
            name,
            kind,
            description,
            source_path,
            content_hash,
            triggers_json,
            tool_chain_json,
            policy_overrides_json,
            script_path,
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn get_skill(db: &Database, id: &str) -> Result<Option<SkillRecord>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, name, kind, description, source_path, content_hash, \
         triggers_json, tool_chain_json, policy_overrides_json, script_path, \
         enabled, last_loaded_at, created_at \
         FROM skills WHERE id = ?1",
        [id],
        row_to_skill,
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn list_skills(db: &Database) -> Result<Vec<SkillRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, kind, description, source_path, content_hash, \
             triggers_json, tool_chain_json, policy_overrides_json, script_path, \
             enabled, last_loaded_at, created_at \
             FROM skills ORDER BY name ASC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], row_to_skill)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn update_skill(
    db: &Database,
    id: &str,
    content_hash: &str,
    triggers_json: Option<&str>,
    tool_chain_json: Option<&str>,
) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE skills SET content_hash = ?1, triggers_json = ?2, tool_chain_json = ?3, \
         last_loaded_at = datetime('now') WHERE id = ?4",
        rusqlite::params![content_hash, triggers_json, tool_chain_json, id],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn delete_skill(db: &Database, id: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute("DELETE FROM skills WHERE id = ?1", [id])
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn toggle_skill_enabled(db: &Database, id: &str) -> Result<Option<bool>> {
    let conn = db.conn();
    let current: Option<i32> = conn
        .query_row("SELECT enabled FROM skills WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    match current {
        Some(val) => {
            let new_val = if val != 0 { 0 } else { 1 };
            conn.execute(
                "UPDATE skills SET enabled = ?1 WHERE id = ?2",
                rusqlite::params![new_val, id],
            )
            .map_err(|e| IroncladError::Database(e.to_string()))?;
            Ok(Some(new_val != 0))
        }
        None => Ok(None),
    }
}

/// Searches skills whose `triggers_json` contains the given keyword (case-insensitive).
pub fn find_by_trigger(db: &Database, keyword: &str) -> Result<Vec<SkillRecord>> {
    let conn = db.conn();
    let pattern = format!("%{keyword}%");
    let mut stmt = conn
        .prepare(
            "SELECT id, name, kind, description, source_path, content_hash, \
             triggers_json, tool_chain_json, policy_overrides_json, script_path, \
             enabled, last_loaded_at, created_at \
             FROM skills WHERE triggers_json LIKE ?1 AND enabled = 1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([&pattern], row_to_skill)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

fn row_to_skill(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillRecord> {
    Ok(SkillRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        description: row.get(3)?,
        source_path: row.get(4)?,
        content_hash: row.get(5)?,
        triggers_json: row.get(6)?,
        tool_chain_json: row.get(7)?,
        policy_overrides_json: row.get(8)?,
        script_path: row.get(9)?,
        enabled: row.get::<_, i32>(10)? != 0,
        last_loaded_at: row.get(11)?,
        created_at: row.get(12)?,
    })
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
    fn register_and_get_skill() {
        let db = test_db();
        let id = register_skill(
            &db,
            "git-commit",
            "structured",
            Some("Auto-commit helper"),
            "/skills/git-commit.toml",
            "abc123",
            Some(r#"{"keywords":["commit","git"]}"#),
            None,
            None,
            None,
        )
        .unwrap();

        let skill = get_skill(&db, &id).unwrap().expect("skill should exist");
        assert_eq!(skill.name, "git-commit");
        assert_eq!(skill.kind, "structured");
        assert!(skill.enabled);
    }

    #[test]
    fn list_and_delete_skills() {
        let db = test_db();
        let id = register_skill(
            &db,
            "s1",
            "instruction",
            None,
            "/s1.toml",
            "h1",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        register_skill(
            &db,
            "s2",
            "structured",
            None,
            "/s2.toml",
            "h2",
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(list_skills(&db).unwrap().len(), 2);

        delete_skill(&db, &id).unwrap();
        assert_eq!(list_skills(&db).unwrap().len(), 1);
    }

    #[test]
    fn update_skill_changes_hash() {
        let db = test_db();
        let id = register_skill(
            &db,
            "s1",
            "structured",
            None,
            "/s1.toml",
            "old-hash",
            None,
            None,
            None,
            None,
        )
        .unwrap();

        update_skill(
            &db,
            &id,
            "new-hash",
            Some(r#"{"keywords":["deploy"]}"#),
            None,
        )
        .unwrap();
        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert_eq!(skill.content_hash, "new-hash");
        assert!(skill.triggers_json.unwrap().contains("deploy"));
    }

    #[test]
    fn toggle_skill_enabled_flips_value() {
        let db = test_db();
        let id = register_skill(
            &db, "s1", "structured", None, "/s1.toml", "h1", None, None, None, None,
        )
        .unwrap();

        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert!(skill.enabled);

        let new_val = toggle_skill_enabled(&db, &id).unwrap();
        assert_eq!(new_val, Some(false));

        let new_val = toggle_skill_enabled(&db, &id).unwrap();
        assert_eq!(new_val, Some(true));

        assert_eq!(toggle_skill_enabled(&db, "nonexistent").unwrap(), None);
    }

    #[test]
    fn find_by_trigger_keyword() {
        let db = test_db();
        register_skill(
            &db,
            "deploy",
            "structured",
            None,
            "/d.toml",
            "h",
            Some(r#"{"keywords":["deploy","ship"]}"#),
            None,
            None,
            None,
        )
        .unwrap();
        register_skill(
            &db,
            "test",
            "structured",
            None,
            "/t.toml",
            "h",
            Some(r#"{"keywords":["test","ci"]}"#),
            None,
            None,
            None,
        )
        .unwrap();

        let results = find_by_trigger(&db, "deploy").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "deploy");
    }
}
