-- Enforce one feedback row per turn for proper upsert behavior.
-- Replace the existing non-unique index with a unique one.
DROP INDEX IF EXISTS idx_turn_feedback_turn;
CREATE UNIQUE INDEX IF NOT EXISTS idx_turn_feedback_turn ON turn_feedback(turn_id);
