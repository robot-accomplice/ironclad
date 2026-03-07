async fn run_mechanic_text_gateway_checks(
    base_url: &str,
    ironclad_dir: &Path,
    repair: bool,
    allow_jobs: &[String],
    fixed: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, BOLD, _, _, _, _, _, RESET, _) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let gateway_up = match super::http_client()?
        .get(format!("{base_url}/api/health"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            println!("  {OK} Gateway reachable at {base_url}");
            true
        }
        Ok(resp) => {
            println!("  {WARN} Gateway returned HTTP {}", resp.status());
            false
        }
        Err(_) => {
            println!("  {WARN} Gateway not running at {base_url}");
            false
        }
    };

    if gateway_up {
        let mut channels_status: Option<Vec<serde_json::Value>> = None;
        let mut runtime_diag: Option<serde_json::Value> = None;

        run_gateway_config_and_diag_checks(base_url, &mut runtime_diag).await?;
        run_gateway_skill_checks(base_url, ironclad_dir, repair, fixed).await?;
        run_gateway_plugin_checks(ironclad_dir, repair, fixed)?;
        run_gateway_wallet_and_channel_checks(base_url, &mut channels_status).await?;
        run_gateway_provider_and_revenue_checks(base_url, ironclad_dir, repair).await;
        run_gateway_log_and_runtime_diagnostics(ironclad_dir, channels_status.as_ref(), runtime_diag.as_ref());
        run_gateway_allowlisted_job_recovery(base_url, repair, allow_jobs, fixed).await?;
    } else {
        println!("    {DETAIL} Skipping server checks (config, skills, wallet, channels)");
    }

    if repair {
        println!("\n  {BOLD}Mechanic Integrated Sweep{RESET}\n");
        run_gateway_integrated_repair_sweep(base_url, ironclad_dir, gateway_up).await?;
    }

    Ok(())
}

async fn run_gateway_config_and_diag_checks(
    base_url: &str,
    runtime_diag: &mut Option<serde_json::Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (OK, _, WARN, _, _) = icons();
    match super::http_client()?
        .get(format!("{base_url}/api/config"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => println!("  {OK} Configuration loaded on server"),
        Ok(resp) => println!("  {WARN} Config endpoint returned HTTP {}", resp.status()),
        Err(e) => println!("  {WARN} Config check failed: {e}"),
    }

    match super::http_client()?
        .get(format!("{base_url}/api/agent/status"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            *runtime_diag = body.get("diagnostics").cloned();
            println!("  {OK} Runtime diagnostics available");
        }
        Ok(resp) => println!("  {WARN} Agent status endpoint returned HTTP {}", resp.status()),
        Err(e) => println!("  {WARN} Agent status check failed: {e}"),
    }
    Ok(())
}

async fn run_gateway_skill_checks(
    base_url: &str,
    ironclad_dir: &Path,
    repair: bool,
    fixed: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let (OK, ACTION, WARN, DETAIL, _) = icons();
    if repair {
        match super::http_client()?
            .post(format!("{base_url}/api/skills/reload"))
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                println!("  {ACTION} Reloaded skills from disk to repair skill DB drift");
                *fixed += 1;
            }
            Ok(resp) => println!("  {WARN} Skills reload failed during repair (HTTP {})", resp.status()),
            Err(e) => println!("  {WARN} Skills reload failed during repair: {e}"),
        }
    }

    match super::http_client()?
        .get(format!("{base_url}/api/skills"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let count = body
                .get("skills")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if count == 0 {
                println!("  {WARN} Skills loaded (0 skills) — builtin skills may be missing from DB");
            } else {
                println!("  {OK} Skills loaded ({count} skills)");
            }
            let db_parity = evaluate_capability_skill_parity(&ironclad_dir.join("state.db"));
            if db_parity.missing_in_db.is_empty() {
                println!("  {OK} Loaded skill DB satisfies capability-to-skill parity");
            } else {
                println!("  {WARN} Loaded skill DB missing required capability skills");
                println!("    {DETAIL} {}", db_parity.missing_in_db.join("; "));
            }
        }
        Ok(resp) => println!("  {WARN} Skills endpoint returned HTTP {}", resp.status()),
        Err(e) => println!("  {WARN} Skills check failed: {e}"),
    }
    Ok(())
}

fn run_gateway_plugin_checks(
    ironclad_dir: &Path,
    repair: bool,
    fixed: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    use ironclad_plugin_sdk::manifest::PluginManifest;

    let (OK, ACTION, WARN, _, ERR) = icons();
    let plugins_dir = ironclad_dir.join("plugins");
    if !plugins_dir.exists() {
        return Ok(());
    }

    let mut orphan_dirs: Vec<PathBuf> = Vec::new();
    let mut valid_plugins: Vec<(PathBuf, PluginManifest)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.toml");
            if !manifest_path.exists() {
                orphan_dirs.push(path);
                continue;
            }
            match PluginManifest::from_file(&manifest_path) {
                Ok(manifest) => valid_plugins.push((path, manifest)),
                Err(_) => orphan_dirs.push(path),
            }
        }
    }

    if orphan_dirs.is_empty() && valid_plugins.is_empty() {
        println!("  {OK} Plugins directory empty (no plugins installed)");
        return Ok(());
    }

    for orphan in &orphan_dirs {
        let dir_name = orphan.file_name().unwrap_or_default().to_string_lossy();
        if repair {
            if prompt_yes_no(&format!(
                "  Remove orphan plugin directory '{dir_name}'? (no valid plugin.toml)"
            )) {
                if let Err(e) = std::fs::remove_dir_all(orphan) {
                    println!("  {ERR} Failed to remove {}: {e}", orphan.display());
                } else {
                    println!("  {ACTION} Removed orphan plugin directory: {dir_name}");
                    *fixed += 1;
                }
            }
        } else {
            println!("  {WARN} Orphan plugin directory: {dir_name} (no valid plugin.toml — use --repair to remove)");
        }
    }

    let skills_dir = ironclad_dir.join("skills");
    for (plugin_dir, manifest) in &valid_plugins {
        let report = manifest.vet(plugin_dir);
        if report.is_ok() && report.warnings.is_empty() {
            println!("  {OK} Plugin '{}' v{} — healthy", manifest.name, manifest.version);
        } else {
            for w in &report.warnings {
                println!("  {WARN} Plugin '{}': {w}", manifest.name);
            }
            for e in &report.errors {
                println!("  {ERR} Plugin '{}': {e}", manifest.name);
            }
        }
        if repair {
            for skill_rel in &manifest.companion_skills {
                let src = plugin_dir.join(skill_rel);
                let installed_name =
                    super::plugins::companion_skill_install_name(&manifest.name, skill_rel);
                let dest = skills_dir.join(&installed_name);
                if src.exists() && !dest.exists() {
                    std::fs::create_dir_all(&skills_dir).ok();
                    if let Err(e) = std::fs::copy(&src, &dest) {
                        println!("  {ERR} Failed to re-deploy companion skill {installed_name}: {e}");
                    } else {
                        println!("  {ACTION} Re-deployed missing companion skill: {installed_name}");
                        *fixed += 1;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn run_gateway_wallet_and_channel_checks(
    base_url: &str,
    channels_status: &mut Option<Vec<serde_json::Value>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (OK, _, WARN, _, _) = icons();
    match super::http_client()?
        .get(format!("{base_url}/api/wallet/balance"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => println!("  {OK} Wallet accessible"),
        Ok(resp) => println!("  {WARN} Wallet endpoint returned HTTP {}", resp.status()),
        Err(e) => println!("  {WARN} Wallet check failed: {e}"),
    }

    match super::http_client()?
        .get(format!("{base_url}/api/channels/status"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
            let active = body
                .iter()
                .filter(|c| c.get("connected").and_then(|v| v.as_bool()).unwrap_or(false))
                .count();
            println!("  {OK} Channels ({active}/{} connected)", body.len());
            *channels_status = Some(body);
        }
        Ok(resp) => println!("  {WARN} Channels endpoint returned HTTP {}", resp.status()),
        Err(e) => println!("  {WARN} Channels check failed: {e}"),
    }
    Ok(())
}

async fn run_gateway_provider_and_revenue_checks(base_url: &str, ironclad_dir: &Path, repair: bool) {
    let (OK, ACTION, WARN, DETAIL, _) = icons();
    match fetch_provider_health(base_url).await {
        Ok(rows) if rows.is_empty() => println!("  {WARN} Provider health check returned no providers"),
        Ok(rows) => {
            println!(
                "  {OK} Provider health check completed ({} provider{})",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" }
            );
            for row in rows {
                match row.status.as_str() {
                    "ok" if row.count > 0 => println!(
                        "    {OK} {}: reachable ({} model{})",
                        row.name,
                        row.count,
                        if row.count == 1 { "" } else { "s" }
                    ),
                    "ok" => {
                        println!("    {WARN} {}: reachable but no models discovered", row.name);
                        println!("      {DETAIL} Probe route: `{}`", provider_scan_hint(Some(&row.name)));
                    }
                    "unreachable" | "error" => {
                        let detail = row.error.as_deref().unwrap_or("unknown provider error");
                        println!("    {WARN} {}: {} ({detail})", row.name, row.status);
                        println!("      {DETAIL} Probe route: `{}`", provider_scan_hint(Some(&row.name)));
                    }
                    other => {
                        let detail = row.error.as_deref().unwrap_or("no extra detail");
                        println!("    {WARN} {}: {other} ({detail})", row.name);
                        println!("      {DETAIL} Probe route: `{}`", provider_scan_hint(Some(&row.name)));
                    }
                }
            }
        }
        Err(e) => println!("  {WARN} Provider health check failed: {e}"),
    }

    match probe_revenue_control_plane(&ironclad_dir.join("state.db"), repair) {
        Ok(health) if health.opportunities_total == 0 => {
            println!("  {OK} Revenue control plane: no opportunities recorded yet");
        }
        Ok(health) => {
            println!(
                "  {OK} Revenue control plane: {} opportunities ({} settled)",
                health.opportunities_total, health.opportunities_settled
            );
            if health.orphan_jobs > 0 {
                println!(
                    "    {WARN} Found {} orphan revenue opportunit{}",
                    health.orphan_jobs,
                    if health.orphan_jobs == 1 { "y" } else { "ies" }
                );
                if repair && health.repaired_orphans > 0 {
                    println!(
                        "    {ACTION} Repaired {} orphan opportunit{} (marked failed)",
                        health.repaired_orphans,
                        if health.repaired_orphans == 1 { "y" } else { "ies" }
                    );
                }
            }
            if health.missing_settlement_ledger > 0 {
                println!(
                    "    {WARN} Found {} settled opportunit{} missing ledger entries",
                    health.missing_settlement_ledger,
                    if health.missing_settlement_ledger == 1 { "y" } else { "ies" }
                );
                if repair && health.reconciled_ledger_rows > 0 {
                    println!(
                        "    {ACTION} Reconciled {} missing revenue settlement ledger entr{}",
                        health.reconciled_ledger_rows,
                        if health.reconciled_ledger_rows == 1 { "y" } else { "ies" }
                    );
                }
            }
            if health.stale_revenue_tasks > 0 {
                println!(
                    "    {WARN} Found {} stale revenue task{} stuck in in_progress",
                    health.stale_revenue_tasks,
                    if health.stale_revenue_tasks == 1 { "" } else { "s" }
                );
                if repair && health.reset_stale_revenue_tasks > 0 {
                    println!(
                        "    {ACTION} Reset {} stale revenue task{} back to pending",
                        health.reset_stale_revenue_tasks,
                        if health.reset_stale_revenue_tasks == 1 { "" } else { "s" }
                    );
                }
            }
        }
        Err(e) => println!("  {WARN} Revenue control-plane probe failed: {e}"),
    }
}

fn run_gateway_log_and_runtime_diagnostics(
    ironclad_dir: &Path,
    channels_status: Option<&Vec<serde_json::Value>>,
    runtime_diag: Option<&serde_json::Value>,
) {
    let (_, _, WARN, _, _) = icons();
    let log_snapshot = recent_log_snapshot(&ironclad_dir.join("logs"), 350_000);
    if let Some(snapshot) = log_snapshot.as_deref() {
        let tg_404_count =
            count_occurrences(snapshot, "Telegram API error\",\"status\":\"404 Not Found");
        let tg_poll_err_count =
            count_occurrences(snapshot, "Telegram poll error, backing off 5s");
        if tg_404_count >= 3 || tg_poll_err_count >= 3 {
            println!("  {WARN} Detected repeated Telegram transport failures (404/poll backoff loop).");
            println!("         Likely cause: invalid/revoked Telegram bot token in keystore.");
            println!("         Repair: `ironclad keystore set telegram_bot_token \"<TOKEN>\"` then `ironclad daemon restart`");
        }

        let unknown_action_count = count_occurrences(snapshot, "unknown action: unknown");
        if unknown_action_count >= 3 {
            println!("  {WARN} Detected recurring scheduler failures: `unknown action: unknown`.");
            println!("         Repair: run `ironclad schedule recover --all --dry-run` and re-enable trusted jobs.");
        }
    }

    if let Some(channels) = channels_status {
        let telegram = channels
            .iter()
            .find(|c| c.get("name").and_then(|v| v.as_str()) == Some("telegram"));
        if let Some(tg) = telegram {
            let connected = tg.get("connected").and_then(|v| v.as_bool()).unwrap_or(false);
            let received = tg.get("messages_received").and_then(|v| v.as_i64()).unwrap_or(0);
            let sent = tg.get("messages_sent").and_then(|v| v.as_i64()).unwrap_or(0);
            if connected && received == 0 && sent == 0 {
                println!("  {WARN} Telegram appears connected but has zero traffic.");
                println!("         If this is unexpected, run `ironclad channels status` and inspect logs for poll/webhook errors.");
            }
        }
    }

    if let Some(diag) = runtime_diag {
        let total = diag.get("taskable_subagents_total").and_then(|v| v.as_u64()).unwrap_or(0);
        let enabled = diag.get("taskable_subagents_enabled").and_then(|v| v.as_u64()).unwrap_or(0);
        let running = diag.get("taskable_subagents_running").and_then(|v| v.as_u64()).unwrap_or(0);
        let error = diag.get("taskable_subagents_error").and_then(|v| v.as_u64()).unwrap_or(0);

        if total > 0 && enabled > 0 && running == 0 {
            println!("  {WARN} Delegation integrity risk: {enabled} taskable subagent(s) enabled, but 0 running.");
            println!("         Any response attributed to a subagent cannot be runtime-verified right now.");
            println!("         Repair: start/recover subagents and re-check with `ironclad status` / `ironclad mechanic`.");
        } else if enabled > running {
            println!("  {WARN} Delegation degradation: enabled subagents ({enabled}) exceed running ({running}).");
            if error > 0 {
                println!("         {error} subagent(s) currently report error state.");
            }
            println!("         Recommendation: treat subagent-attributed outputs as unverified until running count recovers.");
        }
    }
}

async fn run_gateway_allowlisted_job_recovery(
    base_url: &str,
    repair: bool,
    allow_jobs: &[String],
    fixed: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, ACTION, WARN, _, _) = icons();
    if !repair || allow_jobs.is_empty() {
        return Ok(());
    }
    let allowset: std::collections::HashSet<String> =
        allow_jobs.iter().map(|s| s.to_string()).collect();
    let client = super::http_client()?;
    match super::http_client()?
        .get(format!("{base_url}/api/cron/jobs"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let payload: serde_json::Value = resp.json().await.unwrap_or_default();
            let jobs = payload
                .get("jobs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut recovered: Vec<String> = vec![];
            for job in jobs {
                let name = job.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let id = job.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let paused = job.get("last_status").and_then(|v| v.as_str()).unwrap_or("")
                    == "paused_unknown_action";
                if paused
                    && allowset.contains(name)
                    && !id.is_empty()
                    && let Ok(r) = client
                        .put(format!("{base_url}/api/cron/jobs/{id}"))
                        .json(&serde_json::json!({ "enabled": true }))
                        .send()
                        .await
                    && r.status().is_success()
                {
                    recovered.push(name.to_string());
                }
            }
            if !recovered.is_empty() {
                println!("  {ACTION} Re-enabled allowlisted paused jobs: {}", recovered.join(", "));
                *fixed += recovered.len() as u32;
            }
        }
        Ok(resp) => println!(
            "  {WARN} Could not inspect cron jobs for allowlisted recovery (HTTP {})",
            resp.status()
        ),
        Err(e) => println!("  {WARN} Cron allowlist recovery check failed: {e}"),
    }
    Ok(())
}
