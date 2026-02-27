use super::*;
use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn prompt_yes_no(question: &str) -> bool {
    // In non-interactive contexts, default to "no" to avoid surprise installs.
    if std::env::var("IRONCLAD_YES")
        .ok()
        .as_deref()
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return true;
    }

    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn path_contains_dir_in(dir: &Path, path_var: &std::ffi::OsStr) -> bool {
    std::env::split_paths(path_var).any(|p| {
        #[cfg(windows)]
        {
            p.to_string_lossy().to_ascii_lowercase() == dir.to_string_lossy().to_ascii_lowercase()
        }
        #[cfg(not(windows))]
        {
            p == dir
        }
    })
}

fn path_contains_dir(dir: &Path) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    path_contains_dir_in(dir, &path_var)
}

fn go_bin_candidates_with(gopath: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(gopath) = gopath {
        out.push(PathBuf::from(gopath).join("bin"));
    }

    #[cfg(windows)]
    if let Ok(profile) = std::env::var("USERPROFILE") {
        out.push(PathBuf::from(profile).join("go").join("bin"));
    }

    #[cfg(not(windows))]
    if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home).join("go").join("bin"));
    }

    out
}

fn go_bin_candidates() -> Vec<PathBuf> {
    go_bin_candidates_with(std::env::var("GOPATH").ok().as_deref())
}

fn find_gosh_in_go_bins_with(gopath: Option<&str>) -> Option<PathBuf> {
    #[cfg(windows)]
    let gosh_name = "gosh.exe";
    #[cfg(not(windows))]
    let gosh_name = "gosh";

    go_bin_candidates_with(gopath)
        .into_iter()
        .map(|d| d.join(gosh_name))
        .find(|p| p.is_file())
}

fn find_gosh_in_go_bins() -> Option<PathBuf> {
    find_gosh_in_go_bins_with(std::env::var("GOPATH").ok().as_deref())
}

fn recent_log_snapshot(log_dir: &Path, max_bytes: usize) -> Option<String> {
    let entries = std::fs::read_dir(log_dir).ok()?;
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !(name.starts_with("ironclad.log") || name == "ironclad.stderr.log") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    candidates.sort_by_key(|(modified, _)| *modified);
    let (_, newest) = candidates.into_iter().last()?;
    let data = std::fs::read(newest).ok()?;
    let start = data.len().saturating_sub(max_bytes);
    Some(String::from_utf8_lossy(&data[start..]).to_string())
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

#[derive(Debug, Clone, Serialize)]
struct MechanicRepairPlan {
    description: String,
    commands: Vec<String>,
    safe_auto_repair: bool,
    requires_human_approval: bool,
}

#[derive(Debug, Clone, Serialize)]
struct MechanicFinding {
    id: String,
    severity: String,
    confidence: f64,
    summary: String,
    details: String,
    repair_plan: MechanicRepairPlan,
    auto_repaired: bool,
}

#[derive(Debug, Default, Clone, Serialize)]
struct RepairActionSummary {
    directories_created: Vec<String>,
    config_created: bool,
    permissions_hardened: Vec<String>,
    schema_normalized: bool,
    paused_jobs_reenabled: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MechanicJsonReport {
    ok: bool,
    repair_mode: bool,
    findings: Vec<MechanicFinding>,
    actions: RepairActionSummary,
}

#[allow(clippy::too_many_arguments)]
fn finding(
    id: &str,
    severity: &str,
    confidence: f64,
    summary: impl Into<String>,
    details: impl Into<String>,
    plan_desc: impl Into<String>,
    commands: Vec<String>,
    safe_auto_repair: bool,
    requires_human_approval: bool,
) -> MechanicFinding {
    MechanicFinding {
        id: id.to_string(),
        severity: severity.to_string(),
        confidence,
        summary: summary.into(),
        details: details.into(),
        repair_plan: MechanicRepairPlan {
            description: plan_desc.into(),
            commands,
            safe_auto_repair,
            requires_human_approval,
        },
        auto_repaired: false,
    }
}

fn normalize_schema_safe(state_db_path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    if !state_db_path.exists() {
        return Ok(false);
    }
    let conn = rusqlite::Connection::open(state_db_path)?;
    conn.execute_batch(
        "BEGIN;
         UPDATE sub_agents SET role='subagent' WHERE lower(trim(role))='specialist';
         UPDATE sub_agents SET skills_json='[]' WHERE skills_json IS NULL;
         COMMIT;",
    )?;
    Ok(true)
}

async fn cmd_mechanic_json(
    base_url: &str,
    repair: bool,
    allow_jobs: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");
    let mut findings: Vec<MechanicFinding> = vec![];
    let mut actions = RepairActionSummary::default();

    let dirs = [
        ironclad_dir.clone(),
        ironclad_dir.join("workspace"),
        ironclad_dir.join("skills"),
        ironclad_dir.join("plugins"),
        ironclad_dir.join("logs"),
    ];
    for dir in &dirs {
        if !dir.exists() {
            let mut f = finding(
                "missing-directory",
                "medium",
                0.99,
                format!("Missing directory: {}", dir.display()),
                "Required runtime directory is absent.",
                "Create required Ironclad directory tree.",
                vec![format!("mkdir -p \"{}\"", dir.display())],
                true,
                false,
            );
            if repair {
                std::fs::create_dir_all(dir)?;
                f.auto_repaired = true;
                actions.directories_created.push(dir.display().to_string());
            }
            findings.push(f);
        }
    }

    let config_path = std::path::Path::new("ironclad.toml");
    let alt_config = ironclad_dir.join("ironclad.toml");
    if !config_path.exists() && !alt_config.exists() {
        let mut f = finding(
            "missing-config",
            "high",
            0.98,
            "No Ironclad config file found",
            "Neither local ./ironclad.toml nor ~/.ironclad/ironclad.toml exists.",
            "Initialize or restore runtime configuration.",
            vec!["ironclad init".to_string()],
            true,
            false,
        );
        if repair {
            let default_config = format!(
                concat!(
                    "[agent]\n",
                    "name = \"Ironclad\"\n",
                    "id = \"ironclad-dev\"\n\n",
                    "[server]\n",
                    "port = 18789\n",
                    "bind = \"127.0.0.1\"\n\n",
                    "[database]\n",
                    "path = \"{}/state.db\"\n\n",
                    "[models]\n",
                    "primary = \"ollama/qwen3:8b\"\n",
                    "fallbacks = [\"openai/gpt-4o\"]\n",
                ),
                ironclad_dir.display()
            );
            std::fs::create_dir_all(&ironclad_dir)?;
            std::fs::write(&alt_config, default_config)?;
            f.auto_repaired = true;
            actions.config_created = true;
        }
        findings.push(f);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [
            ironclad_dir.join("wallet.json"),
            ironclad_dir.join("state.db"),
        ] {
            if file.exists() {
                let meta = std::fs::metadata(&file)?;
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    let mut f = finding(
                        "loose-permissions",
                        "high",
                        0.97,
                        format!("Loose permissions on {}", file.display()),
                        format!("Current mode {:o} allows group/other access.", mode),
                        "Harden file permissions to owner-only (0600).",
                        vec![format!("chmod 600 \"{}\"", file.display())],
                        true,
                        false,
                    );
                    if repair {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o600);
                        std::fs::set_permissions(&file, perms)?;
                        f.auto_repaired = true;
                        actions
                            .permissions_hardened
                            .push(file.display().to_string());
                    }
                    findings.push(f);
                }
            }
        }
    }

    let gateway = reqwest::get(format!("{base_url}/api/health")).await;
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
        let diag_resp = reqwest::get(format!("{base_url}/api/agent/status")).await?;
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

        let channels_resp = reqwest::get(format!("{base_url}/api/channels/status")).await?;
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

        if repair && !allow_jobs.is_empty() {
            let jobs_resp = reqwest::get(format!("{base_url}/api/cron/jobs")).await?;
            if jobs_resp.status().is_success() {
                let payload: serde_json::Value = jobs_resp.json().await.unwrap_or_default();
                let jobs = payload
                    .get("jobs")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let allowset: std::collections::HashSet<String> =
                    allow_jobs.iter().map(|s| s.to_string()).collect();
                let client = reqwest::Client::new();
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

    let log_snapshot = recent_log_snapshot(&ironclad_dir.join("logs"), 350_000);
    if let Some(snapshot) = log_snapshot.as_deref() {
        let tg_404_count =
            count_occurrences(snapshot, "Telegram API error\",\"status\":\"404 Not Found");
        let tg_poll_err_count = count_occurrences(snapshot, "Telegram poll error, backing off 5s");
        if tg_404_count >= 3 || tg_poll_err_count >= 3 {
            findings.push(finding(
                "telegram-invalid-token-likely",
                "high",
                0.96,
                "Repeated Telegram 404/poll-backoff failures",
                "Log signatures strongly suggest an invalid or revoked Telegram bot token.",
                "Set a valid token and restart daemon.",
                vec![
                    "ironclad keystore set telegram_bot_token \"<TOKEN>\"".to_string(),
                    "ironclad daemon restart".to_string(),
                ],
                false,
                true,
            ));
        }
        let unknown_action_count = count_occurrences(snapshot, "unknown action: unknown");
        if unknown_action_count >= 3 {
            findings.push(finding(
                "cron-unknown-action-storm",
                "high",
                0.92,
                "Recurring cron unknown-action failures",
                "Scheduler repeatedly hit legacy/invalid cron action payloads.",
                "Recover paused jobs selectively after validation.",
                vec!["ironclad schedule recover --all --dry-run".to_string()],
                true,
                false,
            ));
        }
    }

    if repair {
        let state_db = ironclad_dir.join("state.db");
        if normalize_schema_safe(&state_db)? {
            actions.schema_normalized = true;
        }
    }

    let report = MechanicJsonReport {
        ok: findings
            .iter()
            .all(|f| f.severity == "info" || f.auto_repaired),
        repair_mode: repair,
        findings,
        actions,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(windows)]
fn add_dir_to_user_path_windows(dir: &Path) -> Result<(), String> {
    let dir_str = dir.display().to_string().replace('\'', "''");
    let script = format!(
        "$dir='{dir_str}'; \
         $current=[Environment]::GetEnvironmentVariable('Path','User'); \
         if ([string]::IsNullOrWhiteSpace($current)) {{ \
             [Environment]::SetEnvironmentVariable('Path',$dir,'User'); exit 0 \
         }}; \
         $parts=$current -split ';' | Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }}; \
         $exists=$false; \
         foreach ($p in $parts) {{ if ($p.Trim().ToLowerInvariant() -eq $dir.Trim().ToLowerInvariant()) {{ $exists=$true; break }} }}; \
         if (-not $exists) {{ [Environment]::SetEnvironmentVariable('Path', ($current.TrimEnd(';') + ';' + $dir), 'User') }}"
    );

    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map_err(|e| format!("failed to launch PowerShell: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("PowerShell failed to update user PATH".to_string())
    }
}

// ── Circuit breaker ───────────────────────────────────────────

pub async fn cmd_circuit_status(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/breaker/status").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

    heading("Circuit Breaker Status");

    if let Some(providers) = data["providers"].as_object() {
        if providers.is_empty() {
            empty_state("No providers registered yet");
        } else {
            for (name, status) in providers {
                let state = status["state"].as_str().unwrap_or("unknown");
                kv_accent(name, &status_badge(state));
            }
        }
    } else {
        empty_state("No providers registered yet");
    }

    if let Some(note) = data["note"].as_str() {
        eprintln!();
        eprintln!("    {DIM}\u{2139}  {note}{RESET}");
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_circuit_reset(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let client = reqwest::Client::new();
    let status = client
        .get(format!("{url}/api/breaker/status"))
        .send()
        .await
        .inspect_err(|_| {
            eprintln!("  {ERR} Cannot reach gateway at {url}");
        })?;

    heading("Circuit Breaker Reset");

    if !status.status().is_success() {
        eprintln!("    {WARN} Status returned HTTP {}", status.status());
        eprintln!();
        return Ok(());
    }

    let body: serde_json::Value = status.json().await.unwrap_or_default();
    let providers: Vec<String> = body
        .get("providers")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    if providers.is_empty() {
        eprintln!("    {WARN} No providers reported by gateway");
        eprintln!();
        return Ok(());
    }

    let mut reset_ok = 0usize;
    for provider in &providers {
        let resp = client
            .post(format!("{url}/api/breaker/reset/{provider}"))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                reset_ok += 1;
            }
            Ok(r) => {
                eprintln!("    {WARN} reset {} returned HTTP {}", provider, r.status());
            }
            Err(e) => {
                eprintln!("    {WARN} reset {} failed: {}", provider, e);
            }
        }
    }

    if reset_ok == providers.len() {
        eprintln!(
            "    {OK} Reset {} providers to closed state",
            providers.len()
        );
    } else {
        eprintln!(
            "    {WARN} Reset {}/{} providers",
            reset_ok,
            providers.len()
        );
    }

    eprintln!();
    Ok(())
}

// ── Agents, channels ──────────────────────────────────────────

pub async fn cmd_agents_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get(format!("{base_url}/api/agents")).await?;
    let body: serde_json::Value = resp.json().await?;

    let agents = body
        .get("agents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if agents.is_empty() {
        println!("\n  No agents registered.\n");
        return Ok(());
    }

    println!(
        "\n  {:<15} {:<20} {:<10} {:<15}",
        "ID", "Name", "State", "Model"
    );
    println!("  {}", "─".repeat(65));
    for a in &agents {
        let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let state = a.get("state").and_then(|v| v.as_str()).unwrap_or("?");
        let model = a.get("model").and_then(|v| v.as_str()).unwrap_or("?");
        println!("  {:<15} {:<20} {:<10} {:<15}", id, name, state, model);
    }
    println!();
    Ok(())
}

pub async fn cmd_channels_status(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get(format!("{base_url}/api/channels/status")).await?;
    let channels: Vec<serde_json::Value> = resp.json().await?;

    if channels.is_empty() {
        println!("  No channels configured.");
        return Ok(());
    }

    println!(
        "\n  {:<15} {:<10} {:<10} {:<10}",
        "Channel", "Status", "Recv", "Sent"
    );
    println!("  {}", "─".repeat(50));
    for ch in &channels {
        let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let connected = ch
            .get("connected")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let status_str = if connected { "✓ up" } else { "✗ down" };
        let recv = ch
            .get("messages_received")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let sent = ch
            .get("messages_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        println!(
            "  {:<15} {:<10} {:<10} {:<10}",
            name, status_str, recv, sent
        );
    }
    println!();
    Ok(())
}

pub async fn cmd_channels_dead_letter(
    base_url: &str,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get(format!("{base_url}/api/channels/dead-letter?limit={limit}")).await?;
    let body: serde_json::Value = resp.json().await?;
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if items.is_empty() {
        println!("  No dead-letter deliveries.");
        return Ok(());
    }

    println!(
        "\n  {:<38} {:<12} {:<10} {:<40}",
        "ID", "Channel", "Attempts", "Last error"
    );
    println!("  {}", "─".repeat(108));
    for item in items {
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("?");
        let attempts = item
            .get("attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or_default();
        let max_attempts = item
            .get("max_attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or_default();
        let last_error = item
            .get("last_error")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        println!(
            "  {:<38} {:<12} {:<10} {:<40}",
            truncate_id(id, 35),
            channel,
            format!("{attempts}/{max_attempts}"),
            truncate_id(last_error, 37),
        );
    }
    println!();
    Ok(())
}

pub async fn cmd_channels_replay(
    base_url: &str,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/api/channels/dead-letter/{id}/replay"))
        .send()
        .await?;
    if resp.status().is_success() {
        println!("  Replayed dead-letter item: {id}");
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("  Dead-letter item not found: {id}");
    } else {
        println!("  Replay failed for {id}: HTTP {}", resp.status());
    }
    Ok(())
}

// ── Mechanic ──────────────────────────────────────────────────

pub async fn cmd_mechanic(
    base_url: &str,
    repair: bool,
    json_output: bool,
    allow_jobs: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if json_output {
        return cmd_mechanic_json(base_url, repair, allow_jobs).await;
    }
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!(
        "\n  {BOLD}Ironclad Mechanic{RESET}{}\n",
        if repair { " (--repair mode)" } else { "" }
    );

    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");
    let mut fixed = 0u32;

    // Check directories
    let dirs = [
        ironclad_dir.clone(),
        ironclad_dir.join("workspace"),
        ironclad_dir.join("skills"),
        ironclad_dir.join("plugins"),
        ironclad_dir.join("logs"),
    ];

    for dir in &dirs {
        if dir.exists() {
            println!("  {OK} Directory exists: {}", dir.display());
        } else if repair {
            std::fs::create_dir_all(dir)?;
            println!("  {ACTION} Created directory: {}", dir.display());
            fixed += 1;
        } else {
            println!(
                "  {WARN} Missing directory: {} (use --repair to create)",
                dir.display()
            );
        }
    }

    // Check config file
    let config_path = std::path::Path::new("ironclad.toml");
    let alt_config = ironclad_dir.join("ironclad.toml");
    if config_path.exists() || alt_config.exists() {
        println!("  {OK} Configuration file found");
    } else if repair {
        let default_config = format!(
            concat!(
                "[agent]\n",
                "name = \"Ironclad\"\n",
                "id = \"ironclad-dev\"\n\n",
                "[server]\n",
                "port = 18789\n",
                "bind = \"127.0.0.1\"\n\n",
                "[database]\n",
                "path = \"{}/state.db\"\n\n",
                "[models]\n",
                "primary = \"ollama/qwen3:8b\"\n",
                "fallbacks = [\"openai/gpt-4o\"]\n\n",
                "# Provider-specific settings are auto-merged from bundled defaults.\n",
                "# Override any provider below; new providers work the same way.\n",
                "# [providers.ollama]\n",
                "# url = \"http://localhost:11434\"\n",
                "# tier = \"T1\"\n",
                "# format = \"openai\"\n",
                "# is_local = true\n",
            ),
            ironclad_dir.display()
        );
        std::fs::write(&alt_config, default_config)?;
        println!(
            "  {ACTION} Created default config: {}",
            alt_config.display()
        );
        fixed += 1;
    } else {
        println!("  {WARN} No config file found (use --repair or `ironclad init`)");
    }

    // Check file permissions (Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let sensitive_files = [
            ironclad_dir.join("wallet.json"),
            ironclad_dir.join("state.db"),
        ];

        for file in &sensitive_files {
            if file.exists() {
                let meta = std::fs::metadata(file)?;
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    if repair {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o600);
                        std::fs::set_permissions(file, perms)?;
                        println!("  {ACTION} Set permissions 600 on {}", file.display());
                        fixed += 1;
                    } else {
                        println!(
                            "  {WARN} {} has loose permissions ({:o}) - use --repair",
                            file.display(),
                            mode
                        );
                    }
                } else {
                    println!("  {OK} {} permissions OK ({:o})", file.display(), mode);
                }
            }
        }
    }

    // Check Go toolchain
    let mut go_bin = which_binary("go");
    match go_bin.as_ref() {
        Some(path) => {
            let ver = std::process::Command::new(path)
                .arg("version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let ver = ver.trim().strip_prefix("go version ").unwrap_or(ver.trim());
            println!("  {OK} Go toolchain: {ver} ({path})");
        }
        None => {
            println!("  {RED}{ERR}{RESET} Go not found (required for gosh plugin engine)");
            #[cfg(target_os = "windows")]
            println!(
                "         Install from https://go.dev/dl/ or: winget install -e --id GoLang.Go"
            );
            #[cfg(target_os = "macos")]
            println!("         Install from https://go.dev/dl/ or: brew install go");
            #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
            println!("         Install from https://go.dev/dl/ (or your distro package manager)");

            if repair && prompt_yes_no("  Attempt automatic Go installation now?") {
                #[cfg(target_os = "windows")]
                let install_result = std::process::Command::new("winget")
                    .args([
                        "install",
                        "-e",
                        "--id",
                        "GoLang.Go",
                        "--accept-package-agreements",
                        "--accept-source-agreements",
                    ])
                    .status();

                #[cfg(target_os = "macos")]
                let install_result = std::process::Command::new("brew")
                    .args(["install", "go"])
                    .status();

                #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
                let install_result = {
                    if which_binary("apt-get").is_some() {
                        std::process::Command::new("sudo")
                            .args(["apt-get", "install", "-y", "golang-go"])
                            .status()
                    } else if which_binary("dnf").is_some() {
                        std::process::Command::new("sudo")
                            .args(["dnf", "install", "-y", "golang"])
                            .status()
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "no supported package manager found",
                        ))
                    }
                };

                match install_result {
                    Ok(status) if status.success() => {
                        go_bin = which_binary("go");
                        if let Some(path) = go_bin.as_ref() {
                            println!("  {ACTION} Go installed: {path}");
                            fixed += 1;
                        } else {
                            println!(
                                "  {WARN} Go install may have succeeded, but `go` is not on PATH yet. Open a new shell and re-run `ironclad mechanic --repair`."
                            );
                        }
                    }
                    _ => {
                        println!("  {RED}{ERR}{RESET} Automatic Go install failed.");
                    }
                }
            }
        }
    }

    // Report Go bin PATH status explicitly so users can see why gosh may not resolve.
    let go_bin_dirs = go_bin_candidates();
    if go_bin_dirs.is_empty() {
        println!("  {WARN} Go bin path status: no candidate bin directory found");
    } else {
        for dir in &go_bin_dirs {
            if dir.exists() {
                if path_contains_dir(dir) {
                    println!("  {OK} Go bin path status: on PATH ({})", dir.display());
                } else {
                    println!(
                        "  {WARN} Go bin path status: missing from PATH ({})",
                        dir.display()
                    );
                }
            } else {
                println!(
                    "  {WARN} Go bin path status: candidate directory not found ({})",
                    dir.display()
                );
            }
        }
    }

    // Check gosh scripting engine
    match which_binary("gosh") {
        Some(path) => {
            println!("  {OK} gosh scripting engine: {path}");
        }
        None if repair => {
            if go_bin.is_some() {
                if prompt_yes_no("  Install gosh now via `go install`?") {
                    println!("  {ACTION} Installing gosh...");
                    let result = if let Some(go_path) = go_bin.as_deref() {
                        std::process::Command::new(go_path)
                            .args(["install", "github.com/drewwalton19216801/gosh@latest"])
                            .status()
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "go binary not found",
                        ))
                    };
                    match result {
                        Ok(s) if s.success() => {
                            if let Some(path) = which_binary("gosh") {
                                println!("  {ACTION} gosh installed: {path}");
                                fixed += 1;
                            } else if let Some(gosh_path) = find_gosh_in_go_bins() {
                                println!(
                                    "  {WARN} gosh installed at {} but not on PATH.",
                                    gosh_path.display()
                                );
                                if let Some(go_bin_dir) = gosh_path.parent() {
                                    if prompt_yes_no(&format!(
                                        "  Add {} to your PATH now?",
                                        go_bin_dir.display()
                                    )) {
                                        #[cfg(windows)]
                                        {
                                            match add_dir_to_user_path_windows(go_bin_dir) {
                                                Ok(()) => {
                                                    println!(
                                                        "  {ACTION} Added {} to user PATH",
                                                        go_bin_dir.display()
                                                    );
                                                    println!(
                                                        "         Open a new shell and re-run `ironclad mechanic --repair` to verify."
                                                    );
                                                    fixed += 1;
                                                }
                                                Err(e) => {
                                                    println!(
                                                        "  {RED}{ERR}{RESET} Failed to update PATH automatically: {e}"
                                                    );
                                                    println!(
                                                        "         Add this directory manually: {}",
                                                        go_bin_dir.display()
                                                    );
                                                }
                                            }
                                        }
                                        #[cfg(not(windows))]
                                        {
                                            println!(
                                                "  {WARN} Automatic PATH updates are only implemented on Windows."
                                            );
                                            println!(
                                                "         Add this directory manually: {}",
                                                go_bin_dir.display()
                                            );
                                        }
                                    } else {
                                        println!("  {WARN} PATH update skipped by user.");
                                    }
                                }
                            } else {
                                println!(
                                    "  {WARN} go install succeeded but `gosh` is not on PATH."
                                );
                                println!(
                                    "         Add your Go bin directory to PATH, then re-run mechanic."
                                );
                                #[cfg(target_os = "windows")]
                                println!("         Typical path: %USERPROFILE%\\go\\bin");
                                #[cfg(not(target_os = "windows"))]
                                println!("         Typical path: $HOME/go/bin");
                            }
                        }
                        _ => {
                            println!("  {RED}{ERR}{RESET} Failed to install gosh. Try manually:");
                            println!(
                                "         go install github.com/drewwalton19216801/gosh@latest"
                            );
                        }
                    }
                } else {
                    println!("  {WARN} gosh not installed (skipped by user)");
                }
            } else {
                println!("  {WARN} gosh not found (Go is required first)");
            }
        }
        None => {
            println!(
                "  {WARN} gosh not found (use --repair to install, or: go install github.com/drewwalton19216801/gosh@latest)"
            );
            if let Some(gosh_path) = find_gosh_in_go_bins() {
                println!(
                    "         Found gosh at {} but that directory is not on PATH.",
                    gosh_path.display()
                );
                if let Some(dir) = gosh_path.parent()
                    && !path_contains_dir(dir)
                {
                    #[cfg(target_os = "windows")]
                    println!("         Run `ironclad mechanic --repair` to add it with approval.");
                }
            }
        }
    }

    // Check gateway reachability first -- all subsequent server checks depend on this
    let gateway_up = match reqwest::get(format!("{base_url}/api/health")).await {
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

        // Config
        match reqwest::get(format!("{base_url}/api/config")).await {
            Ok(resp) if resp.status().is_success() => {
                println!("  {OK} Configuration loaded on server");
            }
            Ok(resp) => {
                println!("  {WARN} Config endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  {WARN} Config check failed: {e}");
            }
        }

        // Runtime diagnostics (delegation integrity signals)
        match reqwest::get(format!("{base_url}/api/agent/status")).await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                runtime_diag = body.get("diagnostics").cloned();
                println!("  {OK} Runtime diagnostics available");
            }
            Ok(resp) => {
                println!(
                    "  {WARN} Agent status endpoint returned HTTP {}",
                    resp.status()
                );
            }
            Err(e) => {
                println!("  {WARN} Agent status check failed: {e}");
            }
        }

        // Skills
        match reqwest::get(format!("{base_url}/api/skills")).await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let count = body
                    .get("skills")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!("  {OK} Skills loaded ({count} skills)");
            }
            Ok(resp) => {
                println!("  {WARN} Skills endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  {WARN} Skills check failed: {e}");
            }
        }

        // Wallet
        match reqwest::get(format!("{base_url}/api/wallet/balance")).await {
            Ok(resp) if resp.status().is_success() => {
                println!("  {OK} Wallet accessible");
            }
            Ok(resp) => {
                println!("  {WARN} Wallet endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  {WARN} Wallet check failed: {e}");
            }
        }

        // Channels
        match reqwest::get(format!("{base_url}/api/channels/status")).await {
            Ok(resp) if resp.status().is_success() => {
                let body: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
                let active = body
                    .iter()
                    .filter(|c| {
                        c.get("connected")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    })
                    .count();
                println!("  {OK} Channels ({active}/{} connected)", body.len());
                channels_status = Some(body);
            }
            Ok(resp) => {
                println!("  {WARN} Channels endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  {WARN} Channels check failed: {e}");
            }
        }

        // Smart diagnostics from recent logs + channel telemetry.
        let log_snapshot = recent_log_snapshot(&ironclad_dir.join("logs"), 350_000);
        if let Some(snapshot) = log_snapshot.as_deref() {
            let tg_404_count =
                count_occurrences(snapshot, "Telegram API error\",\"status\":\"404 Not Found");
            let tg_poll_err_count =
                count_occurrences(snapshot, "Telegram poll error, backing off 5s");
            if tg_404_count >= 3 || tg_poll_err_count >= 3 {
                println!(
                    "  {WARN} Detected repeated Telegram transport failures (404/poll backoff loop)."
                );
                println!("         Likely cause: invalid/revoked Telegram bot token in keystore.");
                println!(
                    "         Repair: `ironclad keystore set telegram_bot_token \"<TOKEN>\"` then `ironclad daemon restart`"
                );
            }

            let unknown_action_count = count_occurrences(snapshot, "unknown action: unknown");
            if unknown_action_count >= 3 {
                println!(
                    "  {WARN} Detected recurring scheduler failures: `unknown action: unknown`."
                );
                println!(
                    "         Repair: run `ironclad schedule recover --all --dry-run` and re-enable trusted jobs."
                );
            }
        }

        if let Some(channels) = channels_status.as_ref() {
            let telegram = channels
                .iter()
                .find(|c| c.get("name").and_then(|v| v.as_str()) == Some("telegram"));
            if let Some(tg) = telegram {
                let connected = tg
                    .get("connected")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let received = tg
                    .get("messages_received")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let sent = tg
                    .get("messages_sent")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if connected && received == 0 && sent == 0 {
                    println!("  {WARN} Telegram appears connected but has zero traffic.");
                    println!(
                        "         If this is unexpected, run `ironclad channels status` and inspect logs for poll/webhook errors."
                    );
                }
            }
        }

        if let Some(diag) = runtime_diag.as_ref() {
            let total = diag
                .get("taskable_subagents_total")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let enabled = diag
                .get("taskable_subagents_enabled")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let running = diag
                .get("taskable_subagents_running")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let error = diag
                .get("taskable_subagents_error")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if total > 0 && enabled > 0 && running == 0 {
                println!(
                    "  {WARN} Delegation integrity risk: {enabled} taskable subagent(s) enabled, but 0 running."
                );
                println!(
                    "         Any response attributed to a subagent cannot be runtime-verified right now."
                );
                println!(
                    "         Repair: start/recover subagents and re-check with `ironclad status` / `ironclad mechanic`."
                );
            } else if enabled > running {
                println!(
                    "  {WARN} Delegation degradation: enabled subagents ({enabled}) exceed running ({running})."
                );
                if error > 0 {
                    println!("         {error} subagent(s) currently report error state.");
                }
                println!(
                    "         Recommendation: treat subagent-attributed outputs as unverified until running count recovers."
                );
            }
        }

        if repair && !allow_jobs.is_empty() {
            let allowset: std::collections::HashSet<String> =
                allow_jobs.iter().map(|s| s.to_string()).collect();
            let client = reqwest::Client::new();
            match reqwest::get(format!("{base_url}/api/cron/jobs")).await {
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
                        let paused = job
                            .get("last_status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
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
                        println!(
                            "  {ACTION} Re-enabled allowlisted paused jobs: {}",
                            recovered.join(", ")
                        );
                        fixed += recovered.len() as u32;
                    }
                }
                Ok(resp) => {
                    println!(
                        "  {WARN} Could not inspect cron jobs for allowlisted recovery (HTTP {})",
                        resp.status()
                    );
                }
                Err(e) => {
                    println!("  {WARN} Cron allowlist recovery check failed: {e}");
                }
            }
        }
    } else {
        println!("    {DETAIL} Skipping server checks (config, skills, wallet, channels)");
    }

    if repair {
        let state_db = ironclad_dir.join("state.db");
        match normalize_schema_safe(&state_db) {
            Ok(true) => {
                println!("  {ACTION} Applied safe schema normalization in state.db");
                fixed += 1;
            }
            Ok(false) => {}
            Err(e) => {
                println!("  {WARN} Schema normalization skipped: {e}");
            }
        }
    }

    println!();
    if repair && fixed > 0 {
        println!("  {ACTION} Auto-fixed {fixed} issue(s)");
    }
    println!();
    Ok(())
}

// ── Completion, uninstall, reset ───────────────────────────────

pub fn cmd_completion(shell: &str) -> Result<(), Box<dyn std::error::Error>> {
    match shell {
        "bash" => {
            println!("# Ironclad bash completion");
            println!("# Add to ~/.bashrc: eval \"$(ironclad completion bash)\"");
            println!(
                "complete -W \"serve init check version status sessions memory skills cron metrics wallet config breaker channels plugins mechanic daemon completion\" ironclad"
            );
        }
        "zsh" => {
            println!("# Ironclad zsh completion");
            println!("# Add to ~/.zshrc: eval \"$(ironclad completion zsh)\"");
            println!(
                "compctl -k \"(serve init check version status sessions memory skills cron metrics wallet config breaker channels plugins mechanic daemon completion)\" ironclad"
            );
        }
        "fish" => {
            println!("# Ironclad fish completion");
            println!("# Run: ironclad completion fish | source");
            for cmd in [
                "serve",
                "init",
                "check",
                "version",
                "status",
                "sessions",
                "memory",
                "skills",
                "cron",
                "metrics",
                "wallet",
                "config",
                "breaker",
                "channels",
                "plugins",
                "mechanic",
                "daemon",
                "completion",
            ] {
                println!("complete -c ironclad -a {cmd}");
            }
        }
        _ => {
            eprintln!("Unsupported shell: {shell}. Use bash, zsh, or fish.");
        }
    }
    Ok(())
}

pub fn cmd_uninstall(purge: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Ironclad Uninstall{RESET}\n");

    match crate::daemon::uninstall_daemon() {
        Ok(()) => println!("  {OK} Daemon service removed"),
        Err(e) => println!("  {WARN} Daemon removal: {e}"),
    }

    if purge {
        let data_dir = ironclad_core::home_dir().join(".ironclad");
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)?;
            println!("  {OK} Removed {}", data_dir.display());
        } else {
            println!("  {WARN} Data directory not found: {}", data_dir.display());
        }
    } else {
        println!("  {DIM}Data preserved at ~/.ironclad/ (use --purge to remove){RESET}");
    }

    println!("\n  {GREEN}Uninstall complete.{RESET} CLI binary remains at current location.\n");
    Ok(())
}

pub fn cmd_reset(yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Ironclad Reset{RESET}\n");

    if !yes {
        println!("  This will reset configuration and clear the database.");
        println!("  Wallet files will be preserved.");
        println!("  Run with --yes to skip this prompt.\n");
        print!("  Continue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  Aborted.");
            return Ok(());
        }
    }

    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");

    let db_path = ironclad_dir.join("state.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
        println!("  {OK} Database cleared");
    }

    let db_wal = ironclad_dir.join("state.db-wal");
    if db_wal.exists() {
        let _ = std::fs::remove_file(&db_wal);
    }
    let db_shm = ironclad_dir.join("state.db-shm");
    if db_shm.exists() {
        let _ = std::fs::remove_file(&db_shm);
    }

    let config_path = ironclad_dir.join("ironclad.toml");
    if config_path.exists() {
        std::fs::remove_file(&config_path)?;
        println!("  {OK} Configuration removed (re-run `ironclad init` to recreate)");
    }

    let logs_dir = ironclad_dir.join("logs");
    if logs_dir.exists() {
        std::fs::remove_dir_all(&logs_dir)?;
        println!("  {OK} Logs cleared");
    }

    let wallet_dir = ironclad_dir.join("wallet.json");
    if wallet_dir.exists() {
        println!("  {WARN} Wallet preserved: {}", wallet_dir.display());
    }

    println!("\n  {GREEN}Reset complete.{RESET}\n");
    Ok(())
}

// ── Metrics ───────────────────────────────────────────────────

pub async fn cmd_metrics(
    url: &str,
    kind: &str,
    hours: Option<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;

    match kind {
        "costs" => {
            let data = c.get("/api/stats/costs").await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading("Inference Costs");
            let costs = data["costs"].as_array();
            match costs {
                Some(arr) if !arr.is_empty() => {
                    let widths = [20, 16, 10, 10, 10, 8];
                    table_header(
                        &[
                            "Model",
                            "Provider",
                            "Tokens In",
                            "Tokens Out",
                            "Cost",
                            "Cached",
                        ],
                        &widths,
                    );

                    let mut total_cost = 0.0f64;
                    let mut total_in = 0i64;
                    let mut total_out = 0i64;

                    for c in arr {
                        let model = truncate_id(c["model"].as_str().unwrap_or(""), 17);
                        let provider = c["provider"].as_str().unwrap_or("").to_string();
                        let tin = c["tokens_in"].as_i64().unwrap_or(0);
                        let tout = c["tokens_out"].as_i64().unwrap_or(0);
                        let cost = c["cost"].as_f64().unwrap_or(0.0);
                        let cached = c["cached"].as_bool().unwrap_or(false);

                        total_cost += cost;
                        total_in += tin;
                        total_out += tout;

                        table_row(
                            &[
                                format!("{ACCENT}{model}{RESET}"),
                                provider,
                                tin.to_string(),
                                tout.to_string(),
                                format!("${cost:.4}"),
                                if cached {
                                    OK.to_string()
                                } else {
                                    format!("{DIM}-{RESET}")
                                },
                            ],
                            &widths,
                        );
                    }
                    table_separator(&widths);
                    eprintln!();
                    kv_accent("Total Cost", &format!("${total_cost:.4}"));
                    kv("Total Tokens", &format!("{total_in} in / {total_out} out"));
                    kv("Requests", &arr.len().to_string());
                    if !arr.is_empty() {
                        kv(
                            "Avg Cost/Request",
                            &format!("${:.4}", total_cost / arr.len() as f64),
                        );
                    }
                }
                _ => empty_state("No inference costs recorded"),
            }
        }
        "transactions" => {
            let h = hours.unwrap_or(24);
            let data = c
                .get(&format!("/api/stats/transactions?hours={h}"))
                .await
                .map_err(|e| {
                    IroncladClient::check_connectivity_hint(&*e);
                    e
                })?;
            heading(&format!("Transactions (last {h}h)"));
            let txs = data["transactions"].as_array();
            match txs {
                Some(arr) if !arr.is_empty() => {
                    let widths = [14, 12, 12, 20, 22];
                    table_header(&["ID", "Type", "Amount", "Counterparty", "Time"], &widths);

                    let mut total = 0.0f64;
                    for t in arr {
                        let id = truncate_id(t["id"].as_str().unwrap_or(""), 11);
                        let tx_type = t["tx_type"].as_str().unwrap_or("").to_string();
                        let amount = t["amount"].as_f64().unwrap_or(0.0);
                        let currency = t["currency"].as_str().unwrap_or("USD");
                        let counter = t["counterparty"].as_str().unwrap_or("-").to_string();
                        let time = t["created_at"]
                            .as_str()
                            .map(|t| if t.len() > 19 { &t[..19] } else { t })
                            .unwrap_or("")
                            .to_string();

                        total += amount;

                        table_row(
                            &[
                                format!("{MONO}{id}{RESET}"),
                                tx_type,
                                format!("{amount:.2} {currency}"),
                                counter,
                                format!("{DIM}{time}{RESET}"),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    kv_accent("Total", &format!("{total:.2}"));
                    kv("Count", &arr.len().to_string());
                }
                _ => empty_state("No transactions in this time window"),
            }
        }
        "cache" => {
            let data = c.get("/api/stats/cache").await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading("Cache Statistics");
            let hits = data["hits"].as_u64().unwrap_or(0);
            let misses = data["misses"].as_u64().unwrap_or(0);
            let entries = data["entries"].as_u64().unwrap_or(0);
            let hit_rate = data["hit_rate"].as_f64().unwrap_or(0.0);

            kv_accent("Entries", &entries.to_string());
            kv("Hits", &hits.to_string());
            kv("Misses", &misses.to_string());

            let bar_width = 30;
            let filled = (hit_rate * bar_width as f64 / 100.0) as usize;
            let empty_part = bar_width - filled;
            let bar = format!(
                "{GREEN}{}{DIM}{}{RESET} {:.1}%",
                "\u{2588}".repeat(filled),
                "\u{2591}".repeat(empty_part),
                hit_rate
            );
            kv("Hit Rate", &bar);
        }
        _ => {
            return Err(
                format!("unknown metric kind: {kind}. Use: costs, transactions, cache").into(),
            );
        }
    }

    eprintln!();
    Ok(())
}

// ── Logs ──────────────────────────────────────────────────────

fn try_read_log_file(lines: usize, _level: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let log_dir = ironclad_core::home_dir().join(".ironclad").join("logs");

    if !log_dir.exists() {
        println!("  No log directory found at {}", log_dir.display());
        return;
    }

    let mut entries: Vec<_> = match std::fs::read_dir(&log_dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            println!("  Error reading log directory: {e}");
            return;
        }
    };

    entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

    if let Some(latest) = entries.first() {
        let path = latest.path();
        println!("  {DIM}Reading: {}{RESET}\n", path.display());
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let all_lines: Vec<&str> = content.lines().collect();
                let start = if all_lines.len() > lines {
                    all_lines.len() - lines
                } else {
                    0
                };
                for line in &all_lines[start..] {
                    println!("{line}");
                }
            }
            Err(e) => println!("  Error reading log file: {e}"),
        }
    } else {
        println!("  No log files found.");
    }
}

pub async fn cmd_logs(
    base_url: &str,
    lines: usize,
    follow: bool,
    level: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    if follow {
        println!("  {BOLD}Tailing logs{RESET} (level >= {level}, Ctrl+C to stop)\n");

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{base_url}/api/logs"))
            .query(&[
                ("follow", "true"),
                ("level", level),
                ("lines", &lines.to_string()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            eprintln!("  Server returned {}", resp.status());
            eprintln!("  Log tailing requires a running server.");

            try_read_log_file(lines, level);
            return Ok(());
        }

        let mut stream = resp.bytes_stream();
        use tokio_stream::StreamExt;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            println!("{data}");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  Stream error: {e}");
                    break;
                }
            }
        }
    } else {
        let resp = reqwest::get(format!("{base_url}/api/logs?lines={lines}&level={level}")).await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().await.unwrap_or_default();
                if let Some(entries) = body.get("entries").and_then(|v| v.as_array()) {
                    if entries.is_empty() {
                        println!("  No log entries found.");
                    }
                    for entry in entries {
                        let ts = entry
                            .get("timestamp")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let lvl = entry
                            .get("level")
                            .and_then(|v| v.as_str())
                            .unwrap_or("info");
                        let msg = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("");
                        let color = match lvl {
                            "ERROR" | "error" => RED,
                            "WARN" | "warn" => YELLOW,
                            "INFO" | "info" => GREEN,
                            "DEBUG" | "debug" => CYAN,
                            _ => DIM,
                        };
                        println!("{color}{ts} [{lvl:>5}] {target}: {msg}{RESET}");
                    }
                } else {
                    println!("  No log entries returned.");
                }
            }
            Ok(r) => {
                eprintln!("  Server returned {}", r.status());
                try_read_log_file(lines, level);
            }
            Err(_) => {
                eprintln!("  Server not reachable. Reading log files directly...\n");
                try_read_log_file(lines, level);
            }
        }
    }
    Ok(())
}

// ── Security audit ─────────────────────────────────────────────

pub fn cmd_security_audit(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Ironclad Security Audit{RESET}\n");

    let mut pass_count = 0u32;
    let mut warn_count = 0u32;
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut fail_count = 0u32;

    // 1. Check config file permissions
    let config_file = std::path::Path::new(config_path);
    if config_file.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(config_file)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                println!(
                    "  {RED}{ERR} FAIL{RESET} Config file is world/group-readable (mode {:o})",
                    mode & 0o777
                );
                println!("         Fix: chmod 600 {config_path}");
                fail_count += 1;
            } else {
                println!("  {OK} Config file permissions (mode {:o})", mode & 0o777);
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  {WARN} Config file permission check (non-Unix)");
            warn_count += 1;
        }
    } else {
        println!("  {WARN} Config file not found: {config_path}");
        warn_count += 1;
    }

    // 2. Check for API keys in config
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        let has_plaintext_key =
            content.contains("api_key") && !content.contains("${") && !content.contains("env(");
        if has_plaintext_key {
            println!("  {WARN} Plaintext API keys found in config");
            println!("         Recommendation: Use environment variables instead");
            warn_count += 1;
        } else {
            println!("  {OK} No plaintext API keys in config");
            pass_count += 1;
        }
    }

    // 3. Check bind address
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("bind = \"0.0.0.0\"") {
            println!("  {WARN} Server bound to 0.0.0.0 (all interfaces)");
            println!("         Recommendation: Bind to 127.0.0.1 unless external access is needed");
            warn_count += 1;
        } else {
            println!("  {OK} Server not bound to all interfaces");
            pass_count += 1;
        }
    }

    // 4. Check wallet file permissions
    let wallet_path = ironclad_core::home_dir()
        .join(".ironclad")
        .join("wallet.json");
    if wallet_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&wallet_path)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                println!(
                    "  {RED}{ERR} FAIL{RESET} Wallet file is world/group-readable (mode {:o})",
                    mode & 0o777
                );
                println!("         Fix: chmod 600 {}", wallet_path.display());
                fail_count += 1;
            } else {
                println!("  {OK} Wallet file permissions (mode {:o})", mode & 0o777);
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  {WARN} Wallet permission check (non-Unix)");
            warn_count += 1;
        }
    } else {
        println!("  {DIM}  \u{2500}{RESET} No wallet file found (OK if not using wallet features)");
    }

    // 5. Check database file permissions
    let db_path = ironclad_core::home_dir().join(".ironclad").join("state.db");
    if db_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&db_path)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                println!(
                    "  {WARN} Database is world/group-readable (mode {:o})",
                    mode & 0o777
                );
                println!("         Fix: chmod 600 {}", db_path.display());
                warn_count += 1;
            } else {
                println!("  {OK} Database file permissions");
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  {WARN} Database permission check (non-Unix)");
            warn_count += 1;
        }
    }

    // 6. Check CORS settings
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("cors") && content.contains("\"*\"") {
            println!("  {WARN} CORS allows all origins (\"*\")");
            println!("         Recommendation: Restrict CORS to specific origins in production");
            warn_count += 1;
        } else {
            println!("  {OK} CORS configuration");
            pass_count += 1;
        }
    }

    // 7. Check PID file
    let pid_path = ironclad_core::home_dir()
        .join(".ironclad")
        .join("ironclad.pid");
    if pid_path.exists() {
        println!("  {OK} PID file exists");
        pass_count += 1;
    }

    // Summary
    println!();
    let total = pass_count + warn_count + fail_count;
    if fail_count > 0 {
        println!(
            "  {RED}{ERR}{RESET} {fail_count} failure(s), {warn_count} warning(s), {pass_count} passed out of {total} checks"
        );
    } else if warn_count > 0 {
        println!("  {WARN} {warn_count} warning(s), {pass_count} passed out of {total} checks");
    } else {
        println!("  {OK} All {total} checks passed");
    }
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    struct EnvGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: test-local environment mutation restored on Drop.
            unsafe { std::env::set_var(key, value) };
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.old {
                // SAFETY: restoring previous process env value.
                unsafe { std::env::set_var(self.key, v) };
            } else {
                // SAFETY: restoring previous process env value.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn path_contains_dir_and_go_bin_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path_var = std::ffi::OsString::from(dir.path().to_str().unwrap());
        assert!(path_contains_dir_in(dir.path(), &path_var));
        assert!(!path_contains_dir_in(
            Path::new("/definitely/not/here"),
            &path_var
        ));

        let gopath = tempfile::tempdir().unwrap();
        let bin_dir = gopath.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        #[cfg(windows)]
        let gosh = bin_dir.join("gosh.exe");
        #[cfg(not(windows))]
        let gosh = bin_dir.join("gosh");
        std::fs::write(&gosh, "stub").unwrap();
        assert_eq!(
            find_gosh_in_go_bins_with(gopath.path().to_str()),
            Some(gosh)
        );
    }

    #[test]
    fn recent_log_snapshot_and_count_occurrences_work() {
        let dir = tempfile::tempdir().unwrap();
        let older = dir.path().join("ironclad.log");
        let newer = dir.path().join("ironclad.stderr.log");
        std::fs::write(&older, "old line").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&newer, "abc abc abc").unwrap();

        let snap = recent_log_snapshot(dir.path(), 8).unwrap();
        assert!(snap.contains("abc"));
        assert_eq!(count_occurrences("abc abc abc", "abc"), 3);
    }

    #[test]
    fn finding_builder_sets_repair_metadata() {
        let f = finding(
            "id-1",
            "high",
            0.9,
            "summary",
            "details",
            "plan",
            vec!["cmd".into()],
            true,
            false,
        );
        assert_eq!(f.id, "id-1");
        assert_eq!(f.severity, "high");
        assert!(f.repair_plan.safe_auto_repair);
        assert!(!f.repair_plan.requires_human_approval);
        assert_eq!(f.repair_plan.commands, vec!["cmd"]);
    }

    #[test]
    fn normalize_schema_safe_updates_legacy_subagent_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sub_agents (role TEXT, skills_json TEXT);
             INSERT INTO sub_agents (role, skills_json) VALUES ('specialist', NULL);",
        )
        .unwrap();
        drop(conn);

        assert!(normalize_schema_safe(&db_path).unwrap());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let (role, skills): (String, String) = conn
            .query_row(
                "SELECT role, skills_json FROM sub_agents LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(role, "subagent");
        assert_eq!(skills, "[]");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn mechanic_json_repair_mode_creates_default_layout() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let ironclad_dir = home.path().join(".ironclad");
        let logs_dir = ironclad_dir.join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::write(
            logs_dir.join("ironclad.log"),
            "Telegram API error\",\"status\":\"404 Not Found\"\n\
             Telegram API error\",\"status\":\"404 Not Found\"\n\
             Telegram API error\",\"status\":\"404 Not Found\"\n\
             unknown action: unknown\nunknown action: unknown\nunknown action: unknown\n",
        )
        .unwrap();

        let state_db = ironclad_dir.join("state.db");
        let conn = rusqlite::Connection::open(&state_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sub_agents (role TEXT, skills_json TEXT);
             INSERT INTO sub_agents (role, skills_json) VALUES ('specialist', NULL);",
        )
        .unwrap();
        drop(conn);

        let wallet = ironclad_dir.join("wallet.json");
        std::fs::write(&wallet, "{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&wallet).unwrap().permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(&wallet, perms).unwrap();
        }

        cmd_mechanic_json("http://127.0.0.1:9", true, &[])
            .await
            .expect("mechanic should complete with unreachable gateway");

        let conn = rusqlite::Connection::open(&state_db).unwrap();
        let role: String = conn
            .query_row("SELECT role FROM sub_agents LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(role, "subagent");
    }

    #[test]
    fn cmd_reset_yes_removes_state_and_preserves_wallet() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let ironclad_dir = home.path().join(".ironclad");
        let logs_dir = ironclad_dir.join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::write(ironclad_dir.join("state.db"), "db").unwrap();
        std::fs::write(
            ironclad_dir.join("ironclad.toml"),
            "[agent]\nname='x'\nid='x'\n",
        )
        .unwrap();
        std::fs::write(ironclad_dir.join("wallet.json"), "{}").unwrap();

        cmd_reset(true).unwrap();

        assert!(!ironclad_dir.join("state.db").exists());
        assert!(!ironclad_dir.join("ironclad.toml").exists());
        assert!(!logs_dir.exists());
        assert!(ironclad_dir.join("wallet.json").exists());
    }

    #[test]
    fn cmd_uninstall_purge_removes_data_dir() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let ironclad_dir = home.path().join(".ironclad");
        std::fs::create_dir_all(&ironclad_dir).unwrap();
        std::fs::write(ironclad_dir.join("state.db"), "db").unwrap();

        cmd_uninstall(true).unwrap();
        assert!(!ironclad_dir.exists());
    }

    #[test]
    fn cmd_security_audit_runs_against_local_config() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let cfg_dir = tempfile::tempdir().unwrap();
        let cfg_path = cfg_dir.path().join("ironclad.toml");
        std::fs::write(
            &cfg_path,
            r#"[agent]
name = "Test"
id = "test"
[server]
bind = "127.0.0.1"
port = 18789
[database]
path = ":memory:"
[models]
primary = "ollama/qwen3:8b"
"#,
        )
        .unwrap();

        cmd_security_audit(cfg_path.to_str().unwrap()).unwrap();
    }
}
