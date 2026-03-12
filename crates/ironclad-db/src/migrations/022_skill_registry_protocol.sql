-- Migration 022: Skill Registry Protocol
-- Adds version, author, and registry_source columns to the skills table
-- to support multi-registry namespacing and semver-based update checks.

ALTER TABLE skills ADD COLUMN version TEXT NOT NULL DEFAULT '0.0.0';
ALTER TABLE skills ADD COLUMN author TEXT NOT NULL DEFAULT 'local';
ALTER TABLE skills ADD COLUMN registry_source TEXT NOT NULL DEFAULT 'local';
