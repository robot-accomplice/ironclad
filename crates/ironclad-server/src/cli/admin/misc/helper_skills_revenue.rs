#[derive(Debug, Default, Clone)]
struct InternalizedSkillCleanupReport {
    stale_db_skills: Vec<String>,
    stale_files: Vec<PathBuf>,
    stale_dirs: Vec<PathBuf>,
    removed_db_skills: Vec<String>,
    removed_paths: Vec<PathBuf>,
}

fn find_path_case_insensitive(base: &Path, candidate: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.eq_ignore_ascii_case(candidate) {
            return Some(path);
        }
    }
    None
}

fn cleanup_internalized_skill_artifacts(
    state_db_path: &Path,
    skills_dir: &Path,
    repair: bool,
) -> InternalizedSkillCleanupReport {
    let mut report = InternalizedSkillCleanupReport::default();

    if state_db_path.exists() {
        match ironclad_db::Database::new(state_db_path.to_string_lossy().as_ref()) {
            Ok(db) => match ironclad_db::skills::list_skills(&db) {
                Ok(skills) => {
                    for skill in skills {
                        let is_internalized = INTERNALIZED_SKILLS
                            .iter()
                            .any(|name| skill.name.eq_ignore_ascii_case(name));
                        let is_deprecated_generic = DEPRECATED_GENERIC_SKILLS
                            .iter()
                            .any(|name| skill.name.eq_ignore_ascii_case(name));
                        if is_internalized || is_deprecated_generic {
                            report.stale_db_skills.push(skill.name.clone());
                            if repair {
                                if let Err(e) = ironclad_db::skills::delete_skill(&db, &skill.id) {
                                    tracing::warn!(skill = %skill.name, "failed to delete stale skill: {e}");
                                } else {
                                    report.removed_db_skills.push(skill.name);
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("failed to list skills for cleanup: {e}"),
            },
            Err(e) => tracing::warn!("failed to open state DB for skill cleanup: {e}"),
        }
    }

    if skills_dir.exists() {
        let skill_targets = INTERNALIZED_SKILLS
            .iter()
            .chain(DEPRECATED_GENERIC_SKILLS.iter());
        for skill_name in skill_targets {
            let md_name = format!("{skill_name}.md");
            if let Some(path) = find_path_case_insensitive(skills_dir, &md_name) {
                report.stale_files.push(path.clone());
                if repair && std::fs::remove_file(&path).is_ok() {
                    report.removed_paths.push(path);
                }
            }
            if let Some(path) = find_path_case_insensitive(skills_dir, skill_name)
                && path.is_dir()
            {
                report.stale_dirs.push(path.clone());
                if repair && std::fs::remove_dir_all(&path).is_ok() {
                    report.removed_paths.push(path);
                }
            }
        }
    }

    report
}

const BUILTIN_SKILLS_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/builtin-skills.json"));

#[derive(Debug, Deserialize)]
struct BuiltinSkillCatalogEntry {
    name: String,
}

#[derive(Debug, Clone, Copy)]
struct CapabilitySkillParityItem {
    capability: &'static str,
    skills: &'static [&'static str],
}

const CAPABILITY_SKILL_PARITY_ITEMS: &[CapabilitySkillParityItem] = &[
    CapabilitySkillParityItem {
        capability: "runtime introspection and truthful capability disclosure",
        skills: &["introspection"],
    },
    CapabilitySkillParityItem {
        capability: "delegation and specialist orchestration",
        skills: &["supervisor-protocol", "local-subagents"],
    },
    CapabilitySkillParityItem {
        capability: "routing controls and fallback tuning",
        skills: &["model-management", "model-routing-tuner"],
    },
    CapabilitySkillParityItem {
        capability: "diagnostics, repair, and operator continuity",
        skills: &[
            "runtime-diagnostics",
            "self-diagnostics",
            "session-operator",
        ],
    },
    CapabilitySkillParityItem {
        capability: "revenue autonomy operations",
        skills: &["self-funding", "claims-auditor", "efficacy-assessment"],
    },
];

#[derive(Debug, Default, Clone)]
struct CapabilitySkillParityReport {
    missing_in_registry: Vec<String>,
    missing_in_db: Vec<String>,
}

fn evaluate_capability_skill_parity(state_db_path: &Path) -> CapabilitySkillParityReport {
    let mut report = CapabilitySkillParityReport::default();
    let registry_skills: std::collections::HashSet<String> =
        serde_json::from_str::<Vec<BuiltinSkillCatalogEntry>>(BUILTIN_SKILLS_JSON)
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.name.to_ascii_lowercase())
            .collect();

    let mut db_skills: std::collections::HashSet<String> = if state_db_path.exists() {
        match ironclad_db::Database::new(state_db_path.to_string_lossy().as_ref()) {
            Ok(db) => ironclad_db::skills::list_skills(&db)
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to list skills from DB: {e}");
                    Vec::new()
                })
                .into_iter()
                .filter(|s| s.enabled)
                .map(|s| s.name.to_ascii_lowercase())
                .collect(),
            Err(e) => {
                tracing::warn!("failed to open state DB for parity check: {e}");
                std::collections::HashSet::new()
            }
        }
    } else {
        std::collections::HashSet::new()
    };

    // Internalized skills are satisfied by compiled/runtime behavior even when they
    // no longer exist as external skill rows on disk or in the skill DB.
    db_skills.extend(
        INTERNALIZED_SKILLS
            .iter()
            .map(|skill| skill.to_ascii_lowercase()),
    );

    for item in CAPABILITY_SKILL_PARITY_ITEMS {
        for skill in item.skills {
            if !registry_skills.contains(&skill.to_ascii_lowercase()) {
                report
                    .missing_in_registry
                    .push(format!("{} -> {}", item.capability, skill));
            }
            if !db_skills.is_empty() && !db_skills.contains(&skill.to_ascii_lowercase()) {
                report
                    .missing_in_db
                    .push(format!("{} -> {}", item.capability, skill));
            }
        }
    }

    report
}

#[derive(Debug, Clone)]
struct ProviderHealthRow {
    name: String,
    status: String,
    count: u64,
    error: Option<String>,
}

fn provider_scan_hint(provider: Option<&str>) -> String {
    match provider {
        Some(name) if !name.trim().is_empty() => format!("ironclad models scan {}", name.trim()),
        _ => "ironclad models scan".to_string(),
    }
}

#[derive(Debug, Default, Clone)]
struct RevenueControlPlaneHealth {
    opportunities_total: i64,
    opportunities_settled: i64,
    orphan_jobs: i64,
    missing_settlement_ledger: i64,
    normalized_task_sources: i64,
    obvious_noise_tasks: i64,
    revenue_swap_tasks_total: i64,
    revenue_swap_tasks_pending: i64,
    revenue_swap_tasks_in_progress: i64,
    revenue_swap_tasks_failed: i64,
    stale_revenue_swap_tasks: i64,
    stale_revenue_tasks: i64,
    repaired_orphans: i64,
    reconciled_ledger_rows: i64,
    dismissed_noise_tasks: i64,
    reset_stale_revenue_swap_tasks: i64,
    marked_stale_revenue_tasks_needs_review: i64,
}

#[derive(Debug, Default, Clone)]
struct RevenueSwapReconcileHealth {
    submitted_tasks: usize,
    pending_receipts: usize,
    confirmed_repairs: usize,
    failed_repairs: usize,
}

fn sqlite_table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

fn probe_revenue_control_plane(
    state_db_path: &Path,
    repair: bool,
) -> Result<RevenueControlPlaneHealth, Box<dyn std::error::Error>> {
    if !state_db_path.exists() {
        return Ok(RevenueControlPlaneHealth::default());
    }
    let db = ironclad_db::Database::new(state_db_path.to_string_lossy().as_ref())?;
    let revenue_tables_ready = {
        let conn = db.conn();
        sqlite_table_exists(&conn, "revenue_opportunities")
    };
    if !revenue_tables_ready {
        return Ok(RevenueControlPlaneHealth::default());
    }

    let (
        tasks_table_exists,
        opportunities_total,
        opportunities_settled,
        orphan_jobs,
        missing_settlement_ledger,
        revenue_swap_tasks_total,
        revenue_swap_tasks_pending,
        revenue_swap_tasks_in_progress,
        revenue_swap_tasks_failed,
        stale_revenue_swap_tasks,
    ) = {
        let conn = db.conn();
        let tasks_table_exists = sqlite_table_exists(&conn, "tasks");
        let opportunities_total: i64 =
            conn.query_row("SELECT COUNT(*) FROM revenue_opportunities", [], |row| {
                row.get(0)
            })?;
        let opportunities_settled: i64 = conn.query_row(
            "SELECT COUNT(*) FROM revenue_opportunities WHERE status = 'settled'",
            [],
            |row| row.get(0),
        )?;
        let orphan_jobs: i64 = conn.query_row(
            "SELECT COUNT(*) \
             FROM revenue_opportunities ro \
             WHERE ro.request_id IS NOT NULL \
               AND ro.request_id != '' \
               AND NOT EXISTS (SELECT 1 FROM service_requests sr WHERE sr.id = ro.request_id)",
            [],
            |row| row.get(0),
        )?;
        let missing_settlement_ledger: i64 = conn.query_row(
            "SELECT COUNT(*) \
             FROM revenue_opportunities ro \
             WHERE ro.status = 'settled' \
               AND ro.settlement_ref IS NOT NULL \
               AND ro.settlement_ref != '' \
               AND NOT EXISTS (SELECT 1 FROM transactions t WHERE t.tx_type='revenue_settlement' AND t.tx_hash = ro.settlement_ref)",
            [],
            |row| row.get(0),
        )?;
        let (
            revenue_swap_tasks_total,
            revenue_swap_tasks_pending,
            revenue_swap_tasks_in_progress,
            revenue_swap_tasks_failed,
            stale_revenue_swap_tasks,
        ) = if tasks_table_exists {
            conn.query_row(
                "SELECT \
                    COUNT(*), \
                    COALESCE(SUM(CASE WHEN lower(status) = 'pending' THEN 1 ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN lower(status) = 'in_progress' THEN 1 ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN lower(status) = 'failed' THEN 1 ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN lower(status) = 'in_progress' AND datetime(COALESCE(updated_at, created_at)) < datetime('now','-24 hours') THEN 1 ELSE 0 END), 0) \
                 FROM tasks \
                 WHERE lower(COALESCE(source, '')) LIKE '%\"type\":\"revenue_swap\"%'",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )?
        } else {
            (0, 0, 0, 0, 0)
        };
        (
            tasks_table_exists,
            opportunities_total,
            opportunities_settled,
            orphan_jobs,
            missing_settlement_ledger,
            revenue_swap_tasks_total,
            revenue_swap_tasks_pending,
            revenue_swap_tasks_in_progress,
            revenue_swap_tasks_failed,
            stale_revenue_swap_tasks,
        )
    };

    let mut repaired_orphans = 0i64;
    let mut reconciled_ledger_rows = 0i64;
    let mut normalized_task_sources = 0i64;
    let mut stale_revenue_tasks = 0i64;
    let mut obvious_noise_tasks = 0i64;
    let mut marked_stale_revenue_tasks_needs_review = 0i64;
    let mut dismissed_noise_tasks = 0i64;
    let mut reset_stale_revenue_swap_tasks = 0i64;

    if tasks_table_exists {
        normalized_task_sources =
            ironclad_db::tasks::count_task_sources_needing_normalization(&db)?;
    }

    if repair {
        {
            let conn = db.conn();
            repaired_orphans = conn.execute(
                "UPDATE revenue_opportunities \
                 SET status = 'failed', qualification_reason = COALESCE(qualification_reason, 'mechanic orphan repair: missing linked request'), updated_at = datetime('now') \
                 WHERE request_id IS NOT NULL \
                   AND request_id != '' \
                   AND status IN ('intake', 'qualified', 'planned', 'fulfilled') \
                   AND NOT EXISTS (SELECT 1 FROM service_requests sr WHERE sr.id = request_id)",
                [],
            )? as i64;

            reconciled_ledger_rows = conn.execute(
                "INSERT INTO transactions (id, tx_type, amount, currency, counterparty, tx_hash, metadata_json, created_at) \
                 SELECT 'tx_rec_' || hex(randomblob(8)), \
                        'revenue_settlement', \
                        ro.settled_amount_usdc, \
                        'USDC', \
                        'revenue_control_plane', \
                        ro.settlement_ref, \
                        json_object('reconciled_by','mechanic','opportunity_id',ro.id), \
                        datetime('now') \
                 FROM revenue_opportunities ro \
                 WHERE ro.status = 'settled' \
                   AND ro.settlement_ref IS NOT NULL \
                   AND ro.settlement_ref != '' \
                   AND ro.settled_amount_usdc IS NOT NULL \
                   AND ro.settled_amount_usdc > 0 \
                   AND NOT EXISTS (SELECT 1 FROM transactions t WHERE t.tx_type='revenue_settlement' AND t.tx_hash = ro.settlement_ref)",
                [],
            )? as i64;
        }

        if tasks_table_exists {
            let _ = ironclad_db::tasks::normalize_task_sources_in_db(&db)?;
            {
                let conn = db.conn();
                reset_stale_revenue_swap_tasks = conn.execute(
                    "UPDATE tasks \
                     SET status = 'pending', updated_at = datetime('now') \
                     WHERE lower(status) = 'in_progress' \
                       AND datetime(COALESCE(updated_at, created_at)) < datetime('now','-24 hours') \
                       AND lower(COALESCE(source,'')) LIKE '%\"type\":\"revenue_swap\"%'",
                    [],
                )? as i64;
            }
            marked_stale_revenue_tasks_needs_review =
                ironclad_db::tasks::mark_stale_revenue_tasks_needs_review(&db)?;
            dismissed_noise_tasks = ironclad_db::tasks::dismiss_obvious_noise_tasks(&db)?;
        }
    }

    if tasks_table_exists {
        let (_revenue_like, noise_like) = ironclad_db::tasks::classify_open_tasks(&db)?;
        obvious_noise_tasks = noise_like;
        stale_revenue_tasks = ironclad_db::tasks::count_stale_revenue_tasks(&db)?;
    }

    Ok(RevenueControlPlaneHealth {
        opportunities_total,
        opportunities_settled,
        orphan_jobs,
        missing_settlement_ledger,
        normalized_task_sources,
        obvious_noise_tasks,
        revenue_swap_tasks_total,
        revenue_swap_tasks_pending,
        revenue_swap_tasks_in_progress,
        revenue_swap_tasks_failed,
        stale_revenue_swap_tasks,
        stale_revenue_tasks,
        repaired_orphans,
        reconciled_ledger_rows,
        dismissed_noise_tasks,
        reset_stale_revenue_swap_tasks,
        marked_stale_revenue_tasks_needs_review,
    })
}

async fn fetch_provider_health(
    base_url: &str,
) -> Result<Vec<ProviderHealthRow>, Box<dyn std::error::Error>> {
    let resp = super::http_client()?
        .get(format!(
            "{base_url}/api/models/available?validation_level=zero"
        ))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(std::io::Error::other(format!(
            "provider models endpoint returned HTTP {}",
            resp.status()
        ))
        .into());
    }
    let body: serde_json::Value = resp.json().await?;
    let mut rows = Vec::new();
    let providers = body
        .get("providers")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for (name, report) in providers {
        rows.push(ProviderHealthRow {
            name,
            status: report
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            count: report.get("count").and_then(|v| v.as_u64()).unwrap_or(0),
            error: report
                .get("error")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

async fn probe_revenue_swap_reconcile(
    base_url: &str,
    repair: bool,
) -> Result<RevenueSwapReconcileHealth, Box<dyn std::error::Error>> {
    let resp = super::http_client()?
        .get(format!("{base_url}/api/services/swaps?limit=200"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(std::io::Error::other(format!(
            "swap listing endpoint returned HTTP {}",
            resp.status()
        ))
        .into());
    }
    let body: serde_json::Value = resp.json().await.unwrap_or_else(|e| {
        tracing::warn!("failed to parse swap listing response: {e}");
        serde_json::Value::default()
    });
    let tasks = body
        .get("swap_tasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut health = RevenueSwapReconcileHealth::default();
    for task in tasks {
        let status = task
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let source = task
            .get("source")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let swap_tx_hash = source
            .get("swap_tx_hash")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if !status.eq_ignore_ascii_case("in_progress") || swap_tx_hash.is_none() {
            continue;
        }
        health.submitted_tasks += 1;
        if !repair {
            continue;
        }
        let opportunity_id = task
            .get("opportunity_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if opportunity_id.is_empty() {
            continue;
        }
        let reconcile = super::http_client()?
            .post(format!(
                "{base_url}/api/services/swaps/{}/reconcile",
                opportunity_id
            ))
            .send()
            .await?;
        if !reconcile.status().is_success() {
            continue;
        }
        let reconcile_body: serde_json::Value = reconcile.json().await.unwrap_or_else(|e| {
            tracing::warn!(opportunity_id, "failed to parse reconcile response: {e}");
            serde_json::Value::default()
        });
        match reconcile_body
            .get("receipt_status")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
        {
            "confirmed" => health.confirmed_repairs += 1,
            "failed" => health.failed_repairs += 1,
            "pending" => health.pending_receipts += 1,
            _ => {}
        }
    }
    Ok(health)
}
