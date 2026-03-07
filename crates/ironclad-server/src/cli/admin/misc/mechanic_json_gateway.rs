async fn collect_mechanic_json_gateway_findings(
    base_url: &str,
    ironclad_dir: &Path,
    repair: bool,
    allow_jobs: &[String],
    findings: &mut Vec<MechanicFinding>,
    actions: &mut RepairActionSummary,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = super::http_client()?
        .get(format!("{base_url}/api/health"))
        .send()
        .await;
    let gateway_up = matches!(gateway, Ok(ref resp) if resp.status().is_success());
    if !gateway_up {
        findings.push(finding(
            "gateway-unreachable",
            "high",
            0.95,
            "Gateway unreachable",
            format!("Could not reach {base_url}/api/health successfully."),
            "Start or restart the Ironclad daemon.",
            vec!["ironclad daemon restart".to_string()],
            false,
            false,
        ));
    } else {
        let diag_resp = super::http_client()?
            .get(format!("{base_url}/api/agent/status"))
            .send()
            .await?;
        let diagnostics: serde_json::Value = diag_resp.json().await.unwrap_or_default();
        if let Some(diag) = diagnostics.get("diagnostics") {
            let enabled = diag
                .get("taskable_subagents_enabled")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let running = diag
                .get("taskable_subagents_running")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if enabled > 0 && running == 0 {
                findings.push(finding(
                    "delegation-integrity-down",
                    "critical",
                    0.99,
                    "Delegation integrity failure",
                    format!(
                        "{enabled} subagent(s) enabled but none running; delegated output cannot be verified."
                    ),
                    "Recover/start subagents before accepting subagent-attributed responses.",
                    vec!["ironclad status".to_string(), "ironclad mechanic".to_string()],
                    false,
                    false,
                ));
            }
        }

        let channels_resp = super::http_client()?
            .get(format!("{base_url}/api/channels/status"))
            .send()
            .await?;
        let channels: Vec<serde_json::Value> = channels_resp.json().await.unwrap_or_default();
        if let Some(tg) = channels
            .iter()
            .find(|c| c.get("name").and_then(|v| v.as_str()) == Some("telegram"))
        {
            let connected = tg
                .get("connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let rx = tg
                .get("messages_received")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let tx = tg
                .get("messages_sent")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if connected && rx == 0 && tx == 0 {
                findings.push(finding(
                    "telegram-idle",
                    "medium",
                    0.75,
                    "Telegram connected but zero traffic",
                    "No messages received/sent; verify token, polling/webhook, and chat allowlist.",
                    "Inspect channel status and logs for transport/auth errors.",
                    vec![
                        "ironclad channels status".to_string(),
                        "ironclad logs -n 200".to_string(),
                    ],
                    false,
                    false,
                ));
            }
        }

        match fetch_provider_health(base_url).await {
            Ok(rows) if rows.is_empty() => {
                findings.push(finding(
                    "provider-health-empty",
                    "medium",
                    0.85,
                    "Provider health check returned no providers",
                    "No provider status records were returned by /api/models/available.",
                    "Verify providers are configured and reachable from the runtime.",
                    vec![provider_scan_hint(None)],
                    false,
                    false,
                ));
            }
            Ok(rows) => {
                for row in rows {
                    match row.status.as_str() {
                        "ok" if row.count > 0 => {}
                        "ok" => findings.push(finding(
                            "provider-health-no-models",
                            "medium",
                            0.88,
                            format!("Provider '{}' reachable but no models discovered", row.name),
                            "Provider endpoint responded successfully but model list is empty.",
                            "Check provider model inventory and credentials.",
                            vec![provider_scan_hint(Some(&row.name))],
                            false,
                            false,
                        )),
                        "unreachable" | "error" => findings.push(finding(
                            "provider-health-unavailable",
                            "high",
                            0.93,
                            format!("Provider '{}' is {}", row.name, row.status),
                            row.error.unwrap_or_else(|| "provider route is not healthy".to_string()),
                            "Restore provider connectivity/auth so fallback routing can continue automatically.",
                            vec![
                                provider_scan_hint(Some(&row.name)),
                                "ironclad mechanic --repair".to_string(),
                            ],
                            false,
                            false,
                        )),
                        other => findings.push(finding(
                            "provider-health-unknown",
                            "medium",
                            0.8,
                            format!("Provider '{}' reported status '{}'", row.name, other),
                            row.error.unwrap_or_else(|| "unknown provider health state".to_string()),
                            "Inspect provider configuration and discovery path.",
                            vec![provider_scan_hint(Some(&row.name))],
                            false,
                            false,
                        )),
                    }
                }
            }
            Err(e) => {
                findings.push(finding(
                    "provider-health-check-failed",
                    "medium",
                    0.9,
                    "Provider health check failed",
                    format!("Could not query /api/models/available: {e}"),
                    "Inspect gateway and provider discovery endpoint health.",
                    vec![provider_scan_hint(None)],
                    false,
                    false,
                ));
            }
        }

        let revenue_probe = probe_revenue_control_plane(&ironclad_dir.join("state.db"), repair);
        match revenue_probe {
            Ok(health) if health.opportunities_total == 0 => {}
            Ok(health) => {
                if health.orphan_jobs > 0 {
                    findings.push(finding(
                        "revenue-orphan-jobs",
                        "high",
                        0.92,
                        format!(
                            "Revenue control plane has {} orphan opportunity job(s)",
                            health.orphan_jobs
                        ),
                        "Opportunities reference missing service request IDs, breaking end-to-end lifecycle consistency.",
                        "Run mechanic repair to mark orphaned revenue jobs failed and restore consistency.",
                        vec!["ironclad mechanic --repair".to_string()],
                        true,
                        health.repaired_orphans > 0,
                    ));
                }
                if health.missing_settlement_ledger > 0 {
                    findings.push(finding(
                        "revenue-ledger-reconcile",
                        "medium",
                        0.9,
                        format!(
                            "Revenue settlement ledger missing {} entr{}",
                            health.missing_settlement_ledger,
                            if health.missing_settlement_ledger == 1 {
                                "y"
                            } else {
                                "ies"
                            }
                        ),
                        "Settled opportunities exist without corresponding revenue_settlement transactions.",
                        "Run mechanic repair to reconcile missing settlement ledger rows.",
                        vec!["ironclad mechanic --repair".to_string()],
                        true,
                        health.reconciled_ledger_rows > 0,
                    ));
                }
            }
            Err(e) => findings.push(finding(
                "revenue-probe-failed",
                "medium",
                0.85,
                "Revenue control-plane probe failed",
                format!("{e}"),
                "Inspect state.db health and revenue tables.",
                vec![
                    "ironclad defrag".to_string(),
                    "ironclad mechanic".to_string(),
                ],
                false,
                false,
            )),
        }

        if repair && !allow_jobs.is_empty() {
            let jobs_resp = super::http_client()?
                .get(format!("{base_url}/api/cron/jobs"))
                .send()
                .await?;
            if jobs_resp.status().is_success() {
                let payload: serde_json::Value = jobs_resp.json().await.unwrap_or_default();
                let jobs = payload
                    .get("jobs")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let allowset: std::collections::HashSet<String> =
                    allow_jobs.iter().map(|s| s.to_string()).collect();
                let client = super::http_client()?;
                for job in jobs {
                    let name = job.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let id = job.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let paused = job
                        .get("last_status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        == "paused_unknown_action";
                    if paused && allowset.contains(name) && !id.is_empty() {
                        let resp = client
                            .put(format!("{base_url}/api/cron/jobs/{id}"))
                            .json(&serde_json::json!({ "enabled": true }))
                            .send()
                            .await?;
                        if resp.status().is_success() {
                            actions.paused_jobs_reenabled.push(name.to_string());
                        }
                    }
                }
                if !actions.paused_jobs_reenabled.is_empty() {
                    findings.push(MechanicFinding {
                        id: "paused-jobs-recovered".to_string(),
                        severity: "info".to_string(),
                        confidence: 1.0,
                        summary: "Paused cron jobs recovered".to_string(),
                        details: format!(
                            "Re-enabled allowlisted jobs: {}",
                            actions.paused_jobs_reenabled.join(", ")
                        ),
                        repair_plan: MechanicRepairPlan {
                            description: "Allowlisted paused jobs were re-enabled.".to_string(),
                            commands: vec![],
                            safe_auto_repair: true,
                            requires_human_approval: false,
                        },
                        auto_repaired: true,
                    });
                }
            }
        }
    }
    Ok(())
}

