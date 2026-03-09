-- Add settled_at timestamp to revenue_opportunities for cycle-time analytics.
ALTER TABLE revenue_opportunities ADD COLUMN settled_at TEXT;
