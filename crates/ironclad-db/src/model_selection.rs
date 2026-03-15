use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;

use crate::{Database, DbResultExt};

/// Current routing feature schema version. Bump when feature extraction or
/// scoring logic changes to invalidate historical reproducibility.
pub const ROUTING_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct ModelSelectionEventRow {
    pub id: String,
    pub turn_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub channel: String,
    pub selected_model: String,
    pub strategy: String,
    pub primary_model: String,
    pub override_model: Option<String>,
    pub complexity: Option<String>,
    pub user_excerpt: String,
    pub candidates_json: String,
    pub created_at: String,
    // v0.9.4: routing baseline hardening fields
    pub schema_version: i64,
    pub attribution: Option<String>,
    pub metascore_json: Option<String>,
    pub features_json: Option<String>,
}

pub fn record_model_selection_event(db: &Database, row: &ModelSelectionEventRow) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO model_selection_events
         (id, turn_id, session_id, agent_id, channel, selected_model, strategy, primary_model,
          override_model, complexity, user_excerpt, candidates_json, created_at,
          schema_version, attribution, metascore_json, features_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        rusqlite::params![
            row.id,
            row.turn_id,
            row.session_id,
            row.agent_id,
            row.channel,
            row.selected_model,
            row.strategy,
            row.primary_model,
            row.override_model,
            row.complexity,
            row.user_excerpt,
            row.candidates_json,
            row.created_at,
            row.schema_version,
            row.attribution,
            row.metascore_json,
            row.features_json,
        ],
    )
    .map_err(|e| IroncladError::Database(format!("record model selection event: {e}")))?;
    Ok(())
}

pub fn get_model_selection_by_turn_id(
    db: &Database,
    turn_id: &str,
) -> Result<Option<ModelSelectionEventRow>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id, session_id, agent_id, channel, selected_model, strategy, primary_model,
                    override_model, complexity, user_excerpt, candidates_json, created_at,
                    schema_version, attribution, metascore_json, features_json
             FROM model_selection_events
             WHERE turn_id = ?1
             ORDER BY created_at DESC
             LIMIT 1",
        )
        .db_err()?;
    let row = stmt
        .query_row(rusqlite::params![turn_id], |r| {
            Ok(ModelSelectionEventRow {
                id: r.get(0)?,
                turn_id: r.get(1)?,
                session_id: r.get(2)?,
                agent_id: r.get(3)?,
                channel: r.get(4)?,
                selected_model: r.get(5)?,
                strategy: r.get(6)?,
                primary_model: r.get(7)?,
                override_model: r.get(8)?,
                complexity: r.get(9)?,
                user_excerpt: r.get(10)?,
                candidates_json: r.get(11)?,
                created_at: r.get(12)?,
                schema_version: r.get(13)?,
                attribution: r.get(14)?,
                metascore_json: r.get(15)?,
                features_json: r.get(16)?,
            })
        })
        .optional()
        .db_err()?;
    Ok(row)
}

pub fn list_model_selection_events(
    db: &Database,
    limit: usize,
) -> Result<Vec<ModelSelectionEventRow>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id, session_id, agent_id, channel, selected_model, strategy, primary_model,
                    override_model, complexity, user_excerpt, candidates_json, created_at,
                    schema_version, attribution, metascore_json, features_json
             FROM model_selection_events
             ORDER BY created_at DESC
             LIMIT ?1",
        )
        .db_err()?;
    let rows = stmt
        .query_map(rusqlite::params![limit as i64], |r| {
            Ok(ModelSelectionEventRow {
                id: r.get(0)?,
                turn_id: r.get(1)?,
                session_id: r.get(2)?,
                agent_id: r.get(3)?,
                channel: r.get(4)?,
                selected_model: r.get(5)?,
                strategy: r.get(6)?,
                primary_model: r.get(7)?,
                override_model: r.get(8)?,
                complexity: r.get(9)?,
                user_excerpt: r.get(10)?,
                candidates_json: r.get(11)?,
                created_at: r.get(12)?,
                schema_version: r.get(13)?,
                attribution: r.get(14)?,
                metascore_json: r.get(15)?,
                features_json: r.get(16)?,
            })
        })
        .db_err()?
        .collect::<std::result::Result<Vec<_>, _>>()
        .db_err()?;
    Ok(rows)
}

/// Count routing decisions grouped by attribution label since a given datetime.
pub fn attribution_breakdown(db: &Database, since: Option<&str>) -> Result<Vec<(String, i64)>> {
    let conn = db.conn();
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match since {
        Some(dt) => (
            "SELECT COALESCE(attribution, 'unknown'), COUNT(*)
             FROM model_selection_events
             WHERE created_at >= ?1
             GROUP BY COALESCE(attribution, 'unknown')
             ORDER BY COUNT(*) DESC",
            vec![Box::new(dt.to_string())],
        ),
        None => (
            "SELECT COALESCE(attribution, 'unknown'), COUNT(*)
             FROM model_selection_events
             GROUP BY COALESCE(attribution, 'unknown')
             ORDER BY COUNT(*) DESC",
            vec![],
        ),
    };
    let mut stmt = conn.prepare(sql).db_err()?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .db_err()?
        .collect::<std::result::Result<Vec<_>, _>>()
        .db_err()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    fn sample_event(id: &str, turn_id: &str) -> ModelSelectionEventRow {
        ModelSelectionEventRow {
            id: id.to_string(),
            turn_id: turn_id.to_string(),
            session_id: "sess-1".to_string(),
            agent_id: "agent-1".to_string(),
            channel: "cli".to_string(),
            selected_model: "claude-4".to_string(),
            strategy: "complexity".to_string(),
            primary_model: "claude-4".to_string(),
            override_model: None,
            complexity: Some("high".to_string()),
            user_excerpt: "Tell me about Rust".to_string(),
            candidates_json: r#"["claude-4","gpt-4"]"#.to_string(),
            created_at: "2025-06-01T00:00:00".to_string(),
            schema_version: ROUTING_SCHEMA_VERSION,
            attribution: None,
            metascore_json: None,
            features_json: None,
        }
    }

    #[test]
    fn record_and_get_by_turn_id() {
        let db = test_db();
        let evt = sample_event("mse-1", "turn-1");
        record_model_selection_event(&db, &evt).unwrap();

        let found = get_model_selection_by_turn_id(&db, "turn-1")
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "mse-1");
        assert_eq!(found.selected_model, "claude-4");
        assert_eq!(found.strategy, "complexity");
        assert_eq!(found.complexity.as_deref(), Some("high"));
        assert_eq!(found.schema_version, ROUTING_SCHEMA_VERSION);
    }

    #[test]
    fn get_by_turn_id_returns_none_for_missing() {
        let db = test_db();
        let found = get_model_selection_by_turn_id(&db, "nonexistent").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn record_with_override_model() {
        let db = test_db();
        let mut evt = sample_event("mse-2", "turn-2");
        evt.override_model = Some("gpt-4".to_string());
        record_model_selection_event(&db, &evt).unwrap();

        let found = get_model_selection_by_turn_id(&db, "turn-2")
            .unwrap()
            .unwrap();
        assert_eq!(found.override_model.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn record_with_no_complexity() {
        let db = test_db();
        let mut evt = sample_event("mse-3", "turn-3");
        evt.complexity = None;
        record_model_selection_event(&db, &evt).unwrap();

        let found = get_model_selection_by_turn_id(&db, "turn-3")
            .unwrap()
            .unwrap();
        assert!(found.complexity.is_none());
    }

    #[test]
    fn record_with_attribution_and_metascore() {
        let db = test_db();
        let mut evt = sample_event("mse-attr", "turn-attr");
        evt.attribution = Some("metascore".to_string());
        evt.metascore_json = Some(r#"{"efficacy":0.8,"cost":0.5}"#.to_string());
        evt.features_json = Some(r#"[0.3,0.5,0.1]"#.to_string());
        record_model_selection_event(&db, &evt).unwrap();

        let found = get_model_selection_by_turn_id(&db, "turn-attr")
            .unwrap()
            .unwrap();
        assert_eq!(found.attribution.as_deref(), Some("metascore"));
        assert!(found.metascore_json.is_some());
        assert!(found.features_json.is_some());
        assert_eq!(found.schema_version, ROUTING_SCHEMA_VERSION);
    }

    #[test]
    fn list_events_empty() {
        let db = test_db();
        let events = list_model_selection_events(&db, 10).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn list_events_returns_all() {
        let db = test_db();
        for i in 0..3 {
            let mut evt = sample_event(&format!("mse-list-{i}"), &format!("turn-list-{i}"));
            evt.created_at = format!("2025-06-01T0{i}:00:00");
            record_model_selection_event(&db, &evt).unwrap();
        }

        let events = list_model_selection_events(&db, 10).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn list_events_respects_limit() {
        let db = test_db();
        for i in 0..5 {
            let mut evt = sample_event(&format!("mse-lim-{i}"), &format!("turn-lim-{i}"));
            evt.created_at = format!("2025-06-01T0{i}:00:00");
            record_model_selection_event(&db, &evt).unwrap();
        }

        let events = list_model_selection_events(&db, 2).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn list_events_ordered_desc() {
        let db = test_db();
        let mut e1 = sample_event("mse-ord-1", "turn-ord-1");
        e1.created_at = "2025-06-01T01:00:00".to_string();
        let mut e2 = sample_event("mse-ord-2", "turn-ord-2");
        e2.created_at = "2025-06-01T02:00:00".to_string();
        record_model_selection_event(&db, &e1).unwrap();
        record_model_selection_event(&db, &e2).unwrap();

        let events = list_model_selection_events(&db, 10).unwrap();
        assert_eq!(events[0].id, "mse-ord-2", "most recent should be first");
        assert_eq!(events[1].id, "mse-ord-1");
    }

    #[test]
    fn all_fields_populated() {
        let db = test_db();
        let evt = sample_event("mse-fields", "turn-fields");
        record_model_selection_event(&db, &evt).unwrap();

        let found = get_model_selection_by_turn_id(&db, "turn-fields")
            .unwrap()
            .unwrap();
        assert_eq!(found.session_id, "sess-1");
        assert_eq!(found.agent_id, "agent-1");
        assert_eq!(found.channel, "cli");
        assert_eq!(found.primary_model, "claude-4");
        assert_eq!(found.user_excerpt, "Tell me about Rust");
        assert_eq!(found.candidates_json, r#"["claude-4","gpt-4"]"#);
        assert_eq!(found.created_at, "2025-06-01T00:00:00");
    }

    #[test]
    fn duplicate_id_fails() {
        let db = test_db();
        let evt = sample_event("mse-dup", "turn-dup");
        record_model_selection_event(&db, &evt).unwrap();
        // Same id should fail (PRIMARY KEY constraint)
        let result = record_model_selection_event(&db, &evt);
        assert!(result.is_err());
    }

    #[test]
    fn attribution_breakdown_counts_correctly() {
        let db = test_db();
        for (i, attr) in ["metascore", "metascore", "override", "fallback"]
            .iter()
            .enumerate()
        {
            let mut evt = sample_event(&format!("mse-ab-{i}"), &format!("turn-ab-{i}"));
            evt.attribution = Some(attr.to_string());
            evt.created_at = format!("2025-06-01T0{i}:00:00");
            record_model_selection_event(&db, &evt).unwrap();
        }

        let counts = attribution_breakdown(&db, None).unwrap();
        assert_eq!(counts.len(), 3);
        // metascore should be first (count=2)
        assert_eq!(counts[0].0, "metascore");
        assert_eq!(counts[0].1, 2);
    }

    #[test]
    fn attribution_breakdown_with_since_filter() {
        let db = test_db();
        let mut e1 = sample_event("mse-ab-old", "turn-ab-old");
        e1.attribution = Some("metascore".to_string());
        e1.created_at = "2024-01-01T00:00:00".to_string();
        let mut e2 = sample_event("mse-ab-new", "turn-ab-new");
        e2.attribution = Some("override".to_string());
        e2.created_at = "2025-06-01T00:00:00".to_string();
        record_model_selection_event(&db, &e1).unwrap();
        record_model_selection_event(&db, &e2).unwrap();

        let counts = attribution_breakdown(&db, Some("2025-01-01T00:00:00")).unwrap();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].0, "override");
    }
}
