#[derive(Debug, Default, Clone)]
struct RevenueTaxReconcileHealth {
    submitted_tasks: usize,
    pending_receipts: usize,
    confirmed_repairs: usize,
    failed_repairs: usize,
}

async fn probe_revenue_tax_reconcile(
    base_url: &str,
    repair: bool,
) -> Result<RevenueTaxReconcileHealth, Box<dyn std::error::Error>> {
    let resp = super::http_client()?
        .get(format!("{base_url}/api/services/tax-payouts?limit=200"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(std::io::Error::other(format!(
            "tax payout listing endpoint returned HTTP {}",
            resp.status()
        ))
        .into());
    }
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let tasks = body
        .get("tax_tasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut health = RevenueTaxReconcileHealth::default();
    for task in tasks {
        let status = task
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let source = task
            .get("source")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let tax_tx_hash = source
            .get("tax_tx_hash")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if !status.eq_ignore_ascii_case("in_progress") || tax_tx_hash.is_none() {
            continue;
        }
        health.submitted_tasks += 1;
        if !repair {
            continue;
        }
        let opportunity_id = task
            .get("opportunity_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if opportunity_id.is_empty() {
            continue;
        }
        let reconcile = super::http_client()?
            .post(format!(
                "{base_url}/api/services/tax-payouts/{}/reconcile",
                opportunity_id
            ))
            .send()
            .await?;
        if !reconcile.status().is_success() {
            continue;
        }
        let reconcile_body: serde_json::Value = reconcile.json().await.unwrap_or_default();
        match reconcile_body
            .get("receipt_status")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
        {
            "confirmed" => health.confirmed_repairs += 1,
            "failed" => health.failed_repairs += 1,
            "pending" => health.pending_receipts += 1,
            _ => {}
        }
    }
    Ok(health)
}
