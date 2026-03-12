//! Audit log for retrieval-hygiene sweeps.
//!
//! Each governor tick that runs `run_retrieval_hygiene()` records the current
//! config thresholds, table counts, and pruning outcomes.  This creates a
//! time-series that the mechanic can query for forensics ("when did that skill
//! disappear?") and that a future auto-tuner can use as training data.

use crate::Database;
use ironclad_core::{IroncladError, Result};

/// Snapshot of a single hygiene sweep, suitable for trend analysis.
#[derive(Debug, Clone)]
pub struct HygieneLogEntry {
    pub id: String,
    pub sweep_at: String,
    pub stale_procedural_days: u32,
    pub dead_skill_priority_threshold: i64,
    pub proc_total: i64,
    pub proc_stale: i64,
    pub proc_pruned: i64,
    pub skills_total: i64,
    pub skills_dead: i64,
    pub skills_pruned: i64,
    pub avg_skill_priority: f64,
}

/// Input parameters for recording a hygiene sweep (no auto-generated fields).
#[derive(Debug, Clone)]
pub struct HygieneSweepInput {
    pub stale_procedural_days: u32,
    pub dead_skill_priority_threshold: i64,
    pub proc_total: i64,
    pub proc_stale: i64,
    pub proc_pruned: i64,
    pub skills_total: i64,
    pub skills_dead: i64,
    pub skills_pruned: i64,
    pub avg_skill_priority: f64,
}

/// Record a completed hygiene sweep.
pub fn log_hygiene_sweep(db: &Database, input: &HygieneSweepInput) -> Result<()> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO hygiene_log \
             (id, stale_procedural_days, dead_skill_priority_threshold, \
              proc_total, proc_stale, proc_pruned, \
              skills_total, skills_dead, skills_pruned, avg_skill_priority) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id,
            input.stale_procedural_days,
            input.dead_skill_priority_threshold,
            input.proc_total,
            input.proc_stale,
            input.proc_pruned,
            input.skills_total,
            input.skills_dead,
            input.skills_pruned,
            input.avg_skill_priority,
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

/// Retrieve recent hygiene log entries, newest first.
pub fn recent_hygiene_log(db: &Database, limit: usize) -> Result<Vec<HygieneLogEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, sweep_at, stale_procedural_days, dead_skill_priority_threshold, \
                    proc_total, proc_stale, proc_pruned, \
                    skills_total, skills_dead, skills_pruned, avg_skill_priority \
             FROM hygiene_log ORDER BY sweep_at DESC LIMIT ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(HygieneLogEntry {
                id: row.get(0)?,
                sweep_at: row.get(1)?,
                stale_procedural_days: row.get::<_, i64>(2)? as u32,
                dead_skill_priority_threshold: row.get(3)?,
                proc_total: row.get(4)?,
                proc_stale: row.get(5)?,
                proc_pruned: row.get(6)?,
                skills_total: row.get(7)?,
                skills_dead: row.get(8)?,
                skills_pruned: row.get(9)?,
                avg_skill_priority: row.get(10)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    fn sweep_input(proc_total: i64, avg_priority: f64) -> HygieneSweepInput {
        HygieneSweepInput {
            stale_procedural_days: 30,
            dead_skill_priority_threshold: 0,
            proc_total,
            proc_stale: 0,
            proc_pruned: 0,
            skills_total: 0,
            skills_dead: 0,
            skills_pruned: 0,
            avg_skill_priority: avg_priority,
        }
    }

    #[test]
    fn log_and_retrieve_hygiene_sweep() {
        let db = test_db();
        let input1 = HygieneSweepInput {
            stale_procedural_days: 30,
            dead_skill_priority_threshold: 0,
            proc_total: 100,
            proc_stale: 5,
            proc_pruned: 3,
            skills_total: 20,
            skills_dead: 2,
            skills_pruned: 2,
            avg_skill_priority: 45.0,
        };
        let input2 = HygieneSweepInput {
            proc_total: 97,
            proc_stale: 2,
            proc_pruned: 1,
            skills_total: 18,
            skills_dead: 0,
            skills_pruned: 0,
            avg_skill_priority: 48.0,
            ..input1
        };
        log_hygiene_sweep(&db, &input1).unwrap();
        log_hygiene_sweep(&db, &input2).unwrap();

        let entries = recent_hygiene_log(&db, 10).unwrap();
        assert_eq!(entries.len(), 2);
        // Both entries present; verify field mapping on either
        let totals: Vec<i64> = entries.iter().map(|e| e.proc_total).collect();
        assert!(totals.contains(&100));
        assert!(totals.contains(&97));
        // Check config recorded correctly
        assert!(entries.iter().all(|e| e.stale_procedural_days == 30));
        assert!(entries.iter().all(|e| e.dead_skill_priority_threshold == 0));
    }

    #[test]
    fn recent_hygiene_log_empty_db() {
        let db = test_db();
        let entries = recent_hygiene_log(&db, 10).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn recent_hygiene_log_respects_limit() {
        let db = test_db();
        for i in 0..5 {
            log_hygiene_sweep(&db, &sweep_input(i, 0.0)).unwrap();
        }
        let entries = recent_hygiene_log(&db, 3).unwrap();
        assert_eq!(entries.len(), 3);
    }
}
