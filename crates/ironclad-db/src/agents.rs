use ironclad_core::{IroncladError, Result};
use std::collections::HashMap;

use crate::Database;

#[derive(Debug, Clone)]
pub struct SubAgentRow {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub model: String,
    pub fallback_models_json: Option<String>,
    pub role: String,
    pub description: Option<String>,
    pub skills_json: Option<String>,
    pub enabled: bool,
    pub session_count: i64,
}

pub fn upsert_sub_agent(db: &Database, agent: &SubAgentRow) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO sub_agents (id, name, display_name, model, fallback_models_json, role, description, skills_json, enabled, session_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(name) DO UPDATE SET
           display_name = excluded.display_name,
           model = excluded.model,
           fallback_models_json = excluded.fallback_models_json,
           role = excluded.role,
           description = excluded.description,
           skills_json = excluded.skills_json,
           enabled = excluded.enabled,
           session_count = excluded.session_count",
        rusqlite::params![
            agent.id,
            agent.name,
            agent.display_name,
            agent.model,
            agent.fallback_models_json,
            agent.role,
            agent.description,
            agent.skills_json,
            agent.enabled as i32,
            agent.session_count,
        ],
    )
    .map_err(|e| IroncladError::Database(format!("upsert sub_agent: {e}")))?;
    Ok(())
}

pub fn list_sub_agents(db: &Database) -> Result<Vec<SubAgentRow>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, display_name, model, fallback_models_json, role, description, skills_json, enabled, session_count
             FROM sub_agents ORDER BY name",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SubAgentRow {
                id: row.get(0)?,
                name: row.get(1)?,
                display_name: row.get(2)?,
                model: row.get(3)?,
                fallback_models_json: row.get(4)?,
                role: row.get(5)?,
                description: row.get(6)?,
                skills_json: row.get(7)?,
                enabled: row.get::<_, i32>(8)? != 0,
                session_count: row.get(9)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(rows)
}

pub fn list_enabled_sub_agents(db: &Database) -> Result<Vec<SubAgentRow>> {
    let all = list_sub_agents(db)?;
    Ok(all.into_iter().filter(|a| a.enabled).collect())
}

pub fn list_session_counts_by_agent(db: &Database) -> Result<HashMap<String, i64>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT agent_id, COUNT(*) FROM sessions GROUP BY agent_id")
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            let agent_id: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((agent_id, count))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(rows.into_iter().collect())
}

pub fn delete_sub_agent(db: &Database, name: &str) -> Result<bool> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM sub_agents WHERE name = ?1",
            rusqlite::params![name],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(deleted > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    fn sample_agent(name: &str) -> SubAgentRow {
        SubAgentRow {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            display_name: Some(name.replace('-', " ")),
            model: "test-model".into(),
            fallback_models_json: Some("[]".into()),
            role: "specialist".into(),
            description: Some("Test agent".into()),
            skills_json: None,
            enabled: true,
            session_count: 0,
        }
    }

    #[test]
    fn upsert_and_list() {
        let db = test_db();
        upsert_sub_agent(&db, &sample_agent("alpha")).unwrap();
        upsert_sub_agent(&db, &sample_agent("bravo")).unwrap();
        let agents = list_sub_agents(&db).unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].name, "alpha");
        assert_eq!(agents[1].name, "bravo");
    }

    #[test]
    fn upsert_updates_existing() {
        let db = test_db();
        let mut agent = sample_agent("alpha");
        upsert_sub_agent(&db, &agent).unwrap();
        agent.model = "updated-model".into();
        agent.session_count = 42;
        upsert_sub_agent(&db, &agent).unwrap();
        let agents = list_sub_agents(&db).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].model, "updated-model");
        assert_eq!(agents[0].session_count, 42);
    }

    #[test]
    fn list_enabled_filters() {
        let db = test_db();
        let mut a = sample_agent("enabled-one");
        upsert_sub_agent(&db, &a).unwrap();
        a = sample_agent("disabled-one");
        a.enabled = false;
        upsert_sub_agent(&db, &a).unwrap();
        let enabled = list_enabled_sub_agents(&db).unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "enabled-one");
    }

    #[test]
    fn delete_works() {
        let db = test_db();
        upsert_sub_agent(&db, &sample_agent("doomed")).unwrap();
        assert!(delete_sub_agent(&db, "doomed").unwrap());
        assert!(!delete_sub_agent(&db, "doomed").unwrap());
        assert!(list_sub_agents(&db).unwrap().is_empty());
    }

    #[test]
    fn session_counts_by_agent_reads_sessions_table() {
        let db = test_db();
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES (?1, ?2, 'agent', 'active')",
                rusqlite::params!["s1", "alpha"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES (?1, ?2, 'agent', 'ended')",
                rusqlite::params!["s2", "alpha"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES (?1, ?2, 'agent', 'active')",
                rusqlite::params!["s3", "bravo"],
            )
            .unwrap();
        }

        let counts = list_session_counts_by_agent(&db).unwrap();
        assert_eq!(counts.get("alpha"), Some(&2));
        assert_eq!(counts.get("bravo"), Some(&1));
    }
}
