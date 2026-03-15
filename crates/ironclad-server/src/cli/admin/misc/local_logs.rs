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
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    if follow {
        println!("  {BOLD}Tailing logs{RESET} (level >= {level}, Ctrl+C to stop)\n");

        let client = super::http_client()?;
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
        let resp = super::http_client()?
            .get(format!("{base_url}/api/logs?lines={lines}&level={level}"))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().await.unwrap_or_default();
                if json {
                    println!("{}", serde_json::to_string_pretty(&body)?);
                    return Ok(());
                }
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

