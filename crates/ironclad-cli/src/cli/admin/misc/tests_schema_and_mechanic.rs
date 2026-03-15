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

// Tests for `run_state_hygiene` and `normalize_cron_payload_json` live in
// the `state_hygiene` module (crates/ironclad-cli/src/state_hygiene.rs).
