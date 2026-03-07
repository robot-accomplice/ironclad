ALTER TABLE revenue_opportunities ADD COLUMN confidence_score REAL NOT NULL DEFAULT 0;
ALTER TABLE revenue_opportunities ADD COLUMN effort_score REAL NOT NULL DEFAULT 0;
ALTER TABLE revenue_opportunities ADD COLUMN risk_score REAL NOT NULL DEFAULT 0;
ALTER TABLE revenue_opportunities ADD COLUMN priority_score REAL NOT NULL DEFAULT 0;
ALTER TABLE revenue_opportunities ADD COLUMN recommended_approved INTEGER NOT NULL DEFAULT 0;
ALTER TABLE revenue_opportunities ADD COLUMN score_reason TEXT;
