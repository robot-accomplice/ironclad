use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{AreaResult, MigrationArea, SafetyVerdict, copy_dir_recursive, scan_directory_safety};

// ── OpenClaw data structures ───────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenClawConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub channels: Option<OpenClawChannels>,
    #[serde(default)]
    pub cron: Option<Vec<OpenClawCronJob>>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub(crate) struct OpenClawChannels {
    #[serde(default)]
    pub telegram: Option<OpenClawTelegramChannel>,
    #[serde(default)]
    pub whatsapp: Option<OpenClawWhatsappChannel>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenClawTelegramChannel {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenClawWhatsappChannel {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub phone_id: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenClawCronJob {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenClawSession {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<OpenClawMessage>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OpenClawMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════
// 1. Config transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_config(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let config_path = oc_root.join("openclaw.json");
    if !config_path.exists() {
        return err(
            MigrationArea::Config,
            format!("openclaw.json not found at {}", config_path.display()),
        );
    }

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to read openclaw.json: {e}"),
            );
        }
    };
    let oc_cfg: OpenClawConfig = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to parse openclaw.json: {e}"),
            );
        }
    };

    let mut warnings = Vec::new();
    let mut toml = Vec::new();

    // [agent]
    toml.push("[agent]".into());
    let name = oc_cfg.name.as_deref().unwrap_or("Migrated Agent");
    let id = name.to_lowercase().replace(' ', "-");
    toml.push(format!("name = {}", qt(name)));
    toml.push(format!("id = {}", qt(&id)));
    toml.push(format!(
        "workspace = {}",
        qt(&ic_root.join("workspace").to_string_lossy())
    ));
    toml.push(String::new());

    // [server]
    toml.push("[server]".into());
    toml.push("host = \"127.0.0.1\"".into());
    toml.push("port = 18789".into());
    toml.push(String::new());

    // [database]
    toml.push("[database]".into());
    toml.push(format!(
        "path = {}",
        qt(&ic_root.join("ironclad.db").to_string_lossy())
    ));
    toml.push(String::new());

    // [models]
    toml.push("[models]".into());
    if let Some(model) = &oc_cfg.model {
        toml.push(format!("primary = {}", qt(model)));
    } else {
        toml.push("primary = \"gpt-4\"".into());
        warnings.push("No model specified in OpenClaw config, defaulting to gpt-4".into());
    }
    toml.push("fallback = \"gpt-3.5-turbo\"".into());
    if let Some(temp) = oc_cfg.temperature {
        toml.push(format!("temperature = {temp}"));
    }
    if let Some(max) = oc_cfg.max_tokens {
        toml.push(format!("max_tokens = {max}"));
    }
    toml.push(String::new());

    // [providers.*]
    if let Some(provider) = &oc_cfg.provider {
        let key = provider.to_lowercase();
        toml.push(format!("[providers.{key}]"));
        if let Some(url) = &oc_cfg.api_url {
            toml.push(format!("base_url = {}", qt(url)));
        }
        if let Some(api_key) = &oc_cfg.api_key {
            let env_name = format!("{}_API_KEY", provider.to_uppercase());
            toml.push(format!("api_key_env = {}", qt(&env_name)));
            warnings.push(format!(
                "API key found. Set env var {env_name}={api_key} (key NOT stored in config for security)"
            ));
        }
        toml.push(String::new());
    }

    if let Err(e) = fs::create_dir_all(ic_root) {
        return err(
            MigrationArea::Config,
            format!("Failed to create output dir: {e}"),
        );
    }
    if let Err(e) = fs::write(ic_root.join("ironclad.toml"), toml.join("\n")) {
        return err(
            MigrationArea::Config,
            format!("Failed to write ironclad.toml: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Config,
        success: true,
        items_processed: 1,
        warnings,
        error: None,
    }
}

pub(crate) fn export_config(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let config_path = ic_root.join("ironclad.toml");
    if !config_path.exists() {
        return err(MigrationArea::Config, "ironclad.toml not found".into());
    }
    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to read ironclad.toml: {e}"),
            );
        }
    };
    let tv: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to parse ironclad.toml: {e}"),
            );
        }
    };

    let mut oc = serde_json::Map::new();
    let mut warnings = Vec::new();

    if let Some(name) = tv
        .get("agent")
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
    {
        oc.insert("name".into(), serde_json::Value::String(name.into()));
    }
    if let Some(models) = tv.get("models").and_then(|v| v.as_table()) {
        if let Some(p) = models.get("primary").and_then(|v| v.as_str()) {
            oc.insert("model".into(), serde_json::Value::String(p.into()));
        }
        if let Some(t) = models.get("temperature").and_then(|v| v.as_float()) {
            oc.insert("temperature".into(), serde_json::json!(t));
        }
        if let Some(m) = models.get("max_tokens").and_then(|v| v.as_integer()) {
            oc.insert("max_tokens".into(), serde_json::json!(m));
        }
    }
    if let Some(providers) = tv.get("providers").and_then(|v| v.as_table())
        && let Some((name, prov)) = providers.iter().next()
    {
        oc.insert("provider".into(), serde_json::Value::String(name.clone()));
        if let Some(url) = prov.get("base_url").and_then(|v| v.as_str()) {
            oc.insert("api_url".into(), serde_json::Value::String(url.into()));
        }
        if let Some(key_env) = prov.get("api_key_env").and_then(|v| v.as_str()) {
            if let Ok(val) = std::env::var(key_env) {
                oc.insert("api_key".into(), serde_json::Value::String(val));
            } else {
                warnings.push(format!("Env var {key_env} not set; api_key omitted"));
            }
        }
    }

    // Deep-merge with existing openclaw.json if present
    let oc_config_path = oc_root.join("openclaw.json");
    let mut merged: serde_json::Map<String, serde_json::Value> = if oc_config_path.exists() {
        fs::read_to_string(&oc_config_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    for (k, v) in oc {
        merged.insert(k, v);
    }

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Config,
            format!("Failed to create output dir: {e}"),
        );
    }
    let json = serde_json::to_string_pretty(&merged).unwrap_or_default();
    if let Err(e) = fs::write(&oc_config_path, &json) {
        return err(
            MigrationArea::Config,
            format!("Failed to write openclaw.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Config,
        success: true,
        items_processed: 1,
        warnings,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 2. Personality transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_personality(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let ws = oc_root.join("workspace");
    let soul_path = ws.join("SOUL.md");
    let agents_path = ws.join("AGENTS.md");
    let out_dir = ic_root.join("workspace");
    if let Err(e) = fs::create_dir_all(&out_dir) {
        return err(
            MigrationArea::Personality,
            format!("Failed to create workspace dir: {e}"),
        );
    }

    let mut warnings = Vec::new();
    let mut items = 0;

    if soul_path.exists() {
        match fs::read_to_string(&soul_path) {
            Ok(md) => {
                let toml_str = markdown_to_personality_toml(&md, "os");
                if let Err(e) = fs::write(out_dir.join("OS.toml"), &toml_str) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write OS.toml: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read SOUL.md: {e}"),
                );
            }
        }
    } else {
        warnings.push("SOUL.md not found; OS.toml will use defaults".into());
    }

    if agents_path.exists() {
        match fs::read_to_string(&agents_path) {
            Ok(md) => {
                let toml_str = markdown_to_personality_toml(&md, "firmware");
                if let Err(e) = fs::write(out_dir.join("FIRMWARE.toml"), &toml_str) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write FIRMWARE.toml: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read AGENTS.md: {e}"),
                );
            }
        }
    } else {
        warnings.push("AGENTS.md not found; FIRMWARE.toml will use defaults".into());
    }

    AreaResult {
        area: MigrationArea::Personality,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_personality(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let ic_ws = ic_root.join("workspace");
    let out_ws = oc_root.join("workspace");
    if let Err(e) = fs::create_dir_all(&out_ws) {
        return err(
            MigrationArea::Personality,
            format!("Failed to create workspace dir: {e}"),
        );
    }

    let mut warnings = Vec::new();
    let mut items = 0;

    let os_path = ic_ws.join("OS.toml");
    if os_path.exists() {
        match fs::read_to_string(&os_path) {
            Ok(content) => {
                let md = personality_toml_to_markdown(&content, "SOUL");
                if let Err(e) = fs::write(out_ws.join("SOUL.md"), &md) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write SOUL.md: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read OS.toml: {e}"),
                );
            }
        }
    } else {
        warnings.push("OS.toml not found; SOUL.md will be minimal".into());
    }

    let fw_path = ic_ws.join("FIRMWARE.toml");
    if fw_path.exists() {
        match fs::read_to_string(&fw_path) {
            Ok(content) => {
                let md = personality_toml_to_markdown(&content, "AGENTS");
                if let Err(e) = fs::write(out_ws.join("AGENTS.md"), &md) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write AGENTS.md: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read FIRMWARE.toml: {e}"),
                );
            }
        }
    } else {
        warnings.push("FIRMWARE.toml not found; AGENTS.md will be minimal".into());
    }

    AreaResult {
        area: MigrationArea::Personality,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn markdown_to_personality_toml(md: &str, kind: &str) -> String {
    let mut lines = vec![
        format!("# Converted from OpenClaw {kind} markdown"),
        format!("[{kind}]"),
    ];
    // Preserve full original as prompt_text for round-trip fidelity
    lines.push(format!("prompt_text = {}", qt_ml(md)));

    let mut current_section = String::new();
    let mut section_content = Vec::new();

    for line in md.lines() {
        if line.starts_with("# ") || line.starts_with("## ") {
            if !current_section.is_empty() && !section_content.is_empty() {
                let key = current_section.to_lowercase().replace([' ', '-'], "_");
                let val = section_content.join("\n");
                lines.push(format!("{key} = {}", qt_ml(&val)));
                section_content.clear();
            }
            current_section = line.trim_start_matches('#').trim().to_string();
        } else if !line.trim().is_empty() {
            section_content.push(line.to_string());
        }
    }

    if !current_section.is_empty() && !section_content.is_empty() {
        let key = current_section.to_lowercase().replace([' ', '-'], "_");
        let val = section_content.join("\n");
        lines.push(format!("{key} = {}", qt_ml(&val)));
    }

    lines.join("\n") + "\n"
}

pub(crate) fn personality_toml_to_markdown(toml_str: &str, title: &str) -> String {
    let parsed: Result<toml::Value, _> = toml::from_str(toml_str);
    match parsed {
        Ok(toml::Value::Table(table)) => {
            // Check for prompt_text (round-trip fidelity)
            for (_section_key, section_val) in &table {
                if let toml::Value::Table(inner) = section_val
                    && let Some(pt) = inner.get("prompt_text").and_then(|v| v.as_str())
                    && !pt.is_empty()
                {
                    return pt.to_string();
                }
                if let Some(pt) = section_val.as_str()
                    && _section_key == "prompt_text"
                    && !pt.is_empty()
                {
                    return pt.to_string();
                }
            }

            // Generate from structured fields
            let mut lines = vec![format!("# {title}"), String::new()];
            for (_section_key, section_val) in &table {
                if let toml::Value::Table(inner) = section_val {
                    for (key, val) in inner {
                        if key == "prompt_text" {
                            continue;
                        }
                        let heading = titlecase(key);
                        lines.push(format!("## {heading}"));
                        lines.push(String::new());
                        if let Some(s) = val.as_str() {
                            lines.push(s.to_string());
                        } else {
                            lines.push(val.to_string());
                        }
                        lines.push(String::new());
                    }
                }
            }
            lines.join("\n") + "\n"
        }
        _ => format!("# {title}\n\n{toml_str}\n"),
    }
}

fn titlecase(key: &str) -> String {
    key.replace('_', " ")
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ═══════════════════════════════════════════════════════════════════════
// 3. Skills transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_skills(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let skills_dir = oc_root.join("workspace").join("skills");
    if !skills_dir.exists() {
        return AreaResult {
            area: MigrationArea::Skills,
            success: true,
            items_processed: 0,
            warnings: vec!["No skills directory found in OpenClaw workspace".into()],
            error: None,
        };
    }

    let out_dir = ic_root.join("skills");
    if let Err(e) = fs::create_dir_all(&out_dir) {
        return err(
            MigrationArea::Skills,
            format!("Failed to create skills dir: {e}"),
        );
    }

    let report = scan_directory_safety(&skills_dir);
    let mut warnings = Vec::new();

    if let SafetyVerdict::Critical(n) = report.verdict {
        return AreaResult {
            area: MigrationArea::Skills, success: false, items_processed: 0,
            warnings: vec![format!("{n} critical safety finding(s); import blocked")],
            error: Some("Skills blocked by safety check. Use standalone skill import with --no-safety-check to override.".into()),
        };
    }
    if let SafetyVerdict::Warnings(n) = report.verdict {
        warnings.push(format!(
            "{n} warning(s) found in skill scripts; review recommended"
        ));
    }

    let mut items = 0;
    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let src = entry.path();
            let dest = out_dir.join(entry.file_name());
            if src.is_file() {
                if let Err(e) = fs::copy(&src, &dest) {
                    warnings.push(format!("Failed to copy {}: {e}", src.display()));
                } else {
                    items += 1;
                }
            } else if src.is_dir() {
                if let Err(e) = copy_dir_recursive(&src, &dest) {
                    warnings.push(format!("Failed to copy dir {}: {e}", src.display()));
                } else {
                    items += 1;
                }
            }
        }
    }

    AreaResult {
        area: MigrationArea::Skills,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_skills(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let skills_dir = ic_root.join("skills");
    if !skills_dir.exists() {
        return AreaResult {
            area: MigrationArea::Skills,
            success: true,
            items_processed: 0,
            warnings: vec!["No skills directory found in Ironclad workspace".into()],
            error: None,
        };
    }

    let out_dir = oc_root.join("workspace").join("skills");
    if let Err(e) = fs::create_dir_all(&out_dir) {
        return err(
            MigrationArea::Skills,
            format!("Failed to create output skills dir: {e}"),
        );
    }

    let mut items = 0;
    let mut warnings = Vec::new();
    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let src = entry.path();
            let dest = out_dir.join(entry.file_name());
            if src.is_file() {
                if let Err(e) = fs::copy(&src, &dest) {
                    warnings.push(format!("Failed to copy {}: {e}", src.display()));
                } else {
                    items += 1;
                }
            } else if src.is_dir() {
                if let Err(e) = copy_dir_recursive(&src, &dest) {
                    warnings.push(format!("Failed to copy dir {}: {e}", src.display()));
                } else {
                    items += 1;
                }
            }
        }
    }

    AreaResult {
        area: MigrationArea::Skills,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 4. Sessions transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_sessions(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let mut all_sessions: Vec<OpenClawSession> = Vec::new();
    let mut warnings = Vec::new();

    // sessions.json (top-level array)
    let sessions_json = oc_root.join("sessions.json");
    if sessions_json.exists() {
        match fs::read_to_string(&sessions_json) {
            Ok(c) => match serde_json::from_str::<Vec<OpenClawSession>>(&c) {
                Ok(s) => all_sessions.extend(s),
                Err(e) => warnings.push(format!("Failed to parse sessions.json: {e}")),
            },
            Err(e) => warnings.push(format!("Failed to read sessions.json: {e}")),
        }
    }

    // agents/<agent>/sessions/*.jsonl
    let agents_dir = oc_root.join("agents");
    if agents_dir.exists()
        && let Ok(agents) = fs::read_dir(&agents_dir)
    {
        for agent_entry in agents.flatten() {
            let sess_dir = agent_entry.path().join("sessions");
            if !sess_dir.exists() {
                continue;
            }
            if let Ok(files) = fs::read_dir(&sess_dir) {
                for file in files.flatten() {
                    let path = file.path();
                    match path.extension().and_then(|e| e.to_str()) {
                        Some("jsonl") => {
                            if let Ok(content) = fs::read_to_string(&path) {
                                let msgs: Vec<OpenClawMessage> = content
                                    .lines()
                                    .filter_map(|l| serde_json::from_str(l).ok())
                                    .collect();
                                all_sessions.push(OpenClawSession {
                                    id: Some(
                                        path.file_stem()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into(),
                                    ),
                                    agent_id: Some(
                                        agent_entry.file_name().to_string_lossy().into(),
                                    ),
                                    created_at: None,
                                    messages: Some(msgs),
                                });
                            }
                        }
                        Some("json") => {
                            if let Ok(content) = fs::read_to_string(&path)
                                && let Ok(s) = serde_json::from_str::<OpenClawSession>(&content)
                            {
                                all_sessions.push(s);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if all_sessions.is_empty() && !sessions_json.exists() {
        return AreaResult {
            area: MigrationArea::Sessions,
            success: true,
            items_processed: 0,
            warnings: vec!["No sessions found to import".into()],
            error: None,
        };
    }

    let db_path = ic_root.join("ironclad.db");
    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => {
            return err(
                MigrationArea::Sessions,
                format!("Failed to open database: {e}"),
            );
        }
    };

    let conn = db.conn();
    let mut items = 0;
    for session in &all_sessions {
        let default_id = uuid_v4();
        let sid = session.id.as_deref().unwrap_or(&default_id);
        let agent = session.agent_id.as_deref().unwrap_or("default");
        let default_ts = now_iso();
        let created = session.created_at.as_deref().unwrap_or(&default_ts);

        if let Err(e) = conn.execute(
            "INSERT OR IGNORE INTO sessions (id, agent_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![sid, agent, created],
        ) {
            warnings.push(format!("Failed to insert session {sid}: {e}"));
            continue;
        }

        if let Some(msgs) = &session.messages {
            for msg in msgs {
                let mid = uuid_v4();
                let role = msg.role.as_deref().unwrap_or("user");
                let content = msg.content.as_deref().unwrap_or("");
                let ts = msg.timestamp.as_deref().unwrap_or(created);
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO session_messages (id, session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![mid, sid, role, content, ts],
                );
            }
        }
        items += 1;
    }

    AreaResult {
        area: MigrationArea::Sessions,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_sessions(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let db_path = ic_root.join("ironclad.db");
    if !db_path.exists() {
        return AreaResult {
            area: MigrationArea::Sessions,
            success: true,
            items_processed: 0,
            warnings: vec!["No database found".into()],
            error: None,
        };
    }

    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => {
            return err(
                MigrationArea::Sessions,
                format!("Failed to open database: {e}"),
            );
        }
    };

    let conn = db.conn();
    let mut warnings = Vec::new();
    let mut all: Vec<serde_json::Value> = Vec::new();

    let mut stmt =
        match conn.prepare("SELECT id, agent_id, created_at FROM sessions ORDER BY created_at") {
            Ok(s) => s,
            Err(e) => {
                return err(
                    MigrationArea::Sessions,
                    format!("Failed to query sessions: {e}"),
                );
            }
        };
    let sessions: Vec<(String, String, String)> = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            return err(
                MigrationArea::Sessions,
                format!("Failed to iterate sessions: {e}"),
            );
        }
    };

    for (sid, agent_id, created_at) in &sessions {
        let mut msg_stmt = match conn.prepare(
            "SELECT role, content, created_at FROM session_messages WHERE session_id = ?1 ORDER BY created_at"
        ) {
            Ok(s) => s,
            Err(e) => { warnings.push(format!("Failed to query msgs for {sid}: {e}")); continue; }
        };
        let messages: Vec<serde_json::Value> = msg_stmt
            .query_map(rusqlite::params![sid], |row| {
                Ok(serde_json::json!({
                    "role": row.get::<_, String>(0)?,
                    "content": row.get::<_, String>(1)?,
                    "timestamp": row.get::<_, String>(2)?,
                }))
            })
            .map(|iter| iter.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        all.push(serde_json::json!({
            "id": sid, "agent_id": agent_id, "created_at": created_at, "messages": messages,
        }));
    }

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Sessions,
            format!("Failed to create output dir: {e}"),
        );
    }
    if let Err(e) = fs::write(
        oc_root.join("sessions.json"),
        serde_json::to_string_pretty(&all).unwrap_or_default(),
    ) {
        return err(
            MigrationArea::Sessions,
            format!("Failed to write sessions.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Sessions,
        success: true,
        items_processed: all.len(),
        warnings,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 5. Cron transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_cron(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let mut jobs = Vec::new();
    let mut warnings = Vec::new();

    let jobs_json = oc_root.join("jobs.json");
    if jobs_json.exists() {
        match fs::read_to_string(&jobs_json) {
            Ok(c) => match serde_json::from_str::<Vec<OpenClawCronJob>>(&c) {
                Ok(parsed) => jobs.extend(parsed),
                Err(e) => warnings.push(format!("Failed to parse jobs.json: {e}")),
            },
            Err(e) => warnings.push(format!("Failed to read jobs.json: {e}")),
        }
    }

    let config_path = oc_root.join("openclaw.json");
    if config_path.exists()
        && let Ok(c) = fs::read_to_string(&config_path)
        && let Ok(cfg) = serde_json::from_str::<OpenClawConfig>(&c)
        && let Some(cj) = cfg.cron
    {
        jobs.extend(cj);
    }

    if jobs.is_empty() {
        return AreaResult {
            area: MigrationArea::Cron,
            success: true,
            items_processed: 0,
            warnings: vec!["No cron jobs found to import".into()],
            error: None,
        };
    }

    let db_path = ic_root.join("ironclad.db");
    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => return err(MigrationArea::Cron, format!("Failed to open database: {e}")),
    };

    let conn = db.conn();
    let mut items = 0;
    for job in &jobs {
        let id = uuid_v4();
        let name = job.name.as_deref().unwrap_or("unnamed");
        let schedule = job.schedule.as_deref().unwrap_or("0 * * * *");
        let command = job.command.as_deref().unwrap_or("");
        let enabled = job.enabled.unwrap_or(true);
        let payload = serde_json::json!({ "command": command }).to_string();

        match conn.execute(
            "INSERT OR IGNORE INTO cron_jobs (id, name, enabled, schedule_kind, schedule_expr, agent_id, payload_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, name, enabled, "cron", schedule, "default", payload],
        ) {
            Ok(_) => items += 1,
            Err(e) => warnings.push(format!("Failed to insert cron job '{name}': {e}")),
        }
    }

    AreaResult {
        area: MigrationArea::Cron,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_cron(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let db_path = ic_root.join("ironclad.db");
    if !db_path.exists() {
        return AreaResult {
            area: MigrationArea::Cron,
            success: true,
            items_processed: 0,
            warnings: vec!["No database found".into()],
            error: None,
        };
    }

    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => return err(MigrationArea::Cron, format!("Failed to open database: {e}")),
    };

    let conn = db.conn();
    let mut stmt = match conn
        .prepare("SELECT name, schedule_expr, payload_json, enabled FROM cron_jobs ORDER BY name")
    {
        Ok(s) => s,
        Err(e) => {
            return err(
                MigrationArea::Cron,
                format!("Failed to query cron jobs: {e}"),
            );
        }
    };

    let jobs: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            let payload_str: String = row.get(2)?;
            let payload: serde_json::Value = serde_json::from_str(&payload_str).unwrap_or_default();
            let command = payload
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(serde_json::json!({
                "name": row.get::<_, String>(0)?,
                "schedule": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "command": command,
                "enabled": row.get::<_, bool>(3)?,
            }))
        })
        .map(|iter| iter.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Cron,
            format!("Failed to create output dir: {e}"),
        );
    }
    if let Err(e) = fs::write(
        oc_root.join("jobs.json"),
        serde_json::to_string_pretty(&jobs).unwrap_or_default(),
    ) {
        return err(
            MigrationArea::Cron,
            format!("Failed to write jobs.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Cron,
        success: true,
        items_processed: jobs.len(),
        warnings: vec![],
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 6. Channels transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_channels(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let config_path = oc_root.join("openclaw.json");
    if !config_path.exists() {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No openclaw.json found; skipping channel import".into()],
            error: None,
        };
    }
    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Channels,
                format!("Failed to read openclaw.json: {e}"),
            );
        }
    };
    let oc_cfg: OpenClawConfig = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Channels,
                format!("Failed to parse openclaw.json: {e}"),
            );
        }
    };

    let mut items = 0;
    let mut warnings = Vec::new();
    let mut lines = vec!["[channels]".to_string()];

    if let Some(channels) = &oc_cfg.channels {
        if let Some(tg) = &channels.telegram {
            lines.push(String::new());
            lines.push("[channels.telegram]".into());
            lines.push(format!("enabled = {}", tg.enabled.unwrap_or(false)));
            if let Some(token) = &tg.token {
                lines.push("token_env = \"TELEGRAM_BOT_TOKEN\"".into());
                warnings.push(format!(
                    "Set env var TELEGRAM_BOT_TOKEN={token} (token NOT stored in config)"
                ));
            }
            items += 1;
        }
        if let Some(wa) = &channels.whatsapp {
            lines.push(String::new());
            lines.push("[channels.whatsapp]".into());
            lines.push(format!("enabled = {}", wa.enabled.unwrap_or(false)));
            if let Some(token) = &wa.token {
                lines.push("token_env = \"WHATSAPP_TOKEN\"".into());
                warnings.push(format!(
                    "Set env var WHATSAPP_TOKEN={token} (token NOT stored in config)"
                ));
            }
            if let Some(phone) = &wa.phone_id {
                lines.push(format!("phone_id = {}", qt(phone)));
            }
            items += 1;
        }
    }

    if items == 0 {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No channel configuration found in OpenClaw config".into()],
            error: None,
        };
    }

    if let Err(e) = fs::create_dir_all(ic_root) {
        return err(
            MigrationArea::Channels,
            format!("Failed to create dir: {e}"),
        );
    }
    if let Err(e) = fs::write(ic_root.join("channels.toml"), lines.join("\n") + "\n") {
        return err(
            MigrationArea::Channels,
            format!("Failed to write channels.toml: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Channels,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_channels(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let channels_path = ic_root.join("channels.toml");
    let config_path = ic_root.join("ironclad.toml");
    let mut warnings = Vec::new();

    let channel_toml = if channels_path.exists() {
        fs::read_to_string(&channels_path).unwrap_or_default()
    } else if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No channel configuration found".into()],
            error: None,
        };
    };

    let parsed: toml::Value = match toml::from_str(&channel_toml) {
        Ok(v) => v,
        Err(_) => {
            return AreaResult {
                area: MigrationArea::Channels,
                success: true,
                items_processed: 0,
                warnings: vec!["Could not parse channel config".into()],
                error: None,
            };
        }
    };

    let mut oc_channels = serde_json::Map::new();
    let mut items = 0;

    if let Some(channels) = parsed.get("channels").and_then(|v| v.as_table()) {
        if let Some(tg) = channels.get("telegram").and_then(|v| v.as_table()) {
            let mut obj = serde_json::Map::new();
            if let Some(e) = tg.get("enabled").and_then(|v| v.as_bool()) {
                obj.insert("enabled".into(), serde_json::Value::Bool(e));
            }
            if let Some(env) = tg.get("token_env").and_then(|v| v.as_str()) {
                if let Ok(tok) = std::env::var(env) {
                    obj.insert("token".into(), serde_json::Value::String(tok));
                } else {
                    warnings.push(format!("Env var {env} not set; telegram token omitted"));
                }
            }
            oc_channels.insert("telegram".into(), serde_json::Value::Object(obj));
            items += 1;
        }
        if let Some(wa) = channels.get("whatsapp").and_then(|v| v.as_table()) {
            let mut obj = serde_json::Map::new();
            if let Some(e) = wa.get("enabled").and_then(|v| v.as_bool()) {
                obj.insert("enabled".into(), serde_json::Value::Bool(e));
            }
            if let Some(env) = wa.get("token_env").and_then(|v| v.as_str()) {
                if let Ok(tok) = std::env::var(env) {
                    obj.insert("token".into(), serde_json::Value::String(tok));
                } else {
                    warnings.push(format!("Env var {env} not set; whatsapp token omitted"));
                }
            }
            if let Some(phone) = wa.get("phone_id").and_then(|v| v.as_str()) {
                obj.insert("phone_id".into(), serde_json::Value::String(phone.into()));
            }
            oc_channels.insert("whatsapp".into(), serde_json::Value::Object(obj));
            items += 1;
        }
    }

    if items == 0 {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No channel definitions found to export".into()],
            error: None,
        };
    }

    // Merge into existing openclaw.json
    let oc_config_path = oc_root.join("openclaw.json");
    let mut oc_config: serde_json::Map<String, serde_json::Value> = if oc_config_path.exists() {
        fs::read_to_string(&oc_config_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    oc_config.insert("channels".into(), serde_json::Value::Object(oc_channels));

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Channels,
            format!("Failed to create output dir: {e}"),
        );
    }
    if let Err(e) = fs::write(
        &oc_config_path,
        serde_json::to_string_pretty(&oc_config).unwrap_or_default(),
    ) {
        return err(
            MigrationArea::Channels,
            format!("Failed to write openclaw.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Channels,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

pub(crate) fn qt(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn qt_ml(s: &str) -> String {
    format!("\"\"\"\n{}\n\"\"\"", s)
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn err(area: MigrationArea, msg: String) -> AreaResult {
    AreaResult {
        area,
        success: false,
        items_processed: 0,
        warnings: vec![],
        error: Some(msg),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_openclaw(dir: &Path) {
        fs::create_dir_all(dir.join("workspace/skills")).unwrap();
        fs::create_dir_all(dir.join("agents/duncan/sessions")).unwrap();

        let config = serde_json::json!({
            "name": "Duncan Idaho",
            "model": "gpt-4",
            "provider": "openai",
            "api_url": "https://api.openai.com/v1",
            "temperature": 0.7,
            "max_tokens": 4096,
            "channels": {
                "telegram": { "enabled": true, "token": "tg-token" },
                "whatsapp": { "enabled": false, "token": "wa-token", "phone_id": "12345" }
            },
            "cron": [
                { "name": "heartbeat", "schedule": "*/5 * * * *", "command": "ping", "enabled": true },
                { "name": "cleanup", "schedule": "0 3 * * *", "command": "cleanup", "enabled": false }
            ]
        });
        fs::write(
            dir.join("openclaw.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        fs::write(
            dir.join("workspace/SOUL.md"),
            "# Soul\n\n## Identity\nI am Duncan Idaho.\n\n## Traits\nLoyal, fierce, skilled.\n",
        )
        .unwrap();
        fs::write(
            dir.join("workspace/AGENTS.md"),
            "# Agents\n\n## Capabilities\nFighting, strategy, leadership.\n",
        )
        .unwrap();

        fs::write(
            dir.join("workspace/skills/greet.sh"),
            "#!/bin/bash\necho hello\n",
        )
        .unwrap();
        fs::write(dir.join("workspace/skills/math.py"), "print(2+2)\n").unwrap();

        let session = serde_json::json!([{
            "id": "sess-001", "agent_id": "duncan", "created_at": "2025-01-01T00:00:00Z",
            "messages": [
                { "role": "user", "content": "Hello", "timestamp": "2025-01-01T00:00:01Z" },
                { "role": "assistant", "content": "Hi there!", "timestamp": "2025-01-01T00:00:02Z" }
            ]
        }]);
        fs::write(
            dir.join("sessions.json"),
            serde_json::to_string_pretty(&session).unwrap(),
        )
        .unwrap();

        let jsonl = "{\"role\":\"user\",\"content\":\"JSONL msg\",\"timestamp\":\"2025-01-02T00:00:00Z\"}\n{\"role\":\"assistant\",\"content\":\"Reply\",\"timestamp\":\"2025-01-02T00:00:01Z\"}";
        fs::write(dir.join("agents/duncan/sessions/sess-002.jsonl"), jsonl).unwrap();
    }

    fn setup_ironclad(dir: &Path) {
        fs::create_dir_all(dir.join("workspace")).unwrap();
        fs::create_dir_all(dir.join("skills")).unwrap();

        fs::write(dir.join("ironclad.toml"), "[agent]\nname = \"Duncan Idaho\"\nid = \"duncan\"\nworkspace = \"/tmp/workspace\"\n\n[server]\nhost = \"127.0.0.1\"\nport = 18789\n\n[database]\npath = \"/tmp/ironclad.db\"\n\n[models]\nprimary = \"gpt-4\"\nfallback = \"gpt-3.5-turbo\"\ntemperature = 0.7\nmax_tokens = 4096\n").unwrap();
        fs::write(dir.join("channels.toml"), "[channels.telegram]\nenabled = true\ntoken_env = \"TELEGRAM_BOT_TOKEN\"\n\n[channels.whatsapp]\nenabled = false\ntoken_env = \"WHATSAPP_TOKEN\"\nphone_id = \"12345\"\n").unwrap();
        fs::write(dir.join("workspace/OS.toml"), "[os]\nprompt_text = \"\"\"\\n# Soul\\n\\n## Identity\\nI am Duncan.\\n\"\"\"\nidentity = \"I am Duncan.\"\n").unwrap();
        fs::write(
            dir.join("workspace/FIRMWARE.toml"),
            "[firmware]\ncapabilities = \"Fighting, strategy.\"\n",
        )
        .unwrap();
        fs::write(dir.join("skills/greet.gosh"), "echo hello\n").unwrap();
        fs::write(dir.join("skills/math.py"), "print(2+2)\n").unwrap();
    }

    // ── Config ─────────────────────────────────────────────────

    #[test]
    fn import_config_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        let r = import_config(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 1);
        let content = fs::read_to_string(ic.path().join("ironclad.toml")).unwrap();
        assert!(content.contains("Duncan Idaho"));
        assert!(content.contains("gpt-4"));
    }

    #[test]
    fn import_config_missing_file() {
        let r = import_config(Path::new("/nonexistent"), Path::new("/tmp"));
        assert!(!r.success);
    }

    #[test]
    fn export_config_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_config(ic.path(), oc.path());
        assert!(r.success);
        let content = fs::read_to_string(oc.path().join("openclaw.json")).unwrap();
        assert!(content.contains("Duncan Idaho"));
        assert!(content.contains("gpt-4"));
    }

    #[test]
    fn config_roundtrip() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let oc2 = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        assert!(import_config(oc.path(), ic.path()).success);
        assert!(export_config(ic.path(), oc2.path()).success);
        let exported: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(oc2.path().join("openclaw.json")).unwrap())
                .unwrap();
        assert_eq!(exported["name"], "Duncan Idaho");
        assert_eq!(exported["model"], "gpt-4");
    }

    #[test]
    fn export_config_merge_preserves_unknown_fields() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        fs::write(
            oc.path().join("openclaw.json"),
            r#"{"custom_field":"preserved","name":"old"}"#,
        )
        .unwrap();
        let r = export_config(ic.path(), oc.path());
        assert!(r.success);
        let exported: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(oc.path().join("openclaw.json")).unwrap())
                .unwrap();
        assert_eq!(exported["custom_field"], "preserved");
        assert_eq!(exported["name"], "Duncan Idaho");
    }

    // ── Personality ────────────────────────────────────────────

    #[test]
    fn import_personality_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        let r = import_personality(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(ic.path().join("workspace/OS.toml").exists());
        assert!(ic.path().join("workspace/FIRMWARE.toml").exists());
    }

    #[test]
    fn export_personality_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_personality(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(oc.path().join("workspace/SOUL.md").exists());
        assert!(oc.path().join("workspace/AGENTS.md").exists());
    }

    #[test]
    fn personality_roundtrip_via_prompt_text() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let oc2 = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        assert!(import_personality(oc.path(), ic.path()).success);
        assert!(export_personality(ic.path(), oc2.path()).success);
        let original = fs::read_to_string(oc.path().join("workspace/SOUL.md")).unwrap();
        let exported = fs::read_to_string(oc2.path().join("workspace/SOUL.md")).unwrap();
        assert_eq!(original.trim(), exported.trim());
    }

    #[test]
    fn personality_missing_files_warns() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        fs::create_dir_all(oc.path().join("workspace")).unwrap();
        let r = import_personality(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
        assert_eq!(r.warnings.len(), 2);
    }

    // ── Skills ─────────────────────────────────────────────────

    #[test]
    fn import_skills_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        let r = import_skills(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(ic.path().join("skills/greet.sh").exists());
    }

    #[test]
    fn import_skills_no_dir() {
        let r = import_skills(Path::new("/nonexistent"), Path::new("/tmp"));
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    #[test]
    fn export_skills_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_skills(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
    }

    // ── Sessions ───────────────────────────────────────────────

    #[test]
    fn import_sessions_from_json() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        let r = import_sessions(oc.path(), ic.path());
        assert!(r.success);
        assert!(r.items_processed >= 1);
        assert!(ic.path().join("ironclad.db").exists());
    }

    #[test]
    fn import_sessions_from_jsonl() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        fs::remove_file(oc.path().join("sessions.json")).unwrap();
        let r = import_sessions(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 1);
    }

    #[test]
    fn export_sessions_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        import_sessions(oc.path(), ic.path());
        let out = TempDir::new().unwrap();
        let r = export_sessions(ic.path(), out.path());
        assert!(r.success);
        assert!(r.items_processed >= 1);
        assert!(out.path().join("sessions.json").exists());
    }

    #[test]
    fn sessions_no_data() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let r = import_sessions(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    // ── Cron ───────────────────────────────────────────────────

    #[test]
    fn import_cron_from_config() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        let r = import_cron(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
    }

    #[test]
    fn import_cron_from_jobs_json() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let jobs = serde_json::json!([{ "name": "daily", "schedule": "0 0 * * *", "command": "report", "enabled": true }]);
        fs::write(
            oc.path().join("jobs.json"),
            serde_json::to_string(&jobs).unwrap(),
        )
        .unwrap();
        let r = import_cron(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 1);
    }

    #[test]
    fn export_cron_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        import_cron(oc.path(), ic.path());
        let out = TempDir::new().unwrap();
        let r = export_cron(ic.path(), out.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(out.path().join("jobs.json").exists());
    }

    #[test]
    fn cron_no_data() {
        let r = import_cron(Path::new("/nonexistent"), Path::new("/tmp"));
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    // ── Channels ───────────────────────────────────────────────

    #[test]
    fn import_channels_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_openclaw(oc.path());
        let r = import_channels(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(ic.path().join("channels.toml").exists());
    }

    #[test]
    fn export_channels_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_channels(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
    }

    #[test]
    fn channels_no_config() {
        let r = import_channels(Path::new("/nonexistent"), Path::new("/tmp"));
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    // ── Personality conversion ─────────────────────────────────

    #[test]
    fn markdown_to_toml_has_prompt_text() {
        let md = "# Title\n\n## Identity\nI am Duncan.\n";
        let toml = markdown_to_personality_toml(md, "os");
        assert!(toml.contains("[os]"));
        assert!(toml.contains("prompt_text"));
        assert!(toml.contains("identity"));
    }

    #[test]
    fn toml_to_markdown_uses_prompt_text() {
        let toml_str = "[os]\nprompt_text = \"\"\"\n# Soul\n\n## Identity\nI am Duncan.\n\"\"\"\n";
        let md = personality_toml_to_markdown(toml_str, "SOUL");
        assert!(md.contains("# Soul"));
        assert!(md.contains("Duncan"));
    }

    // ── Helpers ────────────────────────────────────────────────

    #[test]
    fn qt_escapes() {
        assert_eq!(qt("hello"), "\"hello\"");
        assert_eq!(qt("he\"llo"), "\"he\\\"llo\"");
        assert_eq!(qt("a\\b"), "\"a\\\\b\"");
    }
}
