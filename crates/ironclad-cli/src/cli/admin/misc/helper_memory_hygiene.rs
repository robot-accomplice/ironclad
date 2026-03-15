#[derive(Debug, Default)]
struct MemoryHygieneReport {
    working_canned: u32,
    semantic_canned: u32,
    episodic_hallucinated: u32,
    total_detected: u32,
    total_purged: u32,
}

/// Scans memory tiers for known contamination patterns and optionally purges them.
///
/// Detection is strictly deterministic — only exact prefix/substring matches on
/// canned fallback responses, memorised canned facts, and hallucinated subagent
/// output.  Zero false-positive rate by design.
fn run_memory_hygiene(
    db_path: &Path,
    repair: bool,
) -> Result<MemoryHygieneReport, Box<dyn std::error::Error>> {
    if !db_path.exists() {
        return Ok(MemoryHygieneReport::default());
    }

    let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref())?;
    let conn = db.conn();

    // ── Tier 1: working_memory — canned assistant responses ──────────────
    let working_patterns = [
        "Duncan here. The prior generation degraded%",
        "Duncan here. I rejected a low-value%",
        "Duncan: by your command%",
        "Duncan reporting in. I am currently running on%",
        "Active path confirmed%",
    ];

    let working_where = working_patterns
        .iter()
        .map(|p| format!("content LIKE '{p}'"))
        .collect::<Vec<_>>()
        .join(" OR ");

    let working_count: u32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM working_memory WHERE {working_where}"),
        [],
        |row| row.get(0),
    )?;

    // ── Tier 2: semantic_memory — canned responses memorised as facts ────
    let semantic_count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM semantic_memory \
         WHERE key LIKE 'turn_%' \
           AND (value LIKE 'Duncan here.%' OR value LIKE 'Duncan:%')",
        [],
        |row| row.get(0),
    )?;

    // ── Tier 3: episodic_memory — hallucinated subagent output ───────────
    let episodic_count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM episodic_memory \
         WHERE content LIKE '%subtask 1 ->%' \
           AND (content LIKE '%geopolitical-sitrep%' \
                OR content LIKE '%moltbook-monitor%')",
        [],
        |row| row.get(0),
    )?;

    let total_detected = working_count + semantic_count + episodic_count;
    let mut total_purged = 0u32;

    if repair && total_detected > 0 {
        let deleted_working: usize = conn.execute(
            &format!("DELETE FROM working_memory WHERE {working_where}"),
            [],
        )?;

        let deleted_semantic: usize = conn.execute(
            "DELETE FROM semantic_memory \
             WHERE key LIKE 'turn_%' \
               AND (value LIKE 'Duncan here.%' OR value LIKE 'Duncan:%')",
            [],
        )?;

        let deleted_episodic: usize = conn.execute(
            "DELETE FROM episodic_memory \
             WHERE content LIKE '%subtask 1 ->%' \
               AND (content LIKE '%geopolitical-sitrep%' \
                    OR content LIKE '%moltbook-monitor%')",
            [],
        )?;

        total_purged = (deleted_working + deleted_semantic + deleted_episodic) as u32;

        // Reclaim space after bulk deletes.
        let _ = conn.execute_batch("VACUUM;");
    }

    Ok(MemoryHygieneReport {
        working_canned: working_count,
        semantic_canned: semantic_count,
        episodic_hallucinated: episodic_count,
        total_detected,
        total_purged,
    })
}
