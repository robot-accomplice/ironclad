-- v0.9.4: Routing baseline hardening — schema versioning, decision attribution,
-- turn linkage, and shadow prediction support.

-- model_selection_events: add schema_version, attribution, metascore/features JSON
ALTER TABLE model_selection_events ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE model_selection_events ADD COLUMN attribution TEXT;
ALTER TABLE model_selection_events ADD COLUMN metascore_json TEXT;
ALTER TABLE model_selection_events ADD COLUMN features_json TEXT;

-- inference_costs: add turn_id for JOIN with model_selection_events
ALTER TABLE inference_costs ADD COLUMN turn_id TEXT;
CREATE INDEX IF NOT EXISTS idx_inference_costs_turn ON inference_costs(turn_id);

-- Shadow ML routing predictions (non-shipping, Phase 2)
CREATE TABLE IF NOT EXISTS shadow_routing_predictions (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    production_model TEXT NOT NULL,
    shadow_model TEXT,
    production_complexity REAL,
    shadow_complexity REAL,
    agreed INTEGER NOT NULL DEFAULT 0,
    detail_json TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_shadow_routing_created ON shadow_routing_predictions(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_shadow_routing_turn ON shadow_routing_predictions(turn_id);
