use super::*;

// ── Skills ────────────────────────────────────────────────────

pub async fn cmd_skills_list(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/skills").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

    heading("Skills");

    let skills = data["skills"].as_array();
    match skills {
        Some(arr) if !arr.is_empty() => {
            let widths = [22, 14, 34, 9];
            table_header(&["Name", "Kind", "Description", "Enabled"], &widths);
            for s in arr {
                let name = s["name"].as_str().unwrap_or("").to_string();
                let kind = s["kind"].as_str().unwrap_or("").to_string();
                let desc = truncate_id(s["description"].as_str().unwrap_or(""), 31);
                let enabled = if s["enabled"].as_bool().unwrap_or(false)
                    || s["enabled"].as_i64().map(|v| v != 0).unwrap_or(false)
                {
                    format!("{OK} yes")
                } else {
                    format!("{RED}{ERR} no{RESET}")
                };
                table_row(
                    &[format!("{ACCENT}{name}{RESET}"), kind, desc, enabled],
                    &widths,
                );
            }
            eprintln!();
            eprintln!("    {DIM}{} skill(s){RESET}", arr.len());
        }
        _ => empty_state("No skills registered"),
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_skill_detail(url: &str, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url)?;
    let s = c.get(&format!("/api/skills/{id}")).await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

    heading(&format!("Skill: {}", s["name"].as_str().unwrap_or(id)));

    kv_mono("ID", s["id"].as_str().unwrap_or(""));
    kv_accent("Name", s["name"].as_str().unwrap_or(""));
    kv("Kind", s["kind"].as_str().unwrap_or(""));
    kv("Description", s["description"].as_str().unwrap_or(""));
    kv_mono("Source", s["source_path"].as_str().unwrap_or(""));
    kv_mono("Hash", s["content_hash"].as_str().unwrap_or(""));

    let enabled = if s["enabled"].as_bool().unwrap_or(false)
        || s["enabled"].as_i64().map(|v| v != 0).unwrap_or(false)
    {
        status_badge("running")
    } else {
        status_badge("dead")
    };
    kv("Enabled", &enabled);

    if let Some(triggers) = s["triggers_json"].as_str()
        && !triggers.is_empty()
        && triggers != "null"
    {
        kv("Triggers", triggers);
    }
    if let Some(script) = s["script_path"].as_str()
        && !script.is_empty()
        && script != "null"
    {
        kv_mono("Script", script);
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_skills_reload(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    c.post("/api/skills/reload", serde_json::json!({}))
        .await
        .map_err(|e| {
            IroncladClient::check_connectivity_hint(&*e);
            e
        })?;
    eprintln!();
    eprintln!("  {OK} Skills reloaded from disk");
    eprintln!();
    Ok(())
}
