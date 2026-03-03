use std::path::Path;

#[derive(Debug, Default, Clone, Copy)]
pub struct StateHygieneReport {
    pub changed: bool,
    pub changed_rows: u64,
    pub subagent_rows_normalized: u64,
    pub cron_payload_rows_repaired: u64,
    pub cron_jobs_disabled_invalid_expr: u64,
}

pub fn run_state_hygiene(
    state_db_path: &Path,
) -> Result<StateHygieneReport, Box<dyn std::error::Error>> {
    if !state_db_path.exists() {
        return Ok(StateHygieneReport::default());
    }
    let conn = rusqlite::Connection::open(state_db_path)?;
    let mut report = StateHygieneReport::default();
    let has_column = |table: &str, column: &str| -> rusqlite::Result<bool> {
        let mut stmt = conn.prepare(&format!(
            "PRAGMA table_info(\"{}\")",
            table.replace('"', "\"\"")
        ))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        for col in rows {
            if col? == column {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let has_table = |table: &str| -> rusqlite::Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
    };

    conn.execute_batch("BEGIN;")?;

    conn.execute(
        "UPDATE sub_agents SET role='subagent' WHERE lower(trim(role))='specialist'",
        [],
    )?;
    let n = conn.changes();
    report.subagent_rows_normalized += n;
    report.changed_rows += n;

    conn.execute(
        "DELETE FROM sub_agents WHERE lower(trim(role))='commander'",
        [],
    )?;
    let n = conn.changes();
    report.subagent_rows_normalized += n;
    report.changed_rows += n;

    conn.execute(
        "UPDATE sub_agents SET skills_json='[]' WHERE skills_json IS NULL",
        [],
    )?;
    let n = conn.changes();
    report.subagent_rows_normalized += n;
    report.changed_rows += n;

    if has_column("sub_agents", "fallback_models_json")? {
        conn.execute(
            "UPDATE sub_agents SET fallback_models_json='[]' WHERE fallback_models_json IS NULL OR trim(fallback_models_json)=''",
            [],
        )?;
        let n = conn.changes();
        report.subagent_rows_normalized += n;
        report.changed_rows += n;
    }
    if has_column("sub_agents", "model")? {
        conn.execute(
            "UPDATE sub_agents
             SET model='auto'
             WHERE lower(trim(role))='subagent'
               AND lower(trim(model)) IN ('ollama-gpu/qwen3:14b','ollama-gpu/qwen3.5:35b-a3b')",
            [],
        )?;
        let n = conn.changes();
        report.subagent_rows_normalized += n;
        report.changed_rows += n;
    }

    if has_table("cron_jobs")?
        && has_column("cron_jobs", "payload_json")?
        && has_column("cron_jobs", "id")?
    {
        let mut stmt = conn.prepare("SELECT id, payload_json FROM cron_jobs")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, payload_raw) = row?;
            if let Some(payload_json) = normalize_cron_payload_json(&payload_raw) {
                conn.execute(
                    "UPDATE cron_jobs SET payload_json=?1 WHERE id=?2",
                    rusqlite::params![payload_json, id],
                )?;
                let n = conn.changes();
                report.cron_payload_rows_repaired += n;
                report.changed_rows += n;
            }
        }
    }

    if has_table("cron_jobs")?
        && has_column("cron_jobs", "id")?
        && has_column("cron_jobs", "enabled")?
        && has_column("cron_jobs", "schedule_kind")?
        && has_column("cron_jobs", "schedule_expr")?
    {
        let mut stmt = conn.prepare(
            "SELECT id, schedule_expr
             FROM cron_jobs
             WHERE enabled=1 AND lower(trim(schedule_kind))='cron'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (id, expr_opt) = row?;
            let valid = expr_opt
                .as_deref()
                .map(ironclad_schedule::DurableScheduler::is_valid_cron_expression)
                .unwrap_or(false);
            if !valid {
                conn.execute(
                    "UPDATE cron_jobs SET enabled=0 WHERE id=?1",
                    rusqlite::params![id],
                )?;
                let n = conn.changes();
                report.cron_jobs_disabled_invalid_expr += n;
                report.changed_rows += n;
            }
        }
    }

    conn.execute_batch("COMMIT;")?;
    report.changed = report.changed_rows > 0;
    Ok(report)
}

pub(crate) fn normalize_cron_payload_json(raw: &str) -> Option<String> {
    let mut payload = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => v,
        Err(_) => return Some(r#"{"action":"noop"}"#.to_string()),
    };
    let obj = match payload.as_object_mut() {
        Some(v) => v,
        None => return Some(r#"{"action":"noop"}"#.to_string()),
    };
    let mut changed = false;
    if let Some(kind) = obj.get("kind").and_then(|v| v.as_str())
        && obj.get("action").and_then(|v| v.as_str()).is_none()
        && let Some(mapped) = legacy_kind_to_action(kind)
    {
        obj.insert(
            "action".to_string(),
            serde_json::Value::String(mapped.to_string()),
        );
        changed = true;
    }
    let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("unknown");
    if matches!(
        action,
        "log" | "metric_snapshot" | "expire_sessions" | "record_transaction" | "noop"
    ) {
        if changed {
            return serde_json::to_string(&payload).ok();
        }
        return None;
    }
    obj.insert(
        "action".to_string(),
        serde_json::Value::String("noop".to_string()),
    );
    serde_json::to_string(&payload).ok()
}

fn legacy_kind_to_action(kind: &str) -> Option<&'static str> {
    match kind {
        "agentTurn" => Some("noop"),
        "metricSnapshot" => Some("metric_snapshot"),
        "expireSessions" => Some("expire_sessions"),
        "recordTransaction" => Some("record_transaction"),
        "log" => Some("log"),
        "noop" => Some("noop"),
        _ => None,
    }
}
