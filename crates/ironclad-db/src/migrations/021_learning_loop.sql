-- Migration 021: Learning loop — learned_skills table
--
-- Part of v0.9.6: closes the procedural memory feedback loop.
-- When sessions close, the agent detects successful multi-step tool
-- sequences and synthesizes reusable skill documents from them.
-- This table tracks those learned skills and their reinforcement history.

CREATE TABLE IF NOT EXISTS learned_skills (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL UNIQUE,
    description       TEXT NOT NULL DEFAULT '',
    trigger_tools     TEXT NOT NULL DEFAULT '[]',
    steps_json        TEXT NOT NULL DEFAULT '[]',
    source_session_id TEXT,
    success_count     INTEGER NOT NULL DEFAULT 1,
    failure_count     INTEGER NOT NULL DEFAULT 0,
    priority          INTEGER NOT NULL DEFAULT 50,
    skill_md_path     TEXT,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_learned_skills_priority ON learned_skills(priority DESC);
