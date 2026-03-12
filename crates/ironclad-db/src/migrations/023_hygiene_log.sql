-- Retrieval-hygiene audit log.
-- Each governor sweep records current config thresholds, table counts, and
-- pruning actions so the mechanic (or future auto-tuner) can review trends.
CREATE TABLE IF NOT EXISTS hygiene_log (
    id                             TEXT PRIMARY KEY,
    sweep_at                       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    stale_procedural_days          INTEGER NOT NULL,
    dead_skill_priority_threshold  INTEGER NOT NULL,
    proc_total                     INTEGER NOT NULL DEFAULT 0,
    proc_stale                     INTEGER NOT NULL DEFAULT 0,
    proc_pruned                    INTEGER NOT NULL DEFAULT 0,
    skills_total                   INTEGER NOT NULL DEFAULT 0,
    skills_dead                    INTEGER NOT NULL DEFAULT 0,
    skills_pruned                  INTEGER NOT NULL DEFAULT 0,
    avg_skill_priority             REAL NOT NULL DEFAULT 0.0
);
CREATE INDEX IF NOT EXISTS idx_hygiene_log_sweep ON hygiene_log(sweep_at DESC);
