#[test]
fn revenue_probe_detects_and_repairs_stale_revenue_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
    let conn = db.conn();
    conn.execute(
            "INSERT INTO tasks (id, title, status, priority, source, created_at, updated_at) \
             VALUES ('t-rev-1','Bounty: stale','in_progress',85,'{\"origin\":\"pg:mentat:tasks\",\"metadata\":{\"type\":\"revenue\"}}',datetime('now','-2 days'),datetime('now','-2 days'))",
            [],
        )
        .unwrap();

    let before = probe_revenue_control_plane(&db_path, false).unwrap();
    assert_eq!(before.stale_revenue_tasks, 1);
    assert_eq!(before.marked_stale_revenue_tasks_needs_review, 0);

    let repaired = probe_revenue_control_plane(&db_path, true).unwrap();
    assert_eq!(repaired.stale_revenue_tasks, 0);
    assert_eq!(repaired.marked_stale_revenue_tasks_needs_review, 1);

    let status: String = conn
        .query_row("SELECT status FROM tasks WHERE id='t-rev-1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "needs_review");
}

#[test]
fn revenue_probe_detects_and_repairs_stale_revenue_swap_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
    drop(db);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
            "INSERT INTO tasks (id, title, status, priority, source, created_at, updated_at) \
             VALUES ('rev_swap:ro_1','Swap settlement','in_progress',95,'{\"origin\":\"revenue_settlement\",\"type\":\"revenue_swap\"}',datetime('now','-2 days'),datetime('now','-2 days'))",
            [],
        )
        .unwrap();
    drop(conn);

    let before = probe_revenue_control_plane(&db_path, false).unwrap();
    assert_eq!(before.revenue_swap_tasks_total, 1);
    assert_eq!(before.stale_revenue_swap_tasks, 1);
    assert_eq!(before.reset_stale_revenue_swap_tasks, 0);

    let repaired = probe_revenue_control_plane(&db_path, true).unwrap();
    assert_eq!(repaired.stale_revenue_swap_tasks, 1);
    assert_eq!(repaired.reset_stale_revenue_swap_tasks, 1);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM tasks WHERE id='rev_swap:ro_1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "pending");
}

#[test]
fn revenue_probe_normalizes_task_sources_and_dismisses_noise() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
    let conn = db.conn();
    conn.execute(
            "INSERT INTO tasks (id, title, status, priority, source, created_at, updated_at) \
             VALUES \
             ('t-src','Bounty: SSV Network','pending',85,'\"{\\\"origin\\\":\\\"pg:mentat:tasks\\\",\\\"metadata\\\":{\\\"type\\\":\\\"revenue\\\"}}\"',datetime('now'),datetime('now')), \
             ('t-noise','What is the juice of saphoo?','pending',5,'pg:agentic_bot:tasks',datetime('now'),datetime('now'))",
            [],
        )
        .unwrap();
    drop(conn);

    let repaired = probe_revenue_control_plane(&db_path, true).unwrap();
    assert_eq!(repaired.normalized_task_sources, 2);
    assert_eq!(repaired.obvious_noise_tasks, 0);
    assert_eq!(repaired.dismissed_noise_tasks, 1);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let normalized_source: String = conn
        .query_row("SELECT source FROM tasks WHERE id='t-src'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let noise_status: String = conn
        .query_row("SELECT status FROM tasks WHERE id='t-noise'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(normalized_source.contains("\"origin\":\"pg:mentat:tasks\""));
    assert_eq!(noise_status, "dismissed");
}

#[test]
fn capability_skill_parity_registry_has_required_skills() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("missing-state.db");
    let report = evaluate_capability_skill_parity(&db_path);
    assert!(
        report.missing_in_registry.is_empty(),
        "builtin registry should include all parity skills: {:?}",
        report.missing_in_registry
    );
}

#[test]
fn capability_skill_parity_detects_db_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
    ironclad_db::skills::register_skill(
        &db,
        "introspection",
        "instruction",
        Some("present"),
        "/tmp/introspection.md",
        "h1",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let report = evaluate_capability_skill_parity(&db_path);
    assert!(report.missing_in_registry.is_empty());
    assert!(
        !report.missing_in_db.is_empty(),
        "expected DB parity gaps when only one required skill is loaded"
    );
}

#[test]
fn capability_skill_parity_treats_internalized_skills_as_satisfied() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
    ironclad_db::skills::register_skill(
        &db,
        "introspection",
        "instruction",
        Some("present"),
        "/tmp/introspection.md",
        "h1",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let report = evaluate_capability_skill_parity(&db_path);
    assert!(
        !report
            .missing_in_db
            .iter()
            .any(|m| m.contains("model-routing-tuner"))
    );
    assert!(
        !report
            .missing_in_db
            .iter()
            .any(|m| m.contains("session-operator"))
    );
    assert!(
        !report
            .missing_in_db
            .iter()
            .any(|m| m.contains("claims-auditor"))
    );
    assert!(
        !report
            .missing_in_db
            .iter()
            .any(|m| m.contains("efficacy-assessment"))
    );
}

#[test]
fn normalize_schema_safe_updates_legacy_subagent_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sub_agents (role TEXT, skills_json TEXT);
             INSERT INTO sub_agents (role, skills_json) VALUES ('specialist', NULL);
             INSERT INTO sub_agents (role, skills_json) VALUES ('commander', '[]');",
    )
    .unwrap();
    drop(conn);

    assert!(normalize_schema_safe(&db_path).unwrap());
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // specialist → subagent, NULL skills → []
    let (role, skills): (String, String) = conn
        .query_row(
            "SELECT role, skills_json FROM sub_agents WHERE rowid = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(role, "subagent");
    assert_eq!(skills, "[]");

    // commander is legacy orchestrator terminology and should not persist in sub_agents.
    let commander_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sub_agents WHERE lower(trim(role))='commander'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(commander_count, 0);

    let total_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM sub_agents", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total_rows, 1);
}

#[test]
fn normalize_schema_safe_converts_invalid_subagent_model_to_auto() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
            "CREATE TABLE sub_agents (role TEXT, model TEXT, skills_json TEXT);
             INSERT INTO sub_agents (role, model, skills_json) VALUES ('subagent', 'orca-ata', '[]');
             INSERT INTO sub_agents (role, model, skills_json) VALUES ('subagent', 'openai/gpt-4o', '[]');
             INSERT INTO sub_agents (role, model, skills_json) VALUES ('subagent', 'auto', '[]');",
        )
        .unwrap();
    drop(conn);

    assert!(normalize_schema_safe(&db_path).unwrap());
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    let m1: String = conn
        .query_row("SELECT model FROM sub_agents WHERE rowid = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let m2: String = conn
        .query_row("SELECT model FROM sub_agents WHERE rowid = 2", [], |r| {
            r.get(0)
        })
        .unwrap();
    let m3: String = conn
        .query_row("SELECT model FROM sub_agents WHERE rowid = 3", [], |r| {
            r.get(0)
        })
        .unwrap();

    assert_eq!(m1, "auto");
    assert_eq!(m2, "openai/gpt-4o");
    assert_eq!(m3, "auto");
}

#[test]
fn normalize_cron_payload_json_migrates_intentful_log_job_to_agent_task() {
    let repaired = crate::state_hygiene::normalize_cron_payload_json(
        Some("summarize overnight events"),
        r#"{"action":"log","message":"scheduled job: morning-briefing"}"#,
    )
    .unwrap();
    assert!(repaired.contains(r#""action":"agent_task""#));
    assert!(repaired.contains(r#""task":"summarize overnight events""#));
}

#[test]
fn normalize_cron_payload_json_repairs_invalid_and_unknown_actions() {
    let repaired_invalid = crate::state_hygiene::normalize_cron_payload_json(None, "not-json").unwrap();
    assert_eq!(repaired_invalid, r#"{"action":"noop"}"#);

    let repaired_unknown =
        crate::state_hygiene::normalize_cron_payload_json(None, r#"{"action":"unknown"}"#).unwrap();
    assert_eq!(repaired_unknown, r#"{"action":"noop"}"#);

    let repaired_legacy =
        crate::state_hygiene::normalize_cron_payload_json(None, r#"{"kind":"metricSnapshot"}"#).unwrap();
    assert!(repaired_legacy.contains(r#""action":"metric_snapshot""#));
}

#[test]
fn normalize_schema_safe_repairs_cron_payloads_and_disables_invalid_cron() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sub_agents (role TEXT, skills_json TEXT);
             CREATE TABLE cron_jobs (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               description TEXT,
               enabled INTEGER NOT NULL DEFAULT 1,
               schedule_kind TEXT NOT NULL,
               schedule_expr TEXT,
               agent_id TEXT NOT NULL DEFAULT '',
               session_target TEXT NOT NULL DEFAULT 'main',
               payload_json TEXT NOT NULL
             );
             INSERT INTO cron_jobs (id, name, payload_json, enabled, schedule_kind, schedule_expr)
               VALUES ('j1', 'job1', '{\"action\":\"unknown\"}', 1, 'every', '5s');
             INSERT INTO cron_jobs (id, name, payload_json, enabled, schedule_kind, schedule_expr)
               VALUES ('j2', 'job2', '{\"kind\":\"metricSnapshot\"}', 1, 'every', '5s');
             INSERT INTO cron_jobs (id, name, payload_json, enabled, schedule_kind, schedule_expr)
               VALUES ('j3', 'job3', '{\"action\":\"log\"}', 1, 'cron', 'NOT_VALID_CRON');",
    )
    .unwrap();
    drop(conn);

    assert!(normalize_schema_safe(&db_path).unwrap());
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    let payload_j1: String = conn
        .query_row(
            "SELECT payload_json FROM cron_jobs WHERE id='j1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(payload_j1, r#"{"action":"noop"}"#);

    let payload_j2: String = conn
        .query_row(
            "SELECT payload_json FROM cron_jobs WHERE id='j2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(payload_j2.contains(r#""action":"metric_snapshot""#));

    let enabled_j3: i64 = conn
        .query_row("SELECT enabled FROM cron_jobs WHERE id='j3'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(enabled_j3, 0);
}
