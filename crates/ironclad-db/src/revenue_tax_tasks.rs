use crate::Database;
use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct RevenueTaxTaskRecord {
    pub id: String,
    pub opportunity_id: String,
    pub title: String,
    pub status: String,
    pub source_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list_revenue_tax_tasks(db: &Database, limit: usize) -> Result<Vec<Value>> {
    let conn = db.conn();
    let limit = limit.clamp(1, 200) as i64;
    let mut stmt = conn
        .prepare(
            "SELECT id, title, status, source, created_at, updated_at \
             FROM tasks \
             WHERE lower(COALESCE(source, '')) LIKE '%\"type\":\"revenue_tax_payout\"%' \
             ORDER BY updated_at DESC, created_at DESC LIMIT ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([limit], |row| {
            let id: String = row.get(0)?;
            let source_json: Option<String> = row.get(3)?;
            let source_value = source_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            Ok(json!({
                "id": id,
                "opportunity_id": id.strip_prefix("rev_tax:").unwrap_or(&id),
                "title": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "source": source_value,
                "created_at": row.get::<_, String>(4)?,
                "updated_at": row.get::<_, String>(5)?,
            }))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn get_revenue_tax_task(
    db: &Database,
    opportunity_id: &str,
) -> Result<Option<RevenueTaxTaskRecord>> {
    let conn = db.conn();
    let task_id = format!("rev_tax:{opportunity_id}");
    conn.query_row(
        "SELECT id, title, status, source, created_at, updated_at FROM tasks WHERE id = ?1",
        [task_id.as_str()],
        |row| {
            Ok(RevenueTaxTaskRecord {
                id: row.get(0)?,
                opportunity_id: opportunity_id.to_string(),
                title: row.get(1)?,
                status: row.get(2)?,
                source_json: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn mark_revenue_tax_in_progress(db: &Database, opportunity_id: &str) -> Result<bool> {
    let conn = db.conn();
    let task_id = format!("rev_tax:{opportunity_id}");
    let updated = conn
        .execute(
            "UPDATE tasks SET status = 'in_progress', updated_at = datetime('now') WHERE id = ?1 AND status = 'pending'",
            [task_id.as_str()],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

pub fn mark_revenue_tax_failed(db: &Database, opportunity_id: &str, reason: &str) -> Result<bool> {
    update_revenue_tax_status(
        db,
        opportunity_id,
        "failed",
        Some(reason),
        None,
        &["pending", "in_progress"],
    )
}

pub fn mark_revenue_tax_confirmed(
    db: &Database,
    opportunity_id: &str,
    tx_hash: &str,
) -> Result<bool> {
    update_revenue_tax_status(
        db,
        opportunity_id,
        "completed",
        None,
        Some(tx_hash),
        &["in_progress"],
    )
}

pub fn mark_revenue_tax_submitted(
    db: &Database,
    opportunity_id: &str,
    tx_hash: &str,
) -> Result<bool> {
    update_revenue_tax_status(
        db,
        opportunity_id,
        "in_progress",
        None,
        Some(tx_hash),
        &["in_progress"],
    )
}

fn update_revenue_tax_status(
    db: &Database,
    opportunity_id: &str,
    status: &str,
    failure_reason: Option<&str>,
    tx_hash: Option<&str>,
    allowed_from_statuses: &[&str],
) -> Result<bool> {
    let conn = db.conn();
    let task_id = format!("rev_tax:{opportunity_id}");
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT source, status FROM tasks WHERE id = ?1",
            [task_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let Some((existing_source_json, current_status)) = existing else {
        return Ok(false);
    };
    if !allowed_from_statuses
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&current_status))
    {
        return Ok(false);
    }
    let mut source_value = serde_json::from_str::<Value>(&existing_source_json).unwrap_or_else(
        |_| json!({ "type": "revenue_tax_payout", "opportunity_id": opportunity_id }),
    );
    if let Some(obj) = source_value.as_object_mut() {
        obj.insert("status".into(), json!(status));
        if let Some(reason) = failure_reason {
            obj.insert("failure_reason".into(), json!(reason));
        }
        if let Some(tx_hash) = tx_hash {
            obj.insert("tax_tx_hash".into(), json!(tx_hash));
        }
    }
    let updated = conn
        .execute(
            "UPDATE tasks SET status = ?2, source = ?3, updated_at = datetime('now') WHERE id = ?1",
            rusqlite::params![
                task_id,
                status,
                serde_json::to_string(&source_value)
                    .map_err(|e| IroncladError::Database(e.to_string()))?,
            ],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revenue_tax_task_lifecycle_updates_status_and_metadata() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO tasks (id, title, status, priority, source) VALUES ('rev_tax:ro_1','Tax payout','pending',96,'{\"type\":\"revenue_tax_payout\",\"opportunity_id\":\"ro_1\"}')",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(mark_revenue_tax_in_progress(&db, "ro_1").unwrap());
        assert!(mark_revenue_tax_confirmed(&db, "ro_1", "0xabc").unwrap());
        let row = get_revenue_tax_task(&db, "ro_1").unwrap().unwrap();
        assert_eq!(row.status, "completed");
        assert_eq!(row.opportunity_id, "ro_1");
        assert!(!row.created_at.is_empty());
    }
}
