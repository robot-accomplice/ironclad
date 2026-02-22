use super::*;

pub async fn cmd_sessions_list(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/sessions").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    heading("Sessions");
    let sessions = data["sessions"].as_array();
    match sessions {
        Some(arr) if !arr.is_empty() => {
            let widths = [14, 18, 22, 22];
            table_header(&["ID", "Agent", "Created", "Updated"], &widths);
            for s in arr {
                let id = truncate_id(s["id"].as_str().unwrap_or(""), 11);
                let agent = s["agent_id"].as_str().unwrap_or("").to_string();
                let created = s["created_at"].as_str().unwrap_or("").to_string();
                let updated = s["updated_at"].as_str().unwrap_or("").to_string();
                table_row(
                    &[
                        format!("{MONO}{id}{RESET}"),
                        agent,
                        format!("{DIM}{created}{RESET}"),
                        format!("{DIM}{updated}{RESET}"),
                    ],
                    &widths,
                );
            }
            eprintln!();
            eprintln!("    {DIM}{} session(s){RESET}", arr.len());
        }
        _ => empty_state("No sessions found"),
    }
    eprintln!();
    Ok(())
}

pub async fn cmd_session_detail(url: &str, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let session = c.get(&format!("/api/sessions/{id}")).await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let messages = c.get(&format!("/api/sessions/{id}/messages")).await?;
    heading(&format!("Session {}", truncate_id(id, 12)));
    kv_mono("ID", id);
    kv("Agent", session["agent_id"].as_str().unwrap_or(""));
    kv("Created", session["created_at"].as_str().unwrap_or(""));
    kv("Updated", session["updated_at"].as_str().unwrap_or(""));
    let msgs = messages["messages"].as_array();
    match msgs {
        Some(arr) if !arr.is_empty() => {
            eprintln!();
            eprintln!("    {BOLD}Messages ({}):{RESET}", arr.len());
            eprintln!("    {DIM}{}{RESET}", "\u{2500}".repeat(56));
            for m in arr {
                let role = m["role"].as_str().unwrap_or("?");
                let content = m["content"].as_str().unwrap_or("");
                let time = m["created_at"].as_str().unwrap_or("");
                let role_color = match role {
                    "user" => CYAN,
                    "assistant" => GREEN,
                    "system" => YELLOW,
                    _ => DIM,
                };
                let short_time = if time.len() > 19 { &time[11..19] } else { time };
                eprintln!(
                    "    {role_color}\u{25b6}{RESET} {role_color}{BOLD}{role}{RESET} {DIM}{short_time}{RESET}"
                );
                for line in content.lines() {
                    eprintln!("      {line}");
                }
                eprintln!();
            }
        }
        _ => {
            eprintln!();
            empty_state("No messages in this session");
        }
    }
    eprintln!();
    Ok(())
}

pub async fn cmd_session_create(
    url: &str,
    agent_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let body = serde_json::json!({ "agent_id": agent_id });
    let result = c.post("/api/sessions", body).await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let session_id = result["session_id"].as_str().unwrap_or("unknown");
    eprintln!();
    eprintln!("  {OK} Session created: {MONO}{session_id}{RESET}");
    eprintln!();
    Ok(())
}

pub async fn cmd_session_export(
    base_url: &str,
    session_id: &str,
    format: &str,
    output: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let resp = reqwest::get(format!("{base_url}/api/sessions/{session_id}")).await?;
    if !resp.status().is_success() {
        eprintln!("  Session not found: {session_id}");
        return Ok(());
    }
    let session: serde_json::Value = resp.json().await?;
    let resp2 = reqwest::get(format!("{base_url}/api/sessions/{session_id}/messages")).await?;
    let body: serde_json::Value = resp2.json().await.unwrap_or_default();
    let messages: Vec<serde_json::Value> = body
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let content = match format {
        "json" => {
            let export = serde_json::json!({ "session": session, "messages": messages, "exported_at": chrono::Utc::now().to_rfc3339() });
            serde_json::to_string_pretty(&export)?
        }
        "markdown" => {
            let mut md = String::new();
            md.push_str(&format!("# Session {}\n\n", session_id));
            md.push_str(&format!(
                "**Agent:** {}\n",
                session
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            ));
            md.push_str(&format!(
                "**Created:** {}\n\n",
                session
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            ));
            md.push_str("---\n\n");
            for msg in &messages {
                let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
                let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let ts = msg.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("### {} *({ts})*\n\n{content}\n\n", role));
            }
            md
        }
        "html" => {
            let mut html = String::new();
            html.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Ironclad Session Export</title><style>");
            html.push_str("body{font-family:-apple-system,sans-serif;max-width:800px;margin:40px auto;padding:0 20px;background:#1a1a2e;color:#e0e0e0}");
            html.push_str(
                "h1{color:#8b5cf6}.msg{margin:16px 0;padding:12px 16px;border-radius:8px}",
            );
            html.push_str(".user{background:#2a2a4a;border-left:3px solid #8b5cf6}.assistant{background:#1e3a2e;border-left:3px solid #22c55e}");
            html.push_str(".system{background:#3a2a1e;border-left:3px solid #f59e0b}.role{font-weight:bold;font-size:.85em}.time{font-size:.75em;color:#888}");
            html.push_str("</style></head><body>");
            html.push_str(&format!("<h1>Session {}</h1>", session_id));
            for msg in &messages {
                let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
                let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let ts = msg.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                let class = match role {
                    "user" => "user",
                    "assistant" => "assistant",
                    "system" => "system",
                    _ => "msg",
                };
                let escaped = content
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('\n', "<br>");
                html.push_str(&format!("<div class=\"msg {class}\"><div class=\"role\">{role} <span class=\"time\">{ts}</span></div><div>{escaped}</div></div>"));
            }
            html.push_str("</body></html>");
            html
        }
        _ => {
            eprintln!("  Unknown format: {format}. Use json, html, or markdown.");
            return Ok(());
        }
    };
    match output {
        Some(path) => {
            std::fs::write(path, &content)?;
            eprintln!("  {OK} Exported to {path}");
        }
        None => print!("{content}"),
    }
    Ok(())
}
