use super::*;

pub async fn cmd_schedule_list(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/cron/jobs").await.map_err(|e| { IroncladClient::check_connectivity_hint(&*e); e })?;
    heading("Cron Jobs");
    let jobs = data["jobs"].as_array();
    match jobs {
        Some(arr) if !arr.is_empty() => {
            let widths = [22, 12, 22, 10, 8];
            table_header(&["Name", "Schedule", "Last Run", "Status", "Errors"], &widths);
            for j in arr {
                let name = j["name"].as_str().unwrap_or("").to_string();
                let kind = j["schedule_kind"].as_str().unwrap_or("?");
                let expr = j["schedule_expr"].as_str().unwrap_or("");
                let schedule = format!("{kind}: {expr}");
                let last_run = j["last_run_at"].as_str().map(|t| if t.len() > 19 { &t[..19] } else { t }).unwrap_or("never").to_string();
                let status = j["last_status"].as_str().unwrap_or("pending");
                let errors = j["consecutive_errors"].as_i64().unwrap_or(0);
                table_row(&[
                    format!("{ACCENT}{name}{RESET}"),
                    truncate_id(&schedule, 12),
                    format!("{DIM}{last_run}{RESET}"),
                    status_badge(status),
                    if errors > 0 { format!("{RED}{errors}{RESET}") } else { format!("{DIM}0{RESET}") },
                ], &widths);
            }
            eprintln!(); eprintln!("    {DIM}{} job(s){RESET}", arr.len());
        }
        _ => empty_state("No cron jobs configured"),
    }
    eprintln!();
    Ok(())
}
