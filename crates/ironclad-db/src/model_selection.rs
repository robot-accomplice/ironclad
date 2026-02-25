use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;

use crate::Database;

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
}

pub fn record_model_selection_event(db: &Database, row: &ModelSelectionEventRow) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO model_selection_events
         (id, turn_id, session_id, agent_id, channel, selected_model, strategy, primary_model,
          override_model, complexity, user_excerpt, candidates_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                    override_model, complexity, user_excerpt, candidates_json, created_at
             FROM model_selection_events
             WHERE turn_id = ?1
             ORDER BY created_at DESC
             LIMIT 1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
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
            })
        })
        .optional()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
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
                    override_model, complexity, user_excerpt, candidates_json, created_at
             FROM model_selection_events
             ORDER BY created_at DESC
             LIMIT ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
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
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(rows)
}
