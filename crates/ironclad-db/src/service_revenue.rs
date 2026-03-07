use crate::Database;
use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub struct ServiceRequestRecord {
    pub id: String,
    pub service_id: String,
    pub requester: String,
    pub parameters_json: String,
    pub status: String,
    pub quoted_amount: f64,
    pub currency: String,
    pub recipient: String,
    pub quote_expires_at: String,
    pub payment_tx_hash: Option<String>,
    pub paid_amount: Option<f64>,
    pub payment_verified_at: Option<String>,
    pub fulfillment_output: Option<String>,
    pub fulfilled_at: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewServiceRequest<'a> {
    pub id: &'a str,
    pub service_id: &'a str,
    pub requester: &'a str,
    pub parameters_json: &'a str,
    pub quoted_amount: f64,
    pub currency: &'a str,
    pub recipient: &'a str,
    pub quote_expires_at: &'a str,
}

pub const STATUS_QUOTED: &str = "quoted";
pub const STATUS_PAYMENT_VERIFIED: &str = "payment_verified";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";

pub const OPPORTUNITY_STATUS_INTAKE: &str = "intake";
pub const OPPORTUNITY_STATUS_QUALIFIED: &str = "qualified";
pub const OPPORTUNITY_STATUS_REJECTED: &str = "rejected";
pub const OPPORTUNITY_STATUS_PLANNED: &str = "planned";
pub const OPPORTUNITY_STATUS_FULFILLED: &str = "fulfilled";
pub const OPPORTUNITY_STATUS_SETTLED: &str = "settled";

#[derive(Debug, Clone)]
pub struct NewRevenueOpportunity<'a> {
    pub id: &'a str,
    pub source: &'a str,
    pub strategy: &'a str,
    pub payload_json: &'a str,
    pub expected_revenue_usdc: f64,
    pub request_id: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct RevenueOpportunityRecord {
    pub id: String,
    pub source: String,
    pub strategy: String,
    pub payload_json: String,
    pub expected_revenue_usdc: f64,
    pub status: String,
    pub qualification_reason: Option<String>,
    pub plan_json: Option<String>,
    pub evidence_json: Option<String>,
    pub request_id: Option<String>,
    pub settlement_ref: Option<String>,
    pub settled_amount_usdc: Option<f64>,
    pub attributable_costs_usdc: f64,
    pub net_profit_usdc: Option<f64>,
    pub tax_rate: f64,
    pub tax_amount_usdc: f64,
    pub retained_earnings_usdc: Option<f64>,
    pub tax_destination_wallet: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct RevenueSettlementAccounting<'a> {
    pub attributable_costs_usdc: f64,
    pub tax_rate: f64,
    pub tax_amount_usdc: f64,
    pub retained_earnings_usdc: f64,
    pub tax_destination_wallet: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementResult {
    Settled,
    AlreadySettled,
}

pub fn create_service_request(db: &Database, req: &NewServiceRequest<'_>) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO service_requests \
         (id, service_id, requester, parameters_json, status, quoted_amount, currency, recipient, quote_expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            req.id,
            req.service_id,
            req.requester,
            req.parameters_json,
            STATUS_QUOTED,
            req.quoted_amount,
            req.currency,
            req.recipient,
            req.quote_expires_at
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn get_service_request(db: &Database, id: &str) -> Result<Option<ServiceRequestRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, service_id, requester, parameters_json, status, quoted_amount, currency, recipient, \
                    quote_expires_at, payment_tx_hash, paid_amount, payment_verified_at, fulfillment_output, \
                    fulfilled_at, failure_reason, created_at, updated_at \
             FROM service_requests WHERE id = ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let row = stmt
        .query_row([id], |row| {
            Ok(ServiceRequestRecord {
                id: row.get(0)?,
                service_id: row.get(1)?,
                requester: row.get(2)?,
                parameters_json: row.get(3)?,
                status: row.get(4)?,
                quoted_amount: row.get(5)?,
                currency: row.get(6)?,
                recipient: row.get(7)?,
                quote_expires_at: row.get(8)?,
                payment_tx_hash: row.get(9)?,
                paid_amount: row.get(10)?,
                payment_verified_at: row.get(11)?,
                fulfillment_output: row.get(12)?,
                fulfilled_at: row.get(13)?,
                failure_reason: row.get(14)?,
                created_at: row.get(15)?,
                updated_at: row.get(16)?,
            })
        })
        .optional()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(row)
}

pub fn mark_payment_verified(
    db: &Database,
    id: &str,
    tx_hash: &str,
    paid_amount: f64,
) -> Result<bool> {
    let conn = db.conn();
    let updated = conn
        .execute(
            "UPDATE service_requests \
             SET status = ?2, payment_tx_hash = ?3, paid_amount = ?4, payment_verified_at = datetime('now'), \
                 updated_at = datetime('now') \
             WHERE id = ?1 AND status = ?5",
            rusqlite::params![
                id,
                STATUS_PAYMENT_VERIFIED,
                tx_hash,
                paid_amount,
                STATUS_QUOTED
            ],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

pub fn mark_fulfilled(db: &Database, id: &str, fulfillment_output: &str) -> Result<bool> {
    let conn = db.conn();
    let updated = conn
        .execute(
            "UPDATE service_requests \
             SET status = ?2, fulfillment_output = ?3, fulfilled_at = datetime('now'), \
                 updated_at = datetime('now') \
             WHERE id = ?1 AND status = ?4",
            rusqlite::params![
                id,
                STATUS_COMPLETED,
                fulfillment_output,
                STATUS_PAYMENT_VERIFIED
            ],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

pub fn create_revenue_opportunity(db: &Database, opp: &NewRevenueOpportunity<'_>) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO revenue_opportunities \
         (id, source, strategy, payload_json, expected_revenue_usdc, status, request_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            opp.id,
            opp.source,
            opp.strategy,
            opp.payload_json,
            opp.expected_revenue_usdc,
            OPPORTUNITY_STATUS_INTAKE,
            opp.request_id
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn get_revenue_opportunity(
    db: &Database,
    id: &str,
) -> Result<Option<RevenueOpportunityRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, source, strategy, payload_json, expected_revenue_usdc, status, qualification_reason, \
                    plan_json, evidence_json, request_id, settlement_ref, settled_amount_usdc, attributable_costs_usdc, \
                    net_profit_usdc, tax_rate, tax_amount_usdc, retained_earnings_usdc, tax_destination_wallet, created_at, updated_at \
             FROM revenue_opportunities WHERE id = ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let row = stmt
        .query_row([id], |row| {
            Ok(RevenueOpportunityRecord {
                id: row.get(0)?,
                source: row.get(1)?,
                strategy: row.get(2)?,
                payload_json: row.get(3)?,
                expected_revenue_usdc: row.get(4)?,
                status: row.get(5)?,
                qualification_reason: row.get(6)?,
                plan_json: row.get(7)?,
                evidence_json: row.get(8)?,
                request_id: row.get(9)?,
                settlement_ref: row.get(10)?,
                settled_amount_usdc: row.get(11)?,
                attributable_costs_usdc: row.get(12)?,
                net_profit_usdc: row.get(13)?,
                tax_rate: row.get(14)?,
                tax_amount_usdc: row.get(15)?,
                retained_earnings_usdc: row.get(16)?,
                tax_destination_wallet: row.get(17)?,
                created_at: row.get(18)?,
                updated_at: row.get(19)?,
            })
        })
        .optional()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(row)
}

pub fn qualify_revenue_opportunity(
    db: &Database,
    id: &str,
    approved: bool,
    reason: Option<&str>,
) -> Result<bool> {
    let conn = db.conn();
    let status = if approved {
        OPPORTUNITY_STATUS_QUALIFIED
    } else {
        OPPORTUNITY_STATUS_REJECTED
    };
    let updated = conn
        .execute(
            "UPDATE revenue_opportunities \
             SET status = ?2, qualification_reason = ?3, updated_at = datetime('now') \
             WHERE id = ?1 AND status = ?4",
            rusqlite::params![id, status, reason, OPPORTUNITY_STATUS_INTAKE],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

pub fn plan_revenue_opportunity(db: &Database, id: &str, plan_json: &str) -> Result<bool> {
    let conn = db.conn();
    let updated = conn
        .execute(
            "UPDATE revenue_opportunities \
             SET status = ?2, plan_json = ?3, updated_at = datetime('now') \
             WHERE id = ?1 AND status = ?4",
            rusqlite::params![
                id,
                OPPORTUNITY_STATUS_PLANNED,
                plan_json,
                OPPORTUNITY_STATUS_QUALIFIED
            ],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

pub fn mark_revenue_opportunity_fulfilled(
    db: &Database,
    id: &str,
    evidence_json: &str,
) -> Result<bool> {
    let conn = db.conn();
    let updated = conn
        .execute(
            "UPDATE revenue_opportunities \
             SET status = ?2, evidence_json = ?3, updated_at = datetime('now') \
             WHERE id = ?1 AND status = ?4",
            rusqlite::params![
                id,
                OPPORTUNITY_STATUS_FULFILLED,
                evidence_json,
                OPPORTUNITY_STATUS_PLANNED
            ],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(updated > 0)
}

pub fn settle_revenue_opportunity(
    db: &Database,
    id: &str,
    settlement_ref: &str,
    settled_amount_usdc: f64,
    accounting: &RevenueSettlementAccounting<'_>,
) -> Result<SettlementResult> {
    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let existing: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT status, settlement_ref FROM revenue_opportunities WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let Some((status, existing_ref)) = existing else {
        return Err(IroncladError::Database(format!(
            "revenue opportunity '{id}' not found"
        )));
    };
    if status.eq_ignore_ascii_case(OPPORTUNITY_STATUS_SETTLED)
        && existing_ref.as_deref() == Some(settlement_ref)
    {
        tx.commit()
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        return Ok(SettlementResult::AlreadySettled);
    }
    let updated = tx
        .execute(
            "UPDATE revenue_opportunities \
             SET status = ?2, settlement_ref = ?3, settled_amount_usdc = ?4, attributable_costs_usdc = ?5, \
                 net_profit_usdc = (?4 - ?5), tax_rate = ?6, tax_amount_usdc = ?7, retained_earnings_usdc = ?8, \
                 tax_destination_wallet = ?9, updated_at = datetime('now') \
             WHERE id = ?1 AND status = ?10",
            rusqlite::params![
                id,
                OPPORTUNITY_STATUS_SETTLED,
                settlement_ref,
                settled_amount_usdc,
                accounting.attributable_costs_usdc,
                accounting.tax_rate,
                accounting.tax_amount_usdc,
                accounting.retained_earnings_usdc,
                accounting.tax_destination_wallet,
                OPPORTUNITY_STATUS_FULFILLED
            ],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    if updated == 0 {
        return Err(IroncladError::Database(
            "revenue opportunity must be fulfilled before settlement".to_string(),
        ));
    }
    tx.commit()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(SettlementResult::Settled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn service_request_lifecycle() {
        let db = test_db();
        let req = NewServiceRequest {
            id: "sr_1",
            service_id: "geopolitical-sitrep-verified",
            requester: "tester",
            parameters_json: r#"{"scope":"us"}"#,
            quoted_amount: 0.25,
            currency: "USDC",
            recipient: "0x0000000000000000000000000000000000000001",
            quote_expires_at: "2099-01-01T00:00:00Z",
        };
        create_service_request(&db, &req).unwrap();

        let fetched = get_service_request(&db, "sr_1").unwrap().unwrap();
        assert_eq!(fetched.status, STATUS_QUOTED);

        assert!(mark_payment_verified(&db, "sr_1", "0xabc", 0.25).unwrap());
        let fetched = get_service_request(&db, "sr_1").unwrap().unwrap();
        assert_eq!(fetched.status, STATUS_PAYMENT_VERIFIED);
        assert_eq!(fetched.payment_tx_hash.as_deref(), Some("0xabc"));

        assert!(mark_fulfilled(&db, "sr_1", "done").unwrap());
        let fetched = get_service_request(&db, "sr_1").unwrap().unwrap();
        assert_eq!(fetched.status, STATUS_COMPLETED);
        assert_eq!(fetched.fulfillment_output.as_deref(), Some("done"));
    }

    #[test]
    fn mark_payment_verified_requires_quoted_status() {
        let db = test_db();
        let req = NewServiceRequest {
            id: "sr_2",
            service_id: "geopolitical-sitrep-verified",
            requester: "tester",
            parameters_json: "{}",
            quoted_amount: 0.25,
            currency: "USDC",
            recipient: "0x0000000000000000000000000000000000000001",
            quote_expires_at: "2099-01-01T00:00:00Z",
        };
        create_service_request(&db, &req).unwrap();
        assert!(mark_payment_verified(&db, "sr_2", "0xabc", 0.25).unwrap());
        assert!(!mark_payment_verified(&db, "sr_2", "0xdef", 0.25).unwrap());
    }

    #[test]
    fn revenue_opportunity_lifecycle_with_idempotent_settlement() {
        let db = test_db();
        create_revenue_opportunity(
            &db,
            &NewRevenueOpportunity {
                id: "ro_1",
                source: "micro_bounty_board",
                strategy: "micro_bounty",
                payload_json: r#"{"issue_id":"123"}"#,
                expected_revenue_usdc: 2.5,
                request_id: Some("job_1"),
            },
        )
        .unwrap();
        assert!(qualify_revenue_opportunity(&db, "ro_1", true, Some("eligible")).unwrap());
        assert!(plan_revenue_opportunity(&db, "ro_1", r#"{"executor":"self"}"#).unwrap());
        assert!(mark_revenue_opportunity_fulfilled(&db, "ro_1", r#"{"proof":"ok"}"#).unwrap());
        assert_eq!(
            settle_revenue_opportunity(
                &db,
                "ro_1",
                "tx_1",
                2.5,
                &RevenueSettlementAccounting {
                    attributable_costs_usdc: 0.5,
                    tax_rate: 0.1,
                    tax_amount_usdc: 0.2,
                    retained_earnings_usdc: 1.8,
                    tax_destination_wallet: Some("0x123"),
                },
            )
            .unwrap(),
            SettlementResult::Settled
        );
        assert_eq!(
            settle_revenue_opportunity(
                &db,
                "ro_1",
                "tx_1",
                2.5,
                &RevenueSettlementAccounting {
                    attributable_costs_usdc: 0.5,
                    tax_rate: 0.1,
                    tax_amount_usdc: 0.2,
                    retained_earnings_usdc: 1.8,
                    tax_destination_wallet: Some("0x123"),
                },
            )
            .unwrap(),
            SettlementResult::AlreadySettled
        );
        let row = get_revenue_opportunity(&db, "ro_1").unwrap().unwrap();
        assert_eq!(row.status, OPPORTUNITY_STATUS_SETTLED);
        assert_eq!(row.settlement_ref.as_deref(), Some("tx_1"));
        assert!((row.attributable_costs_usdc - 0.5).abs() < f64::EPSILON);
        assert_eq!(row.tax_destination_wallet.as_deref(), Some("0x123"));
    }
}
