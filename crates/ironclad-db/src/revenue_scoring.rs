use crate::Database;
use ironclad_core::{IroncladError, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct RevenueOpportunityScoreInput<'a> {
    pub source: &'a str,
    pub strategy: &'a str,
    pub payload_json: &'a str,
    pub expected_revenue_usdc: f64,
    pub request_id: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct RevenueOpportunityScore {
    pub confidence_score: f64,
    pub effort_score: f64,
    pub risk_score: f64,
    pub priority_score: f64,
    pub recommended_approved: bool,
    pub score_reason: String,
}

pub fn score_revenue_opportunity(
    input: &RevenueOpportunityScoreInput<'_>,
) -> RevenueOpportunityScore {
    score_revenue_opportunity_from_signal(input, None)
}

pub fn score_revenue_opportunity_with_feedback(
    db: &Database,
    input: &RevenueOpportunityScoreInput<'_>,
) -> Result<RevenueOpportunityScore> {
    let signal = crate::revenue_feedback::revenue_feedback_signal_for_strategy(db, input.strategy)?;
    Ok(score_revenue_opportunity_from_signal(input, signal))
}

fn score_revenue_opportunity_from_signal(
    input: &RevenueOpportunityScoreInput<'_>,
    feedback_signal: Option<crate::revenue_feedback::RevenueFeedbackSignal>,
) -> RevenueOpportunityScore {
    let (payload, payload_parse_failed) = match serde_json::from_str::<Value>(input.payload_json) {
        Ok(v) => (v, false),
        Err(e) => {
            tracing::warn!(source = %input.source, strategy = %input.strategy, error = %e, "payload_json parse failed during scoring");
            (Value::Null, true)
        }
    };
    let strategy = input.strategy.trim().to_ascii_lowercase();
    let source = input.source.trim().to_ascii_lowercase();
    let has_scope_marker = [
        "repo",
        "url",
        "endpoint",
        "pair",
        "source_url",
        "issue",
        "title",
    ]
    .iter()
    .any(|key| payload.get(key).is_some());
    let action_text = payload
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let multi_repo = payload
        .get("multi_repo")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || action_text.contains("multi-repo");

    let mut confidence: f64 = match strategy.as_str() {
        "oracle_feed" => 0.65,
        "micro_bounty" => 0.55,
        _ => 0.45,
    };
    let mut effort: f64 = match strategy.as_str() {
        "oracle_feed" => 0.35,
        "micro_bounty" => 0.40,
        _ => 0.50,
    };
    let mut risk: f64 = match strategy.as_str() {
        "oracle_feed" => 0.20,
        "micro_bounty" => 0.30,
        _ => 0.40,
    };

    if input.request_id.is_some() {
        confidence += 0.10;
    }
    if has_scope_marker {
        confidence += 0.10;
        effort -= 0.10;
    } else {
        confidence -= 0.10;
        risk += 0.15;
    }
    if source.contains("trusted") || source.contains("board") || source.contains("feed") {
        confidence += 0.05;
    }
    if input.expected_revenue_usdc >= 5.0 {
        confidence += 0.05;
    }
    if input.expected_revenue_usdc > 500.0 {
        risk += 0.10;
    }
    if multi_repo {
        effort += 0.15;
        risk += 0.10;
    }
    if let Some(signal) = feedback_signal {
        let sample_weight = (signal.feedback_count as f64 / 5.0).clamp(0.0, 1.0);
        let grade_weight = ((signal.avg_grade - 3.0) / 2.0).clamp(-1.0, 1.0);
        confidence += 0.10 * sample_weight * grade_weight;
        risk -= 0.08 * sample_weight * grade_weight;
    }

    confidence = confidence.clamp(0.0, 1.0);
    effort = effort.clamp(0.0, 1.0);
    risk = risk.clamp(0.0, 1.0);
    let revenue_weight = (input.expected_revenue_usdc / 1000.0).clamp(0.0, 1.0);
    let priority = ((confidence * 0.45)
        + ((1.0 - risk) * 0.25)
        + ((1.0 - effort) * 0.15)
        + (revenue_weight * 0.15))
        * 100.0;
    let recommended_approved = confidence >= 0.55 && risk <= 0.60 && effort <= 0.70;
    let mut reason = format!(
        "strategy={strategy}; confidence={confidence:.2}; risk={risk:.2}; effort={effort:.2}; source={source}; scope_marker={}; multi_repo={}; feedback_count={}; feedback_avg={}",
        if has_scope_marker { "yes" } else { "no" },
        if multi_repo { "yes" } else { "no" },
        feedback_signal.map(|s| s.feedback_count).unwrap_or(0),
        feedback_signal
            .map(|s| format!("{:.2}", s.avg_grade))
            .unwrap_or_else(|| "n/a".to_string())
    );
    if payload_parse_failed {
        reason.push_str("; WARNING: payload_json was unparseable");
    }

    RevenueOpportunityScore {
        confidence_score: confidence,
        effort_score: effort,
        risk_score: risk,
        priority_score: priority,
        recommended_approved,
        score_reason: reason,
    }
}

pub fn persist_revenue_opportunity_score(
    db: &Database,
    id: &str,
    score: &RevenueOpportunityScore,
) -> Result<bool> {
    let conn = db.conn();
    let updated = conn
        .execute(
            "UPDATE revenue_opportunities \
             SET confidence_score = ?2, effort_score = ?3, risk_score = ?4, priority_score = ?5, \
                 recommended_approved = ?6, score_reason = ?7, updated_at = datetime('now') \
             WHERE id = ?1",
            rusqlite::params![
                id,
                score.confidence_score,
                score.effort_score,
                score.risk_score,
                score.priority_score,
                if score.recommended_approved { 1 } else { 0 },
                score.score_reason,
            ],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_prefers_well_scoped_oracle_feed_work() {
        let score = score_revenue_opportunity(&RevenueOpportunityScoreInput {
            source: "trusted_feed_registry",
            strategy: "oracle_feed",
            payload_json: r#"{"pair":"ETH/USD","source_url":"https://example.com/feed"}"#,
            expected_revenue_usdc: 12.0,
            request_id: Some("job_1"),
        });
        assert!(score.recommended_approved);
        assert!(score.priority_score > 60.0);
        assert!(score.confidence_score > score.risk_score);
    }

    #[test]
    fn scoring_penalizes_underspecified_multi_repo_bounty_work() {
        let score = score_revenue_opportunity(&RevenueOpportunityScoreInput {
            source: "external_board",
            strategy: "micro_bounty",
            payload_json: r#"{"action":"multi-repo audit"}"#,
            expected_revenue_usdc: 1.0,
            request_id: None,
        });
        assert!(!score.recommended_approved);
        assert!(score.risk_score >= 0.45);
        assert!(score.effort_score >= 0.45);
    }

    #[test]
    fn scoring_uses_negative_feedback_to_reduce_priority() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) VALUES ('ro_1','a','micro_bounty','{}',3.0,'settled')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) VALUES ('ro_2','b','micro_bounty','{}',3.0,'settled')",
            [],
        )
        .unwrap();
        drop(conn);
        crate::revenue_feedback::record_revenue_feedback(&db, "ro_1", 1.5, "operator", None)
            .unwrap();
        crate::revenue_feedback::record_revenue_feedback(&db, "ro_2", 2.0, "operator", None)
            .unwrap();

        let input = RevenueOpportunityScoreInput {
            source: "external_board",
            strategy: "micro_bounty",
            payload_json: r#"{"repo":"example","action":"single repo audit"}"#,
            expected_revenue_usdc: 8.0,
            request_id: Some("job_1"),
        };
        let baseline = score_revenue_opportunity(&input);
        let adjusted = score_revenue_opportunity_with_feedback(&db, &input).unwrap();
        assert!(adjusted.priority_score < baseline.priority_score);
        assert!(adjusted.score_reason.contains("feedback_count=2"));
    }
}
