use crate::Database;
use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

pub fn normalize_task_source_value(raw: Option<&str>) -> Value {
    let Some(raw) = raw else {
        return Value::Null;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::String(inner)) => parse_inner_or_origin(&inner),
        Ok(parsed) => parsed,
        Err(_) => parse_inner_or_origin(trimmed),
    }
}

fn parse_inner_or_origin(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
        return parsed;
    }
    if looks_like_origin(trimmed) {
        return json!({ "origin": trimmed });
    }
    Value::String(raw.to_string())
}

fn looks_like_origin(raw: &str) -> bool {
    raw.contains(':')
        && !raw.contains(' ')
        && !raw.contains('{')
        && !raw.contains('}')
        && !raw.contains('[')
        && !raw.contains(']')
}

pub fn canonical_task_source_json(raw: Option<&str>) -> Option<String> {
    let normalized = normalize_task_source_value(raw);
    if normalized.is_null() {
        None
    } else {
        serde_json::to_string(&normalized).ok()
    }
}

pub fn task_is_revenue_like(title: &str, source: &Value) -> bool {
    let title_lc = title.to_ascii_lowercase();
    if title_lc.contains("bounty:")
        || title_lc.contains("audit:")
        || title_lc.contains("self-funding")
        || title_lc.contains("monetization")
        || title_lc.contains("trading")
    {
        return true;
    }
    let haystack = source.to_string().to_ascii_lowercase();
    haystack.contains("\"type\":\"revenue\"")
        || haystack.contains("immunefi")
        || haystack.contains("bounty")
        || haystack.contains("mentat:tasks")
}

pub fn task_is_obvious_noise(title: &str, source: &Value) -> bool {
    let title_lc = title.trim().to_ascii_lowercase();
    if title_lc.is_empty() {
        return false;
    }
    let canned = [
        "what is the juice of saphoo?",
        "no, that was just a test.  thank you",
        "no, that was just a test. thank you",
    ];
    if canned.iter().any(|item| title_lc == *item) {
        return true;
    }
    let source_text = source.to_string().to_ascii_lowercase();
    source_text.contains("agentic_bot:tasks")
        && (title_lc.contains("just a test") || title_lc.contains("saphoo"))
}

pub fn normalize_task_sources_in_db(db: &Database) -> Result<i64> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT id, source FROM tasks WHERE source IS NOT NULL AND trim(source) != ''")
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let mut updated = 0i64;
    for row in rows {
        let (id, source) = row.map_err(|e| IroncladError::Database(e.to_string()))?;
        let normalized = canonical_task_source_json(Some(&source));
        if normalized.as_deref() != Some(source.trim()) {
            updated += conn
                .execute(
                    "UPDATE tasks SET source = ?2 WHERE id = ?1",
                    rusqlite::params![id, normalized],
                )
                .map_err(|e| IroncladError::Database(e.to_string()))? as i64;
        }
    }
    Ok(updated)
}

pub fn count_task_sources_needing_normalization(db: &Database) -> Result<i64> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT source FROM tasks WHERE source IS NOT NULL AND trim(source) != ''")
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let mut count = 0i64;
    for row in rows {
        let source = row.map_err(|e| IroncladError::Database(e.to_string()))?;
        let normalized = canonical_task_source_json(Some(&source));
        if normalized.as_deref() != Some(source.trim()) {
            count += 1;
        }
    }
    Ok(count)
}

pub fn classify_open_tasks(db: &Database) -> Result<(i64, i64)> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT title, source \
             FROM tasks \
             WHERE lower(status) IN ('pending','in_progress')",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let mut revenue_like = 0i64;
    let mut obvious_noise = 0i64;
    for row in rows {
        let (title, source_raw) = row.map_err(|e| IroncladError::Database(e.to_string()))?;
        let source = normalize_task_source_value(source_raw.as_deref());
        if task_is_revenue_like(&title, &source) {
            revenue_like += 1;
        }
        if task_is_obvious_noise(&title, &source) {
            obvious_noise += 1;
        }
    }
    Ok((revenue_like, obvious_noise))
}

pub fn count_stale_revenue_tasks(db: &Database) -> Result<i64> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT title, source \
             FROM tasks \
             WHERE lower(status) = 'in_progress' \
               AND datetime(COALESCE(updated_at, created_at)) < datetime('now','-24 hours')",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let mut count = 0i64;
    for row in rows {
        let (title, source_raw) = row.map_err(|e| IroncladError::Database(e.to_string()))?;
        let source = normalize_task_source_value(source_raw.as_deref());
        if task_is_revenue_like(&title, &source) && !source.to_string().contains("revenue_swap") {
            count += 1;
        }
    }
    Ok(count)
}

pub fn mark_stale_revenue_tasks_needs_review(db: &Database) -> Result<i64> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, title, source \
             FROM tasks \
             WHERE lower(status) = 'in_progress' \
               AND datetime(COALESCE(updated_at, created_at)) < datetime('now','-24 hours')",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let mut updated = 0i64;
    for row in rows {
        let (id, title, source_raw) = row.map_err(|e| IroncladError::Database(e.to_string()))?;
        let source = normalize_task_source_value(source_raw.as_deref());
        if task_is_revenue_like(&title, &source) && !source.to_string().contains("revenue_swap") {
            updated += conn
                .execute(
                    "UPDATE tasks SET status = 'needs_review', updated_at = datetime('now') WHERE id = ?1",
                    [id],
                )
                .map_err(|e| IroncladError::Database(e.to_string()))? as i64;
        }
    }
    Ok(updated)
}

pub fn dismiss_obvious_noise_tasks(db: &Database) -> Result<i64> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, title, source \
             FROM tasks \
             WHERE lower(status) IN ('pending','in_progress')",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let mut updated = 0i64;
    for row in rows {
        let (id, title, source_raw) = row.map_err(|e| IroncladError::Database(e.to_string()))?;
        let source = normalize_task_source_value(source_raw.as_deref());
        if task_is_obvious_noise(&title, &source) {
            updated += conn
                .execute(
                    "UPDATE tasks SET status = 'dismissed', updated_at = datetime('now') WHERE id = ?1",
                    [id],
                )
                .map_err(|e| IroncladError::Database(e.to_string()))? as i64;
        }
    }
    Ok(updated)
}

pub fn get_task_source(db: &Database, id: &str) -> Result<Option<Value>> {
    let conn = db.conn();
    let source = conn
        .query_row("SELECT source FROM tasks WHERE id = ?1", [id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .optional()
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .flatten();
    Ok(Some(normalize_task_source_value(source.as_deref())).filter(|v| !v.is_null()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_task_source_handles_json_string_wrapped_object() {
        let raw = "\"{\\\"origin\\\":\\\"pg:mentat:tasks\\\",\\\"metadata\\\":{\\\"type\\\":\\\"revenue\\\"}}\"";
        let normalized = normalize_task_source_value(Some(raw));
        assert_eq!(normalized["origin"], "pg:mentat:tasks");
        assert_eq!(normalized["metadata"]["type"], "revenue");
    }

    #[test]
    fn normalize_task_source_wraps_origin_strings() {
        let normalized = normalize_task_source_value(Some("pg:agentic_bot:tasks"));
        assert_eq!(normalized["origin"], "pg:agentic_bot:tasks");
    }

    #[test]
    fn repair_classifies_and_cleans_revenue_and_noise_tasks() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO tasks (id, title, status, priority, source, created_at, updated_at) VALUES \
             ('t1','Bounty: SSV Network','in_progress',85,'\"{\\\"origin\\\":\\\"pg:mentat:tasks\\\",\\\"metadata\\\":{\\\"type\\\":\\\"revenue\\\"}}\"',datetime('now','-2 days'),datetime('now','-2 days')), \
             ('t2','What is the juice of saphoo?','pending',5,'pg:agentic_bot:tasks',datetime('now'),datetime('now'))",
            [],
        )
        .unwrap();
        drop(conn);

        assert_eq!(count_task_sources_needing_normalization(&db).unwrap(), 2);
        assert_eq!(normalize_task_sources_in_db(&db).unwrap(), 2);
        assert_eq!(mark_stale_revenue_tasks_needs_review(&db).unwrap(), 1);
        assert_eq!(dismiss_obvious_noise_tasks(&db).unwrap(), 1);
        let conn = db.conn();
        let t1_status: String = conn
            .query_row("SELECT status FROM tasks WHERE id='t1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let t2_status: String = conn
            .query_row("SELECT status FROM tasks WHERE id='t2'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let t1_source: String = conn
            .query_row("SELECT source FROM tasks WHERE id='t1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(t1_status, "needs_review");
        assert_eq!(t2_status, "dismissed");
        assert!(t1_source.contains("\"origin\":\"pg:mentat:tasks\""));
    }
}
