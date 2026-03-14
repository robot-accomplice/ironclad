use super::*;

// ── Skills ────────────────────────────────────────────────────

pub async fn cmd_skills_list(url: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/skills").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

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

pub async fn cmd_skill_detail(
    url: &str,
    id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url)?;
    let s = c.get(&format!("/api/skills/{id}")).await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
    }

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

pub async fn cmd_skills_catalog_list(
    url: &str,
    query: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url)?;
    let path = if let Some(q) = query {
        format!("/api/skills/catalog?q={}", crate::cli::urlencoding(q))
    } else {
        "/api/skills/catalog".to_string()
    };
    let data = c.get(&path).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }
    heading("Skills Catalog");
    let items = data["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        empty_state("No catalog skills found");
        eprintln!();
        return Ok(());
    }
    let widths = [28, 12, 16];
    table_header(&["Name", "Kind", "Source"], &widths);
    for item in items {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let kind = item
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let source = item
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        table_row(&[name, kind, source], &widths);
    }
    eprintln!();
    Ok(())
}

pub async fn cmd_skills_catalog_install(
    url: &str,
    skills: &[String],
    activate: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url)?;
    let data = c
        .post(
            "/api/skills/catalog/install",
            serde_json::json!({ "skills": skills, "activate": activate }),
        )
        .await?;
    heading("Catalog Install");
    kv("Installed", &data["installed"].to_string());
    kv("Activated", &data["activated"].to_string());
    eprintln!();
    Ok(())
}

pub async fn cmd_skills_catalog_activate(
    url: &str,
    skills: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url)?;
    let _ = c
        .post(
            "/api/skills/catalog/activate",
            serde_json::json!({ "skills": skills }),
        )
        .await?;
    heading("Catalog Activate");
    kv("Requested", &skills.join(", "));
    eprintln!();
    Ok(())
}
