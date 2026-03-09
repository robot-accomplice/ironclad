use crate::Database;
use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;

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
    pub risk_level: String,
    pub enabled: bool,
    pub last_loaded_at: Option<String>,
    pub created_at: String,
    pub version: String,
    pub author: String,
    pub registry_source: String,
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
    risk_level: Option<&str>,
) -> Result<String> {
    register_skill_full(
        db,
        name,
        kind,
        description,
        source_path,
        content_hash,
        triggers_json,
        tool_chain_json,
        policy_overrides_json,
        script_path,
        risk_level.unwrap_or("Caution"),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn register_skill_full(
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
    risk_level: &str,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO skills (id, name, kind, description, source_path, content_hash, \
         triggers_json, tool_chain_json, policy_overrides_json, script_path, risk_level, last_loaded_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
            risk_level,
            now,
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn get_skill(db: &Database, id: &str) -> Result<Option<SkillRecord>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, name, kind, description, source_path, content_hash, \
         triggers_json, tool_chain_json, policy_overrides_json, script_path, risk_level, \
         enabled, last_loaded_at, created_at, version, author, registry_source \
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
             triggers_json, tool_chain_json, policy_overrides_json, script_path, risk_level, \
             enabled, last_loaded_at, created_at, version, author, registry_source \
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

#[allow(clippy::too_many_arguments)]
pub fn update_skill_full(
    db: &Database,
    id: &str,
    content_hash: &str,
    triggers_json: Option<&str>,
    tool_chain_json: Option<&str>,
    policy_overrides_json: Option<&str>,
    script_path: Option<&str>,
    source_path: &str,
    risk_level: &str,
) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE skills SET content_hash = ?1, triggers_json = ?2, tool_chain_json = ?3, \
         policy_overrides_json = ?4, script_path = ?5, source_path = ?6, risk_level = ?7, \
         last_loaded_at = datetime('now') WHERE id = ?8",
        rusqlite::params![
            content_hash,
            triggers_json,
            tool_chain_json,
            policy_overrides_json,
            script_path,
            source_path,
            risk_level,
            id
        ],
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
    let escaped = keyword.replace('%', "\\%").replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let mut stmt = conn
        .prepare(
            "SELECT id, name, kind, description, source_path, content_hash, \
             triggers_json, tool_chain_json, policy_overrides_json, script_path, risk_level, \
             enabled, last_loaded_at, created_at, version, author, registry_source \
             FROM skills WHERE triggers_json LIKE ?1 ESCAPE '\\' AND enabled = 1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([&pattern], row_to_skill)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn find_enabled_skill_by_script_path(
    db: &Database,
    script_path: &str,
) -> Result<Option<SkillRecord>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, name, kind, description, source_path, content_hash, \
         triggers_json, tool_chain_json, policy_overrides_json, script_path, risk_level, \
         enabled, last_loaded_at, created_at, version, author, registry_source \
         FROM skills WHERE script_path = ?1 AND enabled = 1 LIMIT 1",
        [script_path],
        row_to_skill,
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn find_skill_by_script_path(db: &Database, script_path: &str) -> Result<Option<SkillRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, kind, description, source_path, content_hash, \
             triggers_json, tool_chain_json, policy_overrides_json, script_path, risk_level, \
             enabled, last_loaded_at, created_at, version, author, registry_source \
             FROM skills WHERE script_path = ?1 ORDER BY created_at DESC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([script_path], row_to_skill)
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let mut matches = rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    if matches.len() > 1 {
        return Err(IroncladError::Database(format!(
            "ambiguous script_path '{}' matches {} skills",
            script_path,
            matches.len()
        )));
    }
    Ok(matches.pop())
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
        risk_level: row.get(10)?,
        enabled: row.get::<_, i32>(11)? != 0,
        last_loaded_at: row.get(12)?,
        created_at: row.get(13)?,
        version: row.get(14)?,
        author: row.get(15)?,
        registry_source: row.get(16)?,
    })
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
            &db,
            "s1",
            "structured",
            None,
            "/s1.toml",
            "h1",
            None,
            None,
            None,
            None,
            None,
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
            None,
        )
        .unwrap();

        let results = find_by_trigger(&db, "deploy").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "deploy");
    }

    #[test]
    fn get_skill_nonexistent_returns_none() {
        let db = test_db();
        assert!(get_skill(&db, "no-such-id").unwrap().is_none());
    }

    #[test]
    fn list_skills_empty_db() {
        let db = test_db();
        let skills = list_skills(&db).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn find_by_trigger_no_matches() {
        let db = test_db();
        register_skill(
            &db,
            "s1",
            "structured",
            None,
            "/s1.toml",
            "h",
            Some(r#"{"keywords":["deploy"]}"#),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let results = find_by_trigger(&db, "nonexistent-keyword").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn find_by_trigger_excludes_disabled() {
        let db = test_db();
        let id = register_skill(
            &db,
            "deploy",
            "structured",
            None,
            "/d.toml",
            "h",
            Some(r#"{"keywords":["deploy"]}"#),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let results = find_by_trigger(&db, "deploy").unwrap();
        assert_eq!(results.len(), 1);

        toggle_skill_enabled(&db, &id).unwrap();
        let results = find_by_trigger(&db, "deploy").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn delete_skill_nonexistent_is_noop() {
        let db = test_db();
        delete_skill(&db, "no-such-id").unwrap();
    }

    #[test]
    fn register_skill_all_optional_fields() {
        let db = test_db();
        let id = register_skill(
            &db,
            "full-skill",
            "scripted",
            Some("A fully populated skill"),
            "/skills/full.toml",
            "deadbeef",
            Some(r#"{"keywords":["full","test"]}"#),
            Some(r#"["tool_a","tool_b"]"#),
            Some(r#"{"allow_web":true}"#),
            Some("/scripts/full.sh"),
            None,
        )
        .unwrap();

        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert_eq!(skill.name, "full-skill");
        assert_eq!(skill.kind, "scripted");
        assert_eq!(
            skill.description.as_deref(),
            Some("A fully populated skill")
        );
        assert!(skill.triggers_json.unwrap().contains("full"));
        assert!(skill.tool_chain_json.unwrap().contains("tool_a"));
        assert_eq!(skill.script_path.as_deref(), Some("/scripts/full.sh"));
    }

    #[test]
    fn update_skill_sets_last_loaded_at() {
        let db = test_db();
        let id = register_skill(
            &db,
            "s1",
            "structured",
            None,
            "/s1.toml",
            "old",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let skill = get_skill(&db, &id).unwrap().unwrap();
        // last_loaded_at is set during registration
        assert!(skill.last_loaded_at.is_some(), "set on register");

        update_skill(&db, &id, "new", None, None).unwrap();
        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert!(skill.last_loaded_at.is_some(), "still set after update");
        assert_eq!(skill.content_hash, "new");
    }

    #[test]
    fn register_skill_full_persists_risk_level() {
        let db = test_db();
        let id = register_skill_full(
            &db,
            "high-risk",
            "structured",
            Some("dangerous operation"),
            "/skills/high-risk.toml",
            "h-risk",
            Some(r#"{"keywords":["danger"]}"#),
            None,
            Some(r#"{"require_creator":true}"#),
            Some("/skills/bin/high-risk.sh"),
            "Dangerous",
        )
        .unwrap();
        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert_eq!(skill.risk_level, "Dangerous");
    }

    #[test]
    fn find_skill_by_script_path_rejects_ambiguous_duplicates() {
        let db = test_db();
        register_skill_full(
            &db,
            "dup-a",
            "structured",
            None,
            "/skills/dup-a.toml",
            "h-a",
            None,
            None,
            None,
            Some("/skills/bin/dup.sh"),
            "Caution",
        )
        .unwrap();
        register_skill_full(
            &db,
            "dup-b",
            "structured",
            None,
            "/skills/dup-b.toml",
            "h-b",
            None,
            None,
            None,
            Some("/skills/bin/dup.sh"),
            "Caution",
        )
        .unwrap();

        let err = find_skill_by_script_path(&db, "/skills/bin/dup.sh")
            .expect_err("duplicate script paths must fail closed");
        assert!(err.to_string().contains("ambiguous script_path"));
    }

    #[test]
    fn update_skill_full_updates_all_fields() {
        let db = test_db();
        let id = register_skill(
            &db,
            "updatable",
            "structured",
            Some("original"),
            "/old/path.toml",
            "old-hash",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        update_skill_full(
            &db,
            &id,
            "new-hash",
            Some(r#"{"keywords":["updated"]}"#),
            Some(r#"["new_tool"]"#),
            Some(r#"{"policy":"strict"}"#),
            Some("/scripts/updated.sh"),
            "/new/path.toml",
            "Dangerous",
        )
        .unwrap();

        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert_eq!(skill.content_hash, "new-hash");
        assert!(skill.triggers_json.unwrap().contains("updated"));
        assert!(skill.tool_chain_json.unwrap().contains("new_tool"));
        assert!(skill.policy_overrides_json.unwrap().contains("strict"));
        assert_eq!(skill.script_path.as_deref(), Some("/scripts/updated.sh"));
        assert_eq!(skill.source_path, "/new/path.toml");
        assert_eq!(skill.risk_level, "Dangerous");
        assert!(
            skill.last_loaded_at.is_some(),
            "last_loaded_at should be set"
        );
    }

    #[test]
    fn update_skill_full_clears_optional_fields() {
        let db = test_db();
        let id = register_skill(
            &db,
            "clearable",
            "scripted",
            Some("has everything"),
            "/path.toml",
            "hash",
            Some(r#"{"keywords":["test"]}"#),
            Some(r#"["tool"]"#),
            Some(r#"{"p":true}"#),
            Some("/scripts/test.sh"),
            None,
        )
        .unwrap();

        update_skill_full(
            &db,
            &id,
            "hash2",
            None,
            None,
            None,
            None,
            "/path.toml",
            "Safe",
        )
        .unwrap();

        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert!(skill.triggers_json.is_none());
        assert!(skill.tool_chain_json.is_none());
        assert!(skill.policy_overrides_json.is_none());
        assert!(skill.script_path.is_none());
        assert_eq!(skill.risk_level, "Safe");
    }

    #[test]
    fn find_enabled_skill_by_script_path_found() {
        let db = test_db();
        register_skill_full(
            &db,
            "scripted-skill",
            "scripted",
            None,
            "/skills/scripted.toml",
            "hash",
            None,
            None,
            None,
            Some("/scripts/my_script.sh"),
            "Caution",
        )
        .unwrap();

        let found = find_enabled_skill_by_script_path(&db, "/scripts/my_script.sh")
            .unwrap()
            .unwrap();
        assert_eq!(found.name, "scripted-skill");
    }

    #[test]
    fn find_enabled_skill_by_script_path_not_found() {
        let db = test_db();
        let result = find_enabled_skill_by_script_path(&db, "/no/such/path.sh").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_enabled_skill_by_script_path_excludes_disabled() {
        let db = test_db();
        let id = register_skill_full(
            &db,
            "disabled-scripted",
            "scripted",
            None,
            "/skills/disabled.toml",
            "hash",
            None,
            None,
            None,
            Some("/scripts/disabled.sh"),
            "Caution",
        )
        .unwrap();

        // Disable the skill
        toggle_skill_enabled(&db, &id).unwrap();

        let result = find_enabled_skill_by_script_path(&db, "/scripts/disabled.sh").unwrap();
        assert!(result.is_none(), "disabled skill should not be found");
    }

    #[test]
    fn find_skill_by_script_path_single_match() {
        let db = test_db();
        register_skill_full(
            &db,
            "unique-script",
            "scripted",
            None,
            "/skills/unique.toml",
            "hash",
            None,
            None,
            None,
            Some("/scripts/unique.sh"),
            "Caution",
        )
        .unwrap();

        let found = find_skill_by_script_path(&db, "/scripts/unique.sh")
            .unwrap()
            .unwrap();
        assert_eq!(found.name, "unique-script");
    }

    #[test]
    fn find_skill_by_script_path_no_match() {
        let db = test_db();
        let result = find_skill_by_script_path(&db, "/no/script.sh").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_by_trigger_escapes_special_like_chars() {
        let db = test_db();
        register_skill(
            &db,
            "special-trigger",
            "structured",
            None,
            "/s.toml",
            "h",
            Some(r#"{"keywords":["100%_match","under_score"]}"#),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Search for literal "%" -- should find the skill with "100%_match"
        let results = find_by_trigger(&db, "100%").unwrap();
        assert_eq!(results.len(), 1);

        // Search for literal "_" -- should find the skill with "under_score"
        let results = find_by_trigger(&db, "under_score").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn register_skill_defaults_to_caution_risk() {
        let db = test_db();
        let id = register_skill(
            &db,
            "default-risk",
            "structured",
            None,
            "/s.toml",
            "h",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let skill = get_skill(&db, &id).unwrap().unwrap();
        assert_eq!(skill.risk_level, "Caution");
    }
}
