use rusqlite::OptionalExtension;

use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub schedule_kind: String,
    pub schedule_expr: Option<String>,
    pub schedule_every_ms: Option<i64>,
    pub schedule_tz: Option<String>,
    pub agent_id: String,
    pub session_target: String,
    pub payload_json: String,
    pub delivery_mode: Option<String>,
    pub delivery_channel: Option<String>,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub last_duration_ms: Option<i64>,
    pub consecutive_errors: i64,
    pub next_run_at: Option<String>,
    pub last_error: Option<String>,
    pub lease_holder: Option<String>,
    pub lease_expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CronRun {
    pub id: String,
    pub job_id: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub created_at: String,
}

pub fn create_job(
    db: &Database,
    name: &str,
    agent_id: &str,
    schedule_kind: &str,
    schedule_expr: Option<&str>,
    payload_json: &str,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO cron_jobs (id, name, agent_id, schedule_kind, schedule_expr, payload_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            name,
            agent_id,
            schedule_kind,
            schedule_expr,
            payload_json
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn list_jobs(db: &Database) -> Result<Vec<CronJob>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, description, enabled, schedule_kind, schedule_expr, \
             schedule_every_ms, schedule_tz, agent_id, session_target, payload_json, \
             delivery_mode, delivery_channel, last_run_at, last_status, last_duration_ms, \
             consecutive_errors, next_run_at, last_error, lease_holder, lease_expires_at \
             FROM cron_jobs ORDER BY name ASC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                enabled: row.get::<_, i32>(3)? != 0,
                schedule_kind: row.get(4)?,
                schedule_expr: row.get(5)?,
                schedule_every_ms: row.get(6)?,
                schedule_tz: row.get(7)?,
                agent_id: row.get(8)?,
                session_target: row.get(9)?,
                payload_json: row.get(10)?,
                delivery_mode: row.get(11)?,
                delivery_channel: row.get(12)?,
                last_run_at: row.get(13)?,
                last_status: row.get(14)?,
                last_duration_ms: row.get(15)?,
                consecutive_errors: row.get(16)?,
                next_run_at: row.get(17)?,
                last_error: row.get(18)?,
                lease_holder: row.get(19)?,
                lease_expires_at: row.get(20)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn get_job(db: &Database, id: &str) -> Result<Option<CronJob>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, name, description, enabled, schedule_kind, schedule_expr, \
         schedule_every_ms, schedule_tz, agent_id, session_target, payload_json, \
         delivery_mode, delivery_channel, last_run_at, last_status, last_duration_ms, \
         consecutive_errors, next_run_at, last_error, lease_holder, lease_expires_at \
         FROM cron_jobs WHERE id = ?1",
        [id],
        |row| {
            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                enabled: row.get::<_, i32>(3)? != 0,
                schedule_kind: row.get(4)?,
                schedule_expr: row.get(5)?,
                schedule_every_ms: row.get(6)?,
                schedule_tz: row.get(7)?,
                agent_id: row.get(8)?,
                session_target: row.get(9)?,
                payload_json: row.get(10)?,
                delivery_mode: row.get(11)?,
                delivery_channel: row.get(12)?,
                last_run_at: row.get(13)?,
                last_status: row.get(14)?,
                last_duration_ms: row.get(15)?,
                consecutive_errors: row.get(16)?,
                next_run_at: row.get(17)?,
                last_error: row.get(18)?,
                lease_holder: row.get(19)?,
                lease_expires_at: row.get(20)?,
            })
        },
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn delete_job(db: &Database, id: &str) -> Result<bool> {
    fn quote_ident(s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    // Delete dependent rows from every table that has an FK to cron_jobs.
    // This keeps delete robust across schema versions/custom local tables.
    let mut table_stmt = tx
        .prepare(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let table_names = table_stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    drop(table_stmt);

    for table in table_names {
        let pragma_sql = format!("PRAGMA foreign_key_list({})", quote_ident(&table));
        let mut fk_stmt = tx
            .prepare(&pragma_sql)
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        let fk_cols = fk_stmt
            .query_map([], |row| {
                let ref_table: String = row.get(2)?;
                let from_col: String = row.get(3)?;
                Ok((ref_table, from_col))
            })
            .map_err(|e| IroncladError::Database(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        drop(fk_stmt);

        for (ref_table, from_col) in fk_cols {
            if ref_table == "cron_jobs" {
                let delete_sql = format!(
                    "DELETE FROM {} WHERE {} = ?1",
                    quote_ident(&table),
                    quote_ident(&from_col)
                );
                tx.execute(&delete_sql, [id])
                    .map_err(|e| IroncladError::Database(e.to_string()))?;
            }
        }
    }

    let changed = tx
        .execute("DELETE FROM cron_jobs WHERE id = ?1", [id])
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    tx.commit()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(changed > 0)
}

pub fn update_job(
    db: &Database,
    id: &str,
    name: Option<&str>,
    schedule_kind: Option<&str>,
    schedule_expr: Option<&str>,
    enabled: Option<bool>,
) -> Result<bool> {
    let conn = db.conn();
    let mut sets = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(v) = name {
        sets.push("name = ?");
        params.push(Box::new(v.to_string()));
    }
    if let Some(v) = schedule_kind {
        sets.push("schedule_kind = ?");
        params.push(Box::new(v.to_string()));
    }
    if let Some(v) = schedule_expr {
        sets.push("schedule_expr = ?");
        params.push(Box::new(v.to_string()));
    }
    if let Some(v) = enabled {
        sets.push("enabled = ?");
        params.push(Box::new(v as i32));
    }

    if sets.is_empty() {
        return Ok(false);
    }

    // Renumber placeholders: ?1, ?2, ... ?N, id = ?N+1
    let numbered: Vec<String> = sets
        .iter()
        .enumerate()
        .map(|(i, s)| s.replace('?', &format!("?{}", i + 1)))
        .collect();
    let id_param = params.len() + 1;
    let sql = format!(
        "UPDATE cron_jobs SET {} WHERE id = ?{id_param}",
        numbered.join(", ")
    );
    params.push(Box::new(id.to_string()));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let changed = conn
        .execute(&sql, param_refs.as_slice())
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(changed > 0)
}

/// Attempts to acquire a 60-second lease for `instance_id` on the given job.
/// Returns `true` if the lease was acquired (no existing valid lease or expired).
pub fn acquire_lease(db: &Database, job_id: &str, instance_id: &str) -> Result<bool> {
    let conn = db.conn();
    let changed = conn
        .execute(
            "UPDATE cron_jobs SET lease_holder = ?1, lease_expires_at = datetime('now', '+60 seconds') \
             WHERE id = ?2 AND (lease_holder IS NULL OR lease_expires_at < datetime('now'))",
            rusqlite::params![instance_id, job_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(changed > 0)
}

pub fn release_lease(db: &Database, job_id: &str, lease_holder: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE cron_jobs SET lease_holder = NULL, lease_expires_at = NULL \
         WHERE id = ?1 AND lease_holder = ?2",
        rusqlite::params![job_id, lease_holder],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn record_run(
    db: &Database,
    job_id: &str,
    status: &str,
    duration_ms: Option<i64>,
    error: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO cron_runs (id, job_id, status, duration_ms, error) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, job_id, status, duration_ms, error],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    if status == "success" {
        tx.execute(
            "UPDATE cron_jobs SET last_run_at = datetime('now'), last_status = ?1, \
             last_duration_ms = ?2, consecutive_errors = 0, last_error = NULL WHERE id = ?3",
            rusqlite::params![status, duration_ms, job_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    } else {
        tx.execute(
            "UPDATE cron_jobs SET last_run_at = datetime('now'), last_status = ?1, \
             last_duration_ms = ?2, consecutive_errors = consecutive_errors + 1, \
             last_error = ?3 WHERE id = ?4",
            rusqlite::params![status, duration_ms, error, job_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    }

    tx.commit()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn list_runs(
    db: &Database,
    from: Option<&str>,
    to: Option<&str>,
    job_id: Option<&str>,
    limit: i64,
) -> Result<Vec<CronRun>> {
    let conn = db.conn();
    let sql = "SELECT id, job_id, status, duration_ms, error, created_at
               FROM cron_runs
               WHERE (?1 IS NULL OR created_at >= ?1)
                 AND (?2 IS NULL OR created_at <= ?2)
                 AND (?3 IS NULL OR job_id = ?3)
               ORDER BY created_at DESC
               LIMIT ?4";
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![from, to, job_id, limit], |row| {
            Ok(CronRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                status: row.get(2)?,
                duration_ms: row.get(3)?,
                error: row.get(4)?,
                created_at: row.get(5)?,
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
    fn create_and_list_jobs() {
        let db = test_db();
        create_job(
            &db,
            "heartbeat",
            "agent-1",
            "every",
            None,
            r#"{"action":"ping"}"#,
        )
        .unwrap();
        create_job(
            &db,
            "daily-report",
            "agent-1",
            "cron",
            Some("0 9 * * *"),
            r#"{"action":"report"}"#,
        )
        .unwrap();

        let jobs = list_jobs(&db).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "daily-report");
        assert_eq!(jobs[1].name, "heartbeat");
    }

    #[test]
    fn lease_acquisition_and_release() {
        let db = test_db();
        let job_id = create_job(&db, "task", "a1", "every", None, "{}").unwrap();

        assert!(acquire_lease(&db, &job_id, "instance-1").unwrap());
        // Second acquire by a different instance should fail (lease not expired)
        assert!(!acquire_lease(&db, &job_id, "instance-2").unwrap());

        release_lease(&db, &job_id, "instance-1").unwrap();
        assert!(acquire_lease(&db, &job_id, "instance-2").unwrap());
    }

    #[test]
    fn get_and_delete_job() {
        let db = test_db();
        let job_id = create_job(&db, "to-delete", "a1", "every", None, "{}").unwrap();

        let job = get_job(&db, &job_id).unwrap().expect("job should exist");
        assert_eq!(job.name, "to-delete");
        assert_eq!(job.agent_id, "a1");

        assert!(delete_job(&db, &job_id).unwrap());
        assert!(get_job(&db, &job_id).unwrap().is_none());
        assert!(!delete_job(&db, &job_id).unwrap());
    }

    #[test]
    fn record_run_updates_job() {
        let db = test_db();
        let job_id = create_job(&db, "task", "a1", "every", None, "{}").unwrap();

        record_run(&db, &job_id, "success", Some(150), None).unwrap();
        let jobs = list_jobs(&db).unwrap();
        assert_eq!(jobs[0].last_status.as_deref(), Some("success"));
        assert_eq!(jobs[0].consecutive_errors, 0);

        record_run(&db, &job_id, "error", Some(50), Some("timeout")).unwrap();
        let jobs = list_jobs(&db).unwrap();
        assert_eq!(jobs[0].consecutive_errors, 1);
        assert_eq!(jobs[0].last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn get_job_nonexistent_returns_none() {
        let db = test_db();
        assert!(get_job(&db, "does-not-exist").unwrap().is_none());
    }

    #[test]
    fn list_jobs_empty_db() {
        let db = test_db();
        let jobs = list_jobs(&db).unwrap();
        assert!(jobs.is_empty());
    }

    #[test]
    fn create_job_defaults() {
        let db = test_db();
        let id = create_job(&db, "j1", "a1", "every", None, "{}").unwrap();
        let job = get_job(&db, &id).unwrap().unwrap();
        assert!(job.enabled);
        assert!(job.last_run_at.is_none());
        assert!(job.last_status.is_none());
        assert_eq!(job.consecutive_errors, 0);
        assert!(job.last_error.is_none());
        assert!(job.lease_holder.is_none());
    }

    #[test]
    fn create_job_with_schedule_expr() {
        let db = test_db();
        let id = create_job(
            &db,
            "cron-job",
            "a1",
            "cron",
            Some("0 */5 * * *"),
            r#"{"a":1}"#,
        )
        .unwrap();
        let job = get_job(&db, &id).unwrap().unwrap();
        assert_eq!(job.schedule_kind, "cron");
        assert_eq!(job.schedule_expr.as_deref(), Some("0 */5 * * *"));
    }

    #[test]
    fn record_run_success_clears_last_error() {
        let db = test_db();
        let job_id = create_job(&db, "task", "a1", "every", None, "{}").unwrap();

        record_run(&db, &job_id, "error", Some(10), Some("oops")).unwrap();
        let job = get_job(&db, &job_id).unwrap().unwrap();
        assert_eq!(job.consecutive_errors, 1);
        assert_eq!(job.last_error.as_deref(), Some("oops"));

        record_run(&db, &job_id, "success", Some(20), None).unwrap();
        let job = get_job(&db, &job_id).unwrap().unwrap();
        assert_eq!(job.consecutive_errors, 0);
        assert!(job.last_error.is_none());
    }

    #[test]
    fn record_run_with_none_duration() {
        let db = test_db();
        let job_id = create_job(&db, "task", "a1", "every", None, "{}").unwrap();
        let run_id = record_run(&db, &job_id, "error", None, Some("crash")).unwrap();
        assert!(!run_id.is_empty());
        let job = get_job(&db, &job_id).unwrap().unwrap();
        assert!(job.last_duration_ms.is_none());
    }

    #[test]
    fn consecutive_errors_compound() {
        let db = test_db();
        let job_id = create_job(&db, "task", "a1", "every", None, "{}").unwrap();
        for i in 1..=5 {
            record_run(&db, &job_id, "error", Some(10), Some(&format!("err-{i}"))).unwrap();
            let job = get_job(&db, &job_id).unwrap().unwrap();
            assert_eq!(job.consecutive_errors, i);
        }
    }

    #[test]
    fn acquire_lease_nonexistent_job() {
        let db = test_db();
        let acquired = acquire_lease(&db, "no-such-job", "inst-1").unwrap();
        assert!(!acquired);
    }

    #[test]
    fn release_lease_nonexistent_job() {
        let db = test_db();
        release_lease(&db, "no-such-job", "inst-1").unwrap();
    }

    #[test]
    fn record_run_returns_unique_ids() {
        let db = test_db();
        let job_id = create_job(&db, "task", "a1", "every", None, "{}").unwrap();
        let r1 = record_run(&db, &job_id, "success", Some(10), None).unwrap();
        let r2 = record_run(&db, &job_id, "success", Some(20), None).unwrap();
        assert_ne!(r1, r2);
    }
}
