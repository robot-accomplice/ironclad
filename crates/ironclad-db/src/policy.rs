use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct PolicyRecord {
    pub id: String,
    pub turn_id: Option<String>,
    pub tool_name: String,
    pub decision: String,
    pub rule_name: Option<String>,
    pub reason: Option<String>,
    pub context_json: Option<String>,
    pub created_at: String,
}

pub fn record_policy_decision(
    db: &Database,
    turn_id: Option<&str>,
    tool_name: &str,
    decision: &str,
    rule_name: Option<&str>,
    reason: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO policy_decisions (id, turn_id, tool_name, decision, rule_name, reason) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, turn_id, tool_name, decision, rule_name, reason],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn get_decisions_for_turn(db: &Database, turn_id: &str) -> Result<Vec<PolicyRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id, tool_name, decision, rule_name, reason, context_json, created_at \
             FROM policy_decisions WHERE turn_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([turn_id], |row| {
            Ok(PolicyRecord {
                id: row.get(0)?,
                turn_id: row.get(1)?,
                tool_name: row.get(2)?,
                decision: row.get(3)?,
                rule_name: row.get(4)?,
                reason: row.get(5)?,
                context_json: row.get(6)?,
                created_at: row.get(7)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn record_and_retrieve_decision() {
        let db = test_db();
        // policy_decisions.turn_id is nullable — no FK seed needed
        let id = record_policy_decision(
            &db,
            Some("turn-1"),
            "bash",
            "deny",
            Some("no_rm_rf"),
            Some("destructive command"),
        )
        .unwrap();
        assert!(!id.is_empty());

        let decisions = get_decisions_for_turn(&db, "turn-1").unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].decision, "deny");
        assert_eq!(decisions[0].rule_name.as_deref(), Some("no_rm_rf"));
    }

    #[test]
    fn empty_turn_returns_empty_vec() {
        let db = test_db();
        let decisions = get_decisions_for_turn(&db, "no-such-turn").unwrap();
        assert!(decisions.is_empty());
    }

    #[test]
    fn multiple_decisions_per_turn() {
        let db = test_db();
        record_policy_decision(&db, Some("t1"), "bash", "allow", None, None).unwrap();
        record_policy_decision(
            &db,
            Some("t1"),
            "write_file",
            "deny",
            Some("readonly"),
            Some("read-only mode"),
        )
        .unwrap();

        let decisions = get_decisions_for_turn(&db, "t1").unwrap();
        assert_eq!(decisions.len(), 2);
    }

    #[test]
    fn record_with_no_turn_id() {
        let db = test_db();
        let id = record_policy_decision(&db, None, "search", "allow", None, None).unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn record_all_optional_none() {
        let db = test_db();
        let id = record_policy_decision(&db, None, "tool", "allow", None, None).unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn decision_fields_populated() {
        let db = test_db();
        record_policy_decision(
            &db,
            Some("t2"),
            "exec",
            "escalate",
            Some("human_review"),
            Some("needs approval"),
        )
        .unwrap();
        let decisions = get_decisions_for_turn(&db, "t2").unwrap();
        assert_eq!(decisions[0].tool_name, "exec");
        assert_eq!(decisions[0].decision, "escalate");
        assert_eq!(decisions[0].rule_name.as_deref(), Some("human_review"));
        assert_eq!(decisions[0].reason.as_deref(), Some("needs approval"));
        assert!(!decisions[0].id.is_empty());
        assert!(!decisions[0].created_at.is_empty());
    }
}
