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
}
