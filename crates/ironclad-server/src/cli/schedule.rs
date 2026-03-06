use super::*;

pub async fn cmd_schedule_list(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/cron/jobs").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    heading("Cron Jobs");
    let jobs = data["jobs"].as_array();
    match jobs {
        Some(arr) if !arr.is_empty() => {
            let widths = [22, 24, 12, 22, 10, 8];
            table_header(
                &["Name", "Intent", "Schedule", "Last Run", "Status", "Errors"],
                &widths,
            );
            for j in arr {
                let name = j["name"].as_str().unwrap_or("").to_string();
                let intent = j["description"]
                    .as_str()
                    .map(|d| d.trim())
                    .filter(|d| !d.is_empty())
                    .unwrap_or("no description");
                let kind = j["schedule_kind"].as_str().unwrap_or("?");
                let expr = j["schedule_expr"].as_str().unwrap_or("");
                let schedule = format!("{kind}: {expr}");
                let last_run = j["last_run_at"]
                    .as_str()
                    .map(|t| if t.len() > 19 { &t[..19] } else { t })
                    .unwrap_or("never")
                    .to_string();
                let status = j["last_status"].as_str().unwrap_or("pending");
                let errors = j["consecutive_errors"].as_i64().unwrap_or(0);
                table_row(
                    &[
                        format!("{ACCENT}{name}{RESET}"),
                        format!("{DIM}{}{RESET}", truncate_id(intent, 24)),
                        truncate_id(&schedule, 12),
                        format!("{DIM}{last_run}{RESET}"),
                        status_badge(status),
                        if errors > 0 {
                            format!("{RED}{errors}{RESET}")
                        } else {
                            format!("{DIM}0{RESET}")
                        },
                    ],
                    &widths,
                );
            }
            eprintln!();
            eprintln!("    {DIM}{} job(s){RESET}", arr.len());
        }
        _ => empty_state("No cron jobs configured"),
    }
    eprintln!();
    Ok(())
}

pub async fn cmd_schedule_recover(
    url: &str,
    names: &[String],
    all: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();

    let c = IroncladClient::new(url)?;
    let data = c.get("/api/cron/jobs").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let jobs = data["jobs"].as_array().cloned().unwrap_or_default();

    let paused: Vec<Value> = jobs
        .into_iter()
        .filter(|j| j["last_status"].as_str() == Some("paused_unknown_action"))
        .collect();

    if paused.is_empty() {
        heading("Schedule Recovery");
        empty_state("No paused cron jobs found.");
        eprintln!();
        return Ok(());
    }

    let selected: Vec<Value> = if all {
        paused.clone()
    } else if !names.is_empty() {
        paused
            .iter()
            .filter(|j| {
                let job_name = j["name"].as_str().unwrap_or_default();
                names.iter().any(|n| n == job_name)
            })
            .cloned()
            .collect()
    } else {
        heading("Schedule Recovery");
        eprintln!("    {WARN} Found {} paused job(s).{RESET}", paused.len());
        eprintln!(
            "    {DIM}Use {BOLD}ironclad schedule recover --all{RESET}{DIM} to re-enable all, or {BOLD}--name <job>{RESET}{DIM} to select specific jobs.{RESET}"
        );
        eprintln!();
        let widths = [36, 10, 22];
        table_header(&["Name", "Enabled", "Last Run"], &widths);
        for j in &paused {
            let name = j["name"].as_str().unwrap_or("").to_string();
            let enabled = j["enabled"].as_bool().unwrap_or(false);
            let last_run = j["last_run_at"]
                .as_str()
                .map(|t| if t.len() > 19 { &t[..19] } else { t })
                .unwrap_or("never")
                .to_string();
            table_row(
                &[
                    format!("{ACCENT}{name}{RESET}"),
                    if enabled {
                        format!("{GREEN}true{RESET}")
                    } else {
                        format!("{YELLOW}false{RESET}")
                    },
                    format!("{DIM}{last_run}{RESET}"),
                ],
                &widths,
            );
        }
        eprintln!();
        return Ok(());
    };

    if selected.is_empty() {
        heading("Schedule Recovery");
        eprintln!("    {ERR} No paused jobs matched the provided name filter.{RESET}");
        eprintln!();
        return Ok(());
    }

    heading("Schedule Recovery");
    eprintln!(
        "    {ACTION} {} job(s) selected for re-enable{}",
        selected.len(),
        if dry_run { " (dry-run)" } else { "" }
    );
    eprintln!();

    let widths = [36, 12, 10];
    table_header(&["Name", "Job ID", "Result"], &widths);

    for j in selected {
        let id = j["id"].as_str().unwrap_or_default();
        let name = j["name"].as_str().unwrap_or_default();
        let result = if dry_run {
            format!("{CYAN}would-enable{RESET}")
        } else {
            match c
                .put(
                    &format!("/api/cron/jobs/{id}"),
                    serde_json::json!({ "enabled": true }),
                )
                .await
            {
                Ok(_) => format!("{GREEN}enabled{RESET}"),
                Err(e) => format!("{RED}failed: {}{RESET}", truncate_id(&e.to_string(), 40)),
            }
        };
        table_row(
            &[
                format!("{ACCENT}{name}{RESET}"),
                format!("{MONO}{}{RESET}", truncate_id(id, 12)),
                result,
            ],
            &widths,
        );
    }
    eprintln!();
    Ok(())
}
