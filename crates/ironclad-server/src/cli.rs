#![allow(non_snake_case, unused_variables)]

use std::sync::OnceLock;

use reqwest::Client;
use serde_json::Value;

use ironclad_core::style::Theme;

// ── CRT draw macros ──────────────────────────────────────────
// Shadow the standard println!/eprintln! so ALL output in this
// module goes through the typewriter when draw is enabled.
const CRT_DRAW_MS: u64 = 4;

macro_rules! println {
    () => {{
        use std::io::Write;
        std::io::stdout().write_all(b"\n").ok();
        std::io::stdout().flush().ok();
    }};
    ($($arg:tt)*) => {{
        let __text = format!($($arg)*);
        theme().typewrite_line_stdout(&__text, CRT_DRAW_MS);
    }};
}

macro_rules! eprintln {
    () => {{
        use std::io::Write;
        std::io::stderr().write_all(b"\n").ok();
    }};
    ($($arg:tt)*) => {{
        let __text = format!($($arg)*);
        theme().typewrite_line(&__text, CRT_DRAW_MS);
    }};
}

// ── Theme ────────────────────────────────────────────────────

static THEME: OnceLock<Theme> = OnceLock::new();

/// Initialize the global theme from `--color` and `--no-draw` flags.
pub fn init_theme(color_flag: &str, no_draw: bool) {
    let t = Theme::from_flag(color_flag);
    let t = if no_draw { t.with_draw(false) } else { t };
    let _ = THEME.set(t);
}

pub fn theme() -> &'static Theme {
    THEME.get_or_init(Theme::detect)
}

/// Returns (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO).
#[allow(clippy::type_complexity)]
pub(crate) fn colors() -> (&'static str, &'static str, &'static str, &'static str,
                &'static str, &'static str, &'static str, &'static str, &'static str) {
    let t = theme();
    (t.dim(), t.bold(), t.accent(), t.success(), t.warn(), t.error(), t.info(), t.reset(), t.mono())
}

pub struct IroncladClient {
    client: Client,
    base_url: String,
}

impl IroncladClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("failed to build HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, Box<dyn std::error::Error>> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {body}").into());
        }
        Ok(resp.json().await?)
    }

    async fn post(
        &self,
        path: &str,
        body: Value,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {text}").into());
        }
        Ok(resp.json().await?)
    }

    fn check_connectivity_hint(e: &dyn std::error::Error) {
        let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
        let msg = format!("{e:?}");
        if msg.contains("Connection refused")
            || msg.contains("ConnectionRefused")
            || msg.contains("ConnectError")
            || msg.contains("connect error")
        {
            eprintln!();
            eprintln!(
                "  \u{26a0}\u{fe0f} Is the Ironclad server running? Start it with: {BOLD}ironclad serve{RESET}"
            );
        }
    }
}

// ── Formatting helpers ───────────────────────────────────────────

fn heading(text: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    eprintln!();
    eprintln!("  \u{2705} {BOLD}{text}{RESET}");
    eprintln!("  {DIM}{}{RESET}", "\u{2500}".repeat(60));
}

fn kv(key: &str, value: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    eprintln!("    {DIM}{key:<20}{RESET} {value}");
}

fn kv_accent(key: &str, value: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    eprintln!("    {DIM}{key:<20}{RESET} {ACCENT}{value}{RESET}");
}

fn kv_mono(key: &str, value: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    eprintln!("    {DIM}{key:<20}{RESET} {MONO}{value}{RESET}");
}

fn badge(text: &str, color: &str) -> String {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    format!("{color}\u{25cf} {text}{RESET}")
}

fn status_badge(status: &str) -> String {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    match status {
        "ok" | "running" | "success" => badge(status, GREEN),
        "sleeping" | "pending" | "warning" => badge(status, YELLOW),
        "dead" | "error" | "failed" => badge(status, RED),
        _ => badge(status, DIM),
    }
}

fn truncate_id(id: &str, len: usize) -> String {
    if id.len() > len {
        format!("{}...", &id[..len])
    } else {
        id.to_string()
    }
}

fn table_separator(widths: &[usize]) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let parts: Vec<String> = widths.iter().map(|w| "\u{2500}".repeat(*w)).collect();
    eprintln!("    {DIM}\u{251c}{}\u{2524}{RESET}", parts.join("\u{253c}"));
}

fn table_header(headers: &[&str], widths: &[usize]) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let cells: Vec<String> = headers
        .iter()
        .zip(widths)
        .map(|(h, w)| format!("{BOLD}{h:<width$}{RESET}", width = w))
        .collect();
    eprintln!("    {DIM}\u{2502}{RESET}{}{DIM}\u{2502}{RESET}", cells.join(&format!("{DIM}\u{2502}{RESET}")));
    table_separator(widths);
}

fn table_row(cells: &[String], widths: &[usize]) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let formatted: Vec<String> = cells
        .iter()
        .zip(widths)
        .map(|(c, w)| {
            let visible_len = strip_ansi_len(c);
            if visible_len >= *w {
                c.clone()
            } else {
                format!("{c}{}", " ".repeat(w - visible_len))
            }
        })
        .collect();
    eprintln!("    {DIM}\u{2502}{RESET}{}{DIM}\u{2502}{RESET}", formatted.join(&format!("{DIM}\u{2502}{RESET}")));
}

fn strip_ansi_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

fn empty_state(msg: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    eprintln!("    {DIM}\u{2500}\u{2500} {msg}{RESET}");
}

// ── Commands ─────────────────────────────────────────────────────

pub async fn cmd_status(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url);

    heading("Agent Status");

    let health = c.get("/api/health").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let agent = c.get("/api/agent/status").await?;
    let config = c.get("/api/config").await?;
    let sessions = c.get("/api/sessions").await?;
    let skills = c.get("/api/skills").await?;
    let jobs = c.get("/api/cron/jobs").await?;
    let cache = c.get("/api/stats/cache").await?;
    let wallet = c.get("/api/wallet/balance").await?;

    let agent_name = config["agent"]["name"].as_str().unwrap_or("unknown");
    let agent_id = config["agent"]["id"].as_str().unwrap_or("unknown");
    let agent_state = agent["state"].as_str().unwrap_or("unknown");
    let version = health["version"].as_str().unwrap_or("?");
    let session_count = sessions["sessions"].as_array().map(|a| a.len()).unwrap_or(0);
    let skill_count = skills["skills"].as_array().map(|a| a.len()).unwrap_or(0);
    let job_count = jobs["jobs"].as_array().map(|a| a.len()).unwrap_or(0);
    let hits = cache["hits"].as_u64().unwrap_or(0);
    let misses = cache["misses"].as_u64().unwrap_or(0);
    let hit_rate = if hits + misses > 0 {
        format!("{:.1}%", hits as f64 / (hits + misses) as f64 * 100.0)
    } else {
        "n/a".into()
    };
    let balance = wallet["balance"].as_str().unwrap_or("0.00");
    let currency = wallet["currency"].as_str().unwrap_or("USDC");

    kv_accent("Agent", &format!("{agent_name} ({agent_id})"));
    kv("State", &status_badge(agent_state).to_string());
    kv_accent("Version", version);
    kv("Sessions", &session_count.to_string());
    kv("Skills", &skill_count.to_string());
    kv("Cron Jobs", &job_count.to_string());
    kv("Cache Hit Rate", &hit_rate);
    kv_accent("Balance", &format!("{balance} {currency}"));

    let primary = config["models"]["primary"].as_str().unwrap_or("unknown");
    kv("Primary Model", primary);

    eprintln!();
    Ok(())
}

pub async fn cmd_sessions_list(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
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

pub async fn cmd_session_detail(
    url: &str,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
    let session = c.get(&format!("/api/sessions/{id}")).await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let messages = c.get(&format!("/api/sessions/{id}/messages")).await?;

    heading(&format!("Session {}", truncate_id(id, 12)));

    kv_mono("ID", id);
    kv("Agent", session["agent_id"].as_str().unwrap_or(""));
    kv(
        "Created",
        session["created_at"].as_str().unwrap_or(""),
    );
    kv(
        "Updated",
        session["updated_at"].as_str().unwrap_or(""),
    );

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
    let c = IroncladClient::new(url);
    let body = serde_json::json!({ "agent_id": agent_id });
    let result = c.post("/api/sessions", body).await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

    let session_id = result["session_id"].as_str().unwrap_or("unknown");
    eprintln!();
    eprintln!(
        "  \u{2705} Session created: {MONO}{session_id}{RESET}"
    );
    eprintln!();

    Ok(())
}

pub async fn cmd_memory(
    url: &str,
    tier: &str,
    session_id: Option<&str>,
    query: Option<&str>,
    limit: Option<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);

    match tier {
        "working" => {
            let sid = session_id.ok_or("--session required for working memory")?;
            let data = c.get(&format!("/api/memory/working/{sid}")).await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading("Working Memory");
            let entries = data["entries"].as_array();
            match entries {
                Some(arr) if !arr.is_empty() => {
                    let widths = [12, 14, 36, 10];
                    table_header(&["ID", "Type", "Content", "Importance"], &widths);
                    for e in arr {
                        table_row(
                            &[
                                format!("{MONO}{}{RESET}", truncate_id(e["id"].as_str().unwrap_or(""), 9)),
                                e["entry_type"].as_str().unwrap_or("").to_string(),
                                truncate_id(e["content"].as_str().unwrap_or(""), 33),
                                e["importance"].to_string(),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} entries{RESET}", arr.len());
                }
                _ => empty_state("No working memory entries"),
            }
        }
        "episodic" => {
            let lim = limit.unwrap_or(20);
            let data = c.get(&format!("/api/memory/episodic?limit={lim}")).await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading("Episodic Memory");
            let entries = data["entries"].as_array();
            match entries {
                Some(arr) if !arr.is_empty() => {
                    let widths = [12, 16, 36, 10];
                    table_header(&["ID", "Classification", "Content", "Importance"], &widths);
                    for e in arr {
                        table_row(
                            &[
                                format!("{MONO}{}{RESET}", truncate_id(e["id"].as_str().unwrap_or(""), 9)),
                                e["classification"].as_str().unwrap_or("").to_string(),
                                truncate_id(e["content"].as_str().unwrap_or(""), 33),
                                e["importance"].to_string(),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} entries (limit: {lim}){RESET}", arr.len());
                }
                _ => empty_state("No episodic memory entries"),
            }
        }
        "semantic" => {
            let category = session_id.unwrap_or("general");
            let data = c.get(&format!("/api/memory/semantic/{category}")).await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading(&format!("Semantic Memory [{category}]"));
            let entries = data["entries"].as_array();
            match entries {
                Some(arr) if !arr.is_empty() => {
                    let widths = [20, 34, 12];
                    table_header(&["Key", "Value", "Confidence"], &widths);
                    for e in arr {
                        table_row(
                            &[
                                format!("{ACCENT}{}{RESET}", e["key"].as_str().unwrap_or("")),
                                truncate_id(e["value"].as_str().unwrap_or(""), 31),
                                format!("{:.2}", e["confidence"].as_f64().unwrap_or(0.0)),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} entries{RESET}", arr.len());
                }
                _ => empty_state("No semantic memory entries in this category"),
            }
        }
        "search" => {
            let q = query.ok_or("--query/-q required for memory search")?;
            let data = c.get(&format!("/api/memory/search?q={}", urlencoding(q))).await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading(&format!("Memory Search: \"{q}\""));
            let results = data["results"].as_array();
            match results {
                Some(arr) if !arr.is_empty() => {
                    for (i, r) in arr.iter().enumerate() {
                        let fallback = r.to_string();
                        let text = r.as_str().unwrap_or(&fallback);
                        eprintln!("    {DIM}{:>3}.{RESET} {text}", i + 1);
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} results{RESET}", arr.len());
                }
                _ => empty_state("No results found"),
            }
        }
        _ => {
            return Err(format!("unknown memory tier: {tier}. Use: working, episodic, semantic, search").into());
        }
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_skills_list(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
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
                    format!("\u{2705} yes")
                } else {
                    format!("{RED}\u{26d3} no{RESET}")
                };
                table_row(
                    &[
                        format!("{ACCENT}{name}{RESET}"),
                        kind,
                        desc,
                        enabled,
                    ],
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
) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url);
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

    if let Some(triggers) = s["triggers_json"].as_str() {
        if !triggers.is_empty() && triggers != "null" {
            kv("Triggers", triggers);
        }
    }
    if let Some(script) = s["script_path"].as_str() {
        if !script.is_empty() && script != "null" {
            kv_mono("Script", script);
        }
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_skills_reload(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
    c.post("/api/skills/reload", serde_json::json!({})).await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    eprintln!();
    eprintln!("  \u{2705} Skills reloaded from disk");
    eprintln!();
    Ok(())
}

pub async fn cmd_cron_list(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
    let data = c.get("/api/cron/jobs").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

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
                let last_run = j["last_run_at"]
                    .as_str()
                    .map(|t| if t.len() > 19 { &t[..19] } else { t })
                    .unwrap_or("never")
                    .to_string();
                let status = j["last_status"]
                    .as_str()
                    .unwrap_or("pending");
                let errors = j["consecutive_errors"].as_i64().unwrap_or(0);
                table_row(
                    &[
                        format!("{ACCENT}{name}{RESET}"),
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

pub async fn cmd_metrics(
    url: &str,
    kind: &str,
    hours: Option<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);

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
                    table_header(&["Model", "Provider", "Tokens In", "Tokens Out", "Cost", "Cached"], &widths);

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
                                    format!("\u{2705}")
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
            let data = c.get(&format!("/api/stats/transactions?hours={h}")).await.map_err(|e| {
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
                        let counter = t["counterparty"]
                            .as_str()
                            .unwrap_or("-")
                            .to_string();
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
            return Err(format!("unknown metric kind: {kind}. Use: costs, transactions, cache").into());
        }
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_wallet(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
    let balance = c.get("/api/wallet/balance").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let address = c.get("/api/wallet/address").await?;

    heading("Wallet");

    let bal = balance["balance"].as_str().unwrap_or("0.00");
    let currency = balance["currency"].as_str().unwrap_or("USDC");
    let addr = address["address"].as_str().unwrap_or("not connected");

    kv_accent("Balance", &format!("{bal} {currency}"));
    kv_mono("Address", addr);

    if let Some(note) = balance["note"].as_str() {
        eprintln!();
        eprintln!("    {DIM}\u{2139}  {note}{RESET}");
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_config(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
    let data = c.get("/api/config").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

    heading("Configuration");

    let sections = [
        "agent", "server", "database", "models", "memory", "cache",
        "treasury", "yield", "wallet", "a2a", "skills", "channels",
        "circuit_breaker", "providers",
    ];

    for section in sections {
        if let Some(val) = data.get(section) {
            if val.is_null() {
                continue;
            }
            eprintln!();
            eprintln!("    \u{25b8} {section}{RESET}");
            print_json_section(val, 6);
        }
    }

    eprintln!();
    Ok(())
}

fn print_json_section(val: &Value, indent: usize) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let pad = " ".repeat(indent);
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                match v {
                    Value::Object(_) => {
                        eprintln!("{pad}{DIM}{k}:{RESET}");
                        print_json_section(v, indent + 2);
                    }
                    Value::Array(arr) => {
                        let items: Vec<String> = arr
                            .iter()
                            .map(|i| format!("{}", format_json_val(i)))
                            .collect();
                        eprintln!("{pad}{DIM}{k:<22}{RESET} [{MONO}{}{RESET}]", items.join(", "));
                    }
                    _ => {
                        eprintln!("{pad}{DIM}{k:<22}{RESET} {}", format_json_val(v));
                    }
                }
            }
        }
        _ => {
            eprintln!("{pad}{}", format_json_val(val));
        }
    }
}

fn format_json_val(v: &Value) -> String {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    match v {
        Value::String(s) => format!("{MONO}{s}{RESET}"),
        Value::Number(n) => format!("{ACCENT}{n}{RESET}"),
        Value::Bool(b) => {
            if *b {
                format!("{GREEN}{b}{RESET}")
            } else {
                format!("{YELLOW}{b}{RESET}")
            }
        }
        Value::Null => format!("{DIM}null{RESET}"),
        _ => v.to_string(),
    }
}

pub async fn cmd_breaker(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let c = IroncladClient::new(url);
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

pub async fn cmd_agents_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get(format!("{base_url}/api/agents")).await?;
    let body: serde_json::Value = resp.json().await?;

    let agents = body.get("agents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if agents.is_empty() {
        println!("\n  No agents registered.\n");
        return Ok(());
    }

    println!("\n  {:<15} {:<20} {:<10} {:<15}", "ID", "Name", "State", "Model");
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

pub async fn cmd_plugins_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get(format!("{base_url}/api/plugins")).await?;
    let body: serde_json::Value = resp.json().await?;

    let plugins = body
        .get("plugins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if plugins.is_empty() {
        println!("\n  No plugins installed.\n");
        return Ok(());
    }

    println!("\n  {:<20} {:<10} {:<10} {:<10}", "Plugin", "Version", "Status", "Tools");
    println!("  {}", "─".repeat(55));
    for p in &plugins {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let version = p.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let tools = p
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        println!(
            "  {:<20} {:<10} {:<10} {:<10}",
            name,
            version,
            status,
            tools
        );
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

    println!("\n  {:<15} {:<10} {:<10} {:<10}", "Channel", "Status", "Recv", "Sent");
    println!("  {}", "─".repeat(50));
    for ch in &channels {
        let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let connected = ch.get("connected").and_then(|v| v.as_bool()).unwrap_or(false);
        let status_str = if connected { "✓ up" } else { "✗ down" };
        let recv = ch.get("messages_received").and_then(|v| v.as_u64()).unwrap_or(0);
        let sent = ch.get("messages_sent").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("  {:<15} {:<10} {:<10} {:<10}", name, status_str, recv, sent);
    }
    println!();
    Ok(())
}

fn urlencoding(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('#', "%23")
}

pub async fn cmd_mechanic(base_url: &str, repair: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    println!("\n  {BOLD}Ironclad Mechanic{RESET}{}\n", if repair { " (--repair mode)" } else { "" });

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let ironclad_dir = std::path::PathBuf::from(&home).join(".ironclad");
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
            println!("  \u{2705} Directory exists: {}", dir.display());
        } else if repair {
            std::fs::create_dir_all(dir)?;
            println!("  \u{26a1} Created directory: {}", dir.display());
            fixed += 1;
        } else {
            println!("  \u{26a0}\u{fe0f} Missing directory: {} (use --repair to create)", dir.display());
        }
    }

    // Check config file
    let config_path = std::path::Path::new("ironclad.toml");
    let alt_config = ironclad_dir.join("ironclad.toml");
    if config_path.exists() || alt_config.exists() {
        println!("  \u{2705} Configuration file found");
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
        println!("  \u{26a1} Created default config: {}", alt_config.display());
        fixed += 1;
    } else {
        println!("  \u{26a0}\u{fe0f} No config file found (use --repair or `ironclad init`)");
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
                        println!("  \u{26a1} Set permissions 600 on {}", file.display());
                        fixed += 1;
                    } else {
                        println!("  \u{26a0}\u{fe0f} {} has loose permissions ({:o}) - use --repair", file.display(), mode);
                    }
                } else {
                    println!("  \u{2705} {} permissions OK ({:o})", file.display(), mode);
                }
            }
        }
    }

    // Check Go toolchain
    match which_binary("go") {
        Some(path) => {
            let ver = std::process::Command::new("go").arg("version").output().ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let ver = ver.trim().strip_prefix("go version ").unwrap_or(ver.trim());
            println!("  \u{2705} Go toolchain: {ver} ({path})");
        }
        None => {
            println!("  {RED}\u{26d3}{RESET} Go not found (required for gosh plugin engine)");
            println!("         Install from https://go.dev/dl/ or: brew install go");
        }
    }

    // Check gosh scripting engine
    match which_binary("gosh") {
        Some(path) => {
            println!("  \u{2705} gosh scripting engine: {path}");
        }
        None if repair => {
            if which_binary("go").is_some() {
                println!("  \u{26a1} Installing gosh...");
                let result = std::process::Command::new("go")
                    .args(["install", "github.com/drewwalton19216801/gosh@latest"])
                    .status();
                match result {
                    Ok(s) if s.success() => {
                        println!("  \u{26a1} gosh installed via go install");
                        fixed += 1;
                    }
                    _ => {
                        println!("  {RED}\u{26d3}{RESET} Failed to install gosh. Try manually:");
                        println!("         go install github.com/drewwalton19216801/gosh@latest");
                    }
                }
            } else {
                println!("  \u{26a0}\u{fe0f} gosh not found (install Go first, then: go install github.com/drewwalton19216801/gosh@latest)");
            }
        }
        None => {
            println!("  \u{26a0}\u{fe0f} gosh not found (use --repair to install, or: go install github.com/drewwalton19216801/gosh@latest)");
        }
    }

    // Check gateway reachability first -- all subsequent server checks depend on this
    let gateway_up = match reqwest::get(format!("{base_url}/api/health")).await {
        Ok(resp) if resp.status().is_success() => {
            println!("  \u{2705} Gateway reachable at {base_url}");
            true
        }
        Ok(resp) => {
            println!("  \u{26a0}\u{fe0f} Gateway returned HTTP {}", resp.status());
            false
        }
        Err(_) => {
            println!("  \u{26a0}\u{fe0f} Gateway not running at {base_url}");
            false
        }
    };

    if gateway_up {
        // Config
        match reqwest::get(format!("{base_url}/api/config")).await {
            Ok(resp) if resp.status().is_success() => {
                println!("  \u{2705} Configuration loaded on server");
            }
            Ok(resp) => {
                println!("  \u{26a0}\u{fe0f} Config endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  \u{26a0}\u{fe0f} Config check failed: {e}");
            }
        }

        // Skills
        match reqwest::get(format!("{base_url}/api/skills")).await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let count = body.get("skills").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                println!("  \u{2705} Skills loaded ({count} skills)");
            }
            Ok(resp) => {
                println!("  \u{26a0}\u{fe0f} Skills endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  \u{26a0}\u{fe0f} Skills check failed: {e}");
            }
        }

        // Wallet
        match reqwest::get(format!("{base_url}/api/wallet/balance")).await {
            Ok(resp) if resp.status().is_success() => {
                println!("  \u{2705} Wallet accessible");
            }
            Ok(resp) => {
                println!("  \u{26a0}\u{fe0f} Wallet endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  \u{26a0}\u{fe0f} Wallet check failed: {e}");
            }
        }

        // Channels
        match reqwest::get(format!("{base_url}/api/channels/status")).await {
            Ok(resp) if resp.status().is_success() => {
                let body: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
                let active = body.iter().filter(|c| c.get("connected").and_then(|v| v.as_bool()).unwrap_or(false)).count();
                println!("  \u{2705} Channels ({active}/{} connected)", body.len());
            }
            Ok(resp) => {
                println!("  \u{26a0}\u{fe0f} Channels endpoint returned HTTP {}", resp.status());
            }
            Err(e) => {
                println!("  \u{26a0}\u{fe0f} Channels check failed: {e}");
            }
        }
    } else {
        println!("    \u{25b8} Skipping server checks (config, skills, wallet, channels)");
    }

    println!();
    if repair && fixed > 0 {
        println!("  \u{26a1} Auto-fixed {fixed} issue(s)");
    }
    println!();
    Ok(())
}

pub fn cmd_config_get(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = find_config_file()?;
    let contents = std::fs::read_to_string(&config_path)?;
    let table: toml::Value = contents.parse()?;

    let value = navigate_toml(&table, path);
    match value {
        Some(v) => {
            println!("{}", format_toml_value(v));
        }
        None => {
            eprintln!("  Key not found: {path}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub fn cmd_config_set(path: &str, value: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let contents = std::fs::read_to_string(file)
        .unwrap_or_else(|_| String::new());
    let mut table: toml::Value = if contents.is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        contents.parse()?
    };

    set_toml_value(&mut table, path, value)?;

    let output = toml::to_string_pretty(&table)?;
    std::fs::write(file, output)?;
    println!("  \u{2705} Set {path} = {value} in {file}");
    Ok(())
}

pub fn cmd_config_unset(path: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let contents = std::fs::read_to_string(file)?;
    let mut table: toml::Value = contents.parse()?;

    if remove_toml_key(&mut table, path) {
        let output = toml::to_string_pretty(&table)?;
        std::fs::write(file, output)?;
        println!("  \u{2705} Removed {path} from {file}");
    } else {
        eprintln!("  Key not found: {path}");
    }
    Ok(())
}

fn find_config_file() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let candidates = [
        std::path::PathBuf::from("ironclad.toml"),
        dirs_home().join("ironclad.toml"),
    ];
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    Err("No ironclad.toml found in current directory or ~/.ironclad/".into())
}

fn dirs_home() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".ironclad")
}

fn navigate_toml<'a>(table: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = table;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}

fn format_toml_value(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(a) => {
            let items: Vec<String> = a.iter().map(|i| format_toml_value(i)).collect();
            format!("[{}]", items.join(", "))
        }
        toml::Value::Table(_) => toml::to_string_pretty(v).unwrap_or_else(|_| format!("{v:?}")),
        toml::Value::Datetime(d) => d.to_string(),
    }
}

fn set_toml_value(table: &mut toml::Value, path: &str, value: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = table;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let toml::Value::Table(map) = current {
                let parsed_value = parse_toml_value(value);
                map.insert(part.to_string(), parsed_value);
            }
        } else {
            if current.get(part).is_none() {
                if let toml::Value::Table(map) = current {
                    map.insert(part.to_string(), toml::Value::Table(toml::map::Map::new()));
                }
            }
            current = current.get_mut(part)
                .ok_or_else(|| format!("cannot navigate to {part}"))?;
        }
    }

    Ok(())
}

fn remove_toml_key(table: &mut toml::Value, path: &str) -> bool {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.len() == 1 {
        if let toml::Value::Table(map) = table {
            return map.remove(parts[0]).is_some();
        }
        return false;
    }

    let mut current = table;
    for part in &parts[..parts.len() - 1] {
        current = match current.get_mut(part) {
            Some(v) => v,
            None => return false,
        };
    }

    if let toml::Value::Table(map) = current {
        map.remove(*parts.last().unwrap()).is_some()
    } else {
        false
    }
}

fn parse_toml_value(s: &str) -> toml::Value {
    if s == "true" { return toml::Value::Boolean(true); }
    if s == "false" { return toml::Value::Boolean(false); }
    if let Ok(i) = s.parse::<i64>() { return toml::Value::Integer(i); }
    if let Ok(f) = s.parse::<f64>() { return toml::Value::Float(f); }
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len()-1];
        let items: Vec<toml::Value> = inner.split(',')
            .map(|item| parse_toml_value(item.trim().trim_matches('"')))
            .collect();
        return toml::Value::Array(items);
    }
    toml::Value::String(s.trim_matches('"').to_string())
}

pub fn cmd_completion(shell: &str) -> Result<(), Box<dyn std::error::Error>> {
    match shell {
        "bash" => {
            println!("# Ironclad bash completion");
            println!("# Add to ~/.bashrc: eval \"$(ironclad completion bash)\"");
            println!("complete -W \"serve init check version status sessions memory skills cron metrics wallet config breaker channels plugins mechanic daemon completion\" ironclad");
        }
        "zsh" => {
            println!("# Ironclad zsh completion");
            println!("# Add to ~/.zshrc: eval \"$(ironclad completion zsh)\"");
            println!("compctl -k \"(serve init check version status sessions memory skills cron metrics wallet config breaker channels plugins mechanic daemon completion)\" ironclad");
        }
        "fish" => {
            println!("# Ironclad fish completion");
            println!("# Run: ironclad completion fish | source");
            for cmd in ["serve", "init", "check", "version", "status", "sessions", "memory", "skills", "cron", "metrics", "wallet", "config", "breaker", "channels", "plugins", "mechanic", "daemon", "completion"] {
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
    println!("\n  {BOLD}Ironclad Uninstall{RESET}\n");

    match crate::daemon::uninstall_daemon() {
        Ok(()) => println!("  \u{2705} Daemon service removed"),
        Err(e) => println!("  \u{26a0}\u{fe0f} Daemon removal: {e}"),
    }

    if purge {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let data_dir = std::path::Path::new(&home).join(".ironclad");
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)?;
            println!("  \u{2705} Removed {}", data_dir.display());
        } else {
            println!("  \u{26a0}\u{fe0f} Data directory not found: {}", data_dir.display());
        }
    } else {
        println!("  {DIM}Data preserved at ~/.ironclad/ (use --purge to remove){RESET}");
    }

    println!("\n  {GREEN}Uninstall complete.{RESET} CLI binary remains at current location.\n");
    Ok(())
}

pub fn cmd_reset(yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
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

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let ironclad_dir = std::path::Path::new(&home).join(".ironclad");

    let db_path = ironclad_dir.join("state.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
        println!("  \u{2705} Database cleared");
    }

    let db_wal = ironclad_dir.join("state.db-wal");
    if db_wal.exists() { let _ = std::fs::remove_file(&db_wal); }
    let db_shm = ironclad_dir.join("state.db-shm");
    if db_shm.exists() { let _ = std::fs::remove_file(&db_shm); }

    let config_path = ironclad_dir.join("ironclad.toml");
    if config_path.exists() {
        std::fs::remove_file(&config_path)?;
        println!("  \u{2705} Configuration removed (re-run `ironclad init` to recreate)");
    }

    let logs_dir = ironclad_dir.join("logs");
    if logs_dir.exists() {
        std::fs::remove_dir_all(&logs_dir)?;
        println!("  \u{2705} Logs cleared");
    }

    let wallet_dir = ironclad_dir.join("wallet.json");
    if wallet_dir.exists() {
        println!("  \u{26a0}\u{fe0f} Wallet preserved: {}", wallet_dir.display());
    }

    println!("\n  {GREEN}Reset complete.{RESET}\n");
    Ok(())
}

pub async fn cmd_update(
    channel: &str,
    _yes: bool,
    _no_restart: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    println!("\n  {BOLD}Ironclad Update{RESET}\n");

    let current = env!("CARGO_PKG_VERSION");
    println!("  Current version: v{current}");
    println!("  Channel: {channel}");

    println!("  Checking for updates...");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let resp = client
        .get("https://api.github.com/repos/ironclad/ironclad/releases/latest")
        .header("User-Agent", format!("ironclad/{current}"))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json().await?;
            let latest = body.get("tag_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .trim_start_matches('v');

            if latest == current {
                println!("  \u{2705} Already on latest version (v{current})");
            } else {
                println!("  \u{26a0}\u{fe0f} New version available: v{latest}");
                println!();
                println!("  To update, download the latest release:");
                println!("    curl -fsSL https://github.com/ironclad/ironclad/releases/latest | bash");
                println!();
                println!("  Or build from source:");
                println!("    cargo install --path crates/ironclad-server");
            }
        }
        Ok(r) => {
            println!("  \u{26a0}\u{fe0f} GitHub API returned {}", r.status());
            println!("  Could not check for updates. Try again later.");
        }
        Err(e) => {
            println!("  \u{26a0}\u{fe0f} Could not reach GitHub: {e}");
            println!("  Check your internet connection and try again.");
        }
    }

    println!();
    Ok(())
}

pub async fn cmd_models_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let resp = reqwest::get(format!("{base_url}/api/config")).await?;
    let config: serde_json::Value = resp.json().await?;

    println!("\n  {BOLD}Configured Models{RESET}\n");

    let primary = config.pointer("/models/primary")
        .and_then(|v| v.as_str())
        .unwrap_or("not set");
    println!("  {:<12} {}", format!("{GREEN}primary{RESET}"), primary);

    if let Some(fallbacks) = config.pointer("/models/fallbacks").and_then(|v| v.as_array()) {
        for (i, fb) in fallbacks.iter().enumerate() {
            let name = fb.as_str().unwrap_or("?");
            println!("  {:<12} {}", format!("{YELLOW}fallback {}{RESET}", i + 1), name);
        }
    }

    let mode = config.pointer("/models/routing/mode")
        .and_then(|v| v.as_str())
        .unwrap_or("rule");
    let threshold = config.pointer("/models/routing/confidence_threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.9);
    let local_first = config.pointer("/models/routing/local_first")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    println!();
    println!("  {DIM}Routing: mode={mode}, threshold={threshold}, local_first={local_first}{RESET}");
    println!();
    Ok(())
}

pub async fn cmd_models_scan(base_url: &str, provider: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    println!("\n  {BOLD}Scanning for available models...{RESET}\n");

    let resp = reqwest::get(format!("{base_url}/api/config")).await?;
    let config: serde_json::Value = resp.json().await?;

    let providers = config.get("providers")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    if providers.is_empty() {
        println!("  No providers configured.");
        println!();
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    for (name, prov_config) in &providers {
        if let Some(filter) = provider {
            if name != filter {
                continue;
            }
        }

        let url = prov_config.get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if url.is_empty() {
            println!("  {YELLOW}{name}{RESET}: no URL configured");
            continue;
        }

        let models_url = if url.contains("localhost") || url.contains("127.0.0.1") || url.contains("11434") {
            format!("{url}/api/tags")
        } else {
            format!("{url}/v1/models")
        };

        print!("  {CYAN}{name}{RESET} ({url}): ");

        match client.get(&models_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let models: Vec<String> = if let Some(arr) = body.get("models").and_then(|v| v.as_array()) {
                    arr.iter()
                        .filter_map(|m| m.get("name").or_else(|| m.get("model")).and_then(|v| v.as_str()))
                        .map(String::from)
                        .collect()
                } else if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
                    arr.iter()
                        .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                        .map(String::from)
                        .collect()
                } else {
                    vec![]
                };

                if models.is_empty() {
                    println!("no models found");
                } else {
                    println!("{} model(s)", models.len());
                    for model in &models {
                        println!("    - {model}");
                    }
                }
            }
            Ok(resp) => {
                println!("{RED}error: {}{RESET}", resp.status());
            }
            Err(e) => {
                println!("{RED}unreachable: {e}{RESET}");
            }
        }
    }

    println!();
    Ok(())
}

pub async fn cmd_session_export(
    base_url: &str,
    session_id: &str,
    format: &str,
    output: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let resp = reqwest::get(format!("{base_url}/api/sessions/{session_id}")).await?;
    if !resp.status().is_success() {
        eprintln!("  Session not found: {session_id}");
        return Ok(());
    }
    let session: serde_json::Value = resp.json().await?;

    let resp2 = reqwest::get(format!("{base_url}/api/sessions/{session_id}/messages")).await?;
    let body: serde_json::Value = resp2.json().await.unwrap_or_default();
    let messages: Vec<serde_json::Value> = body.get("messages").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    let content = match format {
        "json" => {
            let export = serde_json::json!({
                "session": session,
                "messages": messages,
                "exported_at": chrono::Utc::now().to_rfc3339(),
            });
            serde_json::to_string_pretty(&export)?
        }
        "markdown" => {
            let mut md = String::new();
            md.push_str(&format!("# Session {}\n\n", session_id));
            md.push_str(&format!("**Agent:** {}\n", session.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?")));
            md.push_str(&format!("**Created:** {}\n\n", session.get("created_at").and_then(|v| v.as_str()).unwrap_or("?")));
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
            html.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\">");
            html.push_str("<title>Ironclad Session Export</title>");
            html.push_str("<style>");
            html.push_str("body { font-family: -apple-system, sans-serif; max-width: 800px; margin: 40px auto; padding: 0 20px; background: #1a1a2e; color: #e0e0e0; }");
            html.push_str("h1 { color: #8b5cf6; }");
            html.push_str(".msg { margin: 16px 0; padding: 12px 16px; border-radius: 8px; }");
            html.push_str(".user { background: #2a2a4a; border-left: 3px solid #8b5cf6; }");
            html.push_str(".assistant { background: #1e3a2e; border-left: 3px solid #22c55e; }");
            html.push_str(".system { background: #3a2a1e; border-left: 3px solid #f59e0b; }");
            html.push_str(".role { font-weight: bold; font-size: 0.85em; text-transform: uppercase; margin-bottom: 4px; }");
            html.push_str(".time { font-size: 0.75em; color: #888; }");
            html.push_str("pre { background: #111; padding: 8px; border-radius: 4px; overflow-x: auto; }");
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
                let escaped = content.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>");
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
            eprintln!("  \u{2705} Exported to {path}");
        }
        None => {
            print!("{content}");
        }
    }

    Ok(())
}

pub fn cmd_onboard() -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    use dialoguer::{Input, Select, Confirm};

    println!("\n  {BOLD}Ironclad Setup Wizard{RESET}\n");
    println!("  This wizard will help you create an ironclad.toml configuration.\n");

    // Prerequisites: Go + gosh (plugin scripting engine)
    println!("  {BOLD}Checking prerequisites...{RESET}\n");
    let has_go = which_binary("go").is_some();
    let has_gosh = which_binary("gosh").is_some();

    if !has_go {
        println!("  \u{26a0}\u{fe0f} Go is not installed (required for the gosh plugin engine).");
        println!("     Install from {CYAN}https://go.dev/dl/{RESET} or: {MONO}brew install go{RESET}");
        println!();
        let proceed = Confirm::new()
            .with_prompt("  Continue without Go? (plugins won't work until Go + gosh are installed)")
            .default(true)
            .interact()?;
        if !proceed {
            println!("\n  Setup paused. Install Go, then re-run {BOLD}ironclad init{RESET}.\n");
            return Ok(());
        }
    } else if !has_gosh {
        println!("  \u{2705} Go found");
        println!("  \u{26a0}\u{fe0f} gosh scripting engine not found.");
        let install_now = Confirm::new()
            .with_prompt("  Install gosh now via `go install`?")
            .default(true)
            .interact()?;
        if install_now {
            println!("  Installing gosh...");
            let result = std::process::Command::new("go")
                .args(["install", "github.com/drewwalton19216801/gosh@latest"])
                .status();
            match result {
                Ok(s) if s.success() => {
                    println!("  \u{2705} gosh installed successfully");
                }
                _ => {
                    println!("  \u{26a0}\u{fe0f} gosh installation failed. Install manually:");
                    println!("     {MONO}go install github.com/drewwalton19216801/gosh@latest{RESET}");
                }
            }
        } else {
            println!("  Skipped. Install later: {MONO}go install github.com/drewwalton19216801/gosh@latest{RESET}");
        }
    } else {
        println!("  \u{2705} Go found");
        println!("  \u{2705} gosh scripting engine found");
    }
    println!();

    // 1. Agent name
    let agent_name: String = Input::new()
        .with_prompt("  Agent name")
        .default("MyAgent".into())
        .interact_text()?;

    // 2. LLM provider
    let providers = vec!["Ollama (local)", "OpenAI", "Anthropic", "Google AI"];
    let provider_idx = Select::new()
        .with_prompt("  Select LLM provider")
        .items(&providers)
        .default(0)
        .interact()?;

    let (provider_prefix, needs_api_key) = match provider_idx {
        0 => ("ollama", false),
        1 => ("openai", true),
        2 => ("anthropic", true),
        3 => ("google", true),
        _ => ("ollama", false),
    };

    // 3. API key
    let api_key = if needs_api_key {
        let key: String = Input::new()
            .with_prompt("  API key (or press Enter to set later)")
            .allow_empty(true)
            .interact_text()?;
        if key.is_empty() { None } else { Some(key) }
    } else {
        None
    };

    // 4. Model selection
    let default_model = match provider_idx {
        0 => "ollama/qwen3:8b",
        1 => "openai/gpt-4o",
        2 => "anthropic/claude-sonnet-4-20250514",
        3 => "google/gemini-2.5-pro",
        _ => "ollama/qwen3:8b",
    };
    let model: String = Input::new()
        .with_prompt("  Model")
        .default(default_model.into())
        .interact_text()?;

    // 5. Server port
    let port: String = Input::new()
        .with_prompt("  Server port")
        .default("18789".into())
        .interact_text()?;
    let port_num: u16 = port.parse().unwrap_or(18789);

    // 6. Channels
    let enable_telegram = Confirm::new()
        .with_prompt("  Enable Telegram channel?")
        .default(false)
        .interact()?;

    let telegram_token = if enable_telegram {
        let token: String = Input::new()
            .with_prompt("  Telegram bot token")
            .interact_text()?;
        Some(token)
    } else {
        None
    };

    let enable_discord = Confirm::new()
        .with_prompt("  Enable Discord channel?")
        .default(false)
        .interact()?;

    let discord_token = if enable_discord {
        let token: String = Input::new()
            .with_prompt("  Discord bot token")
            .interact_text()?;
        Some(token)
    } else {
        None
    };

    // 7. Workspace directory
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let default_workspace = format!("{home}/.ironclad/workspace");
    let workspace: String = Input::new()
        .with_prompt("  Workspace directory")
        .default(default_workspace)
        .interact_text()?;

    // 8. Database path
    let default_db = format!("{home}/.ironclad/state.db");
    let db_path: String = Input::new()
        .with_prompt("  Database path")
        .default(default_db)
        .interact_text()?;

    // Generate config
    let mut config = String::new();
    config.push_str("# Ironclad Configuration (generated by onboard wizard)\n\n");
    config.push_str("[agent]\n");
    config.push_str(&format!("name = \"{agent_name}\"\n"));
    config.push_str(&format!("id = \"{}\"\n", agent_name.to_lowercase().replace(' ', "-")));
    config.push_str(&format!("workspace = \"{workspace}\"\n"));
    config.push_str("log_level = \"info\"\n\n");

    config.push_str("[server]\n");
    config.push_str(&format!("port = {port_num}\n"));
    config.push_str("bind = \"127.0.0.1\"\n\n");

    config.push_str("[database]\n");
    config.push_str(&format!("path = \"{db_path}\"\n\n"));

    config.push_str("[models]\n");
    config.push_str(&format!("primary = \"{model}\"\n"));
    config.push_str("fallbacks = []\n\n");

    config.push_str("[models.routing]\n");
    config.push_str("mode = \"rule\"\n");
    config.push_str("confidence_threshold = 0.9\n");
    config.push_str("local_first = true\n\n");

    config.push_str("# Bundled provider defaults (ollama, openai, anthropic, google, openrouter)\n");
    config.push_str("# are auto-merged. Override or add new providers here.\n");
    if api_key.is_some() {
        config.push_str(&format!("# Set the API key via env: {}_API_KEY\n\n", provider_prefix.to_uppercase()));
    } else {
        config.push_str("\n");
    }

    config.push_str("[memory]\n");
    config.push_str("working_budget_pct = 30.0\n");
    config.push_str("episodic_budget_pct = 25.0\n");
    config.push_str("semantic_budget_pct = 20.0\n");
    config.push_str("procedural_budget_pct = 15.0\n");
    config.push_str("relationship_budget_pct = 10.0\n\n");

    config.push_str("[treasury]\n");
    config.push_str("per_payment_cap = 100.0\n");
    config.push_str("hourly_transfer_limit = 500.0\n");
    config.push_str("daily_transfer_limit = 2000.0\n");
    config.push_str("minimum_reserve = 5.0\n");
    config.push_str("daily_inference_budget = 50.0\n\n");

    if let Some(ref token) = telegram_token {
        config.push_str("[channels.telegram]\n");
        config.push_str(&format!("token = \"{token}\"\n\n"));
    }

    if let Some(ref token) = discord_token {
        config.push_str("[channels.discord]\n");
        config.push_str(&format!("token = \"{token}\"\n\n"));
    }

    config.push_str("[skills]\n");
    config.push_str(&format!("skills_dir = \"{home}/.ironclad/skills\"\n\n"));

    config.push_str("[a2a]\n");
    config.push_str("enabled = true\n");

    // Write config
    let config_path = "ironclad.toml";
    if std::path::Path::new(config_path).exists() {
        let overwrite = Confirm::new()
            .with_prompt("  ironclad.toml already exists. Overwrite?")
            .default(false)
            .interact()?;
        if !overwrite {
            println!("\n  Aborted. Existing config preserved.\n");
            return Ok(());
        }
    }

    std::fs::write(config_path, &config)?;
    println!("\n  \u{2705} Configuration written to {config_path}");

    // Create workspace dir
    let ws_path = std::path::Path::new(&workspace);
    if !ws_path.exists() {
        std::fs::create_dir_all(ws_path)?;
        println!("  \u{2705} Created workspace: {workspace}");
    }

    // Create skills dir
    let skills_path = format!("{home}/.ironclad/skills");
    let sp = std::path::Path::new(&skills_path);
    if !sp.exists() {
        std::fs::create_dir_all(sp)?;
        println!("  \u{2705} Created skills directory");
    }

    // Personality setup
    println!("\n  {BOLD}Personality Setup{RESET}\n");
    let personality_options = vec![
        "Keep Roboticus (recommended default)",
        "Quick setup (5 questions)",
        "Full interview (guided conversation with your agent)",
    ];
    let personality_idx = Select::new()
        .with_prompt("  How would you like to configure your agent's personality?")
        .items(&personality_options)
        .default(0)
        .interact()?;

    match personality_idx {
        0 => {
            ironclad_core::personality::write_defaults(ws_path)?;
            println!("  \u{2705} Roboticus personality loaded (OS.toml + FIRMWARE.toml)");
        }
        1 => {
            run_quick_personality_setup(ws_path)?;
        }
        2 => {
            let basic_name: String = Input::new()
                .with_prompt("  Agent name")
                .default(agent_name.clone())
                .interact_text()?;
            let domains = vec!["general", "developer", "business", "creative", "research"];
            let domain_idx = Select::new()
                .with_prompt("  Primary domain")
                .items(&domains)
                .default(0)
                .interact()?;

            // Write a starter OS.toml with basics; the full interview will overwrite
            let starter_os = ironclad_core::personality::generate_os_toml(
                &basic_name, "balanced", "suggest", domains[domain_idx],
            );
            std::fs::write(ws_path.join("OS.toml"), &starter_os)?;
            ironclad_core::personality::write_defaults(ws_path)
                .ok(); // ensures FIRMWARE.toml exists

            println!();
            println!("  \u{2705} Starter personality written.");
            println!("  \u{25b8} Start your agent:  {BOLD}ironclad serve{RESET}");
            println!("  \u{25b8} Then send it:      {BOLD}/interview{RESET}");
            println!("  \u{25b8} The agent will walk you through a deep personality interview.");
        }
        _ => {}
    }

    println!();
    println!("  \u{2705} Setup complete! Run {BOLD}ironclad serve{RESET} to start.");
    println!();

    Ok(())
}

fn run_quick_personality_setup(workspace: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    use dialoguer::{Input, Select};

    let name: String = Input::new()
        .with_prompt("  Agent name")
        .default("Roboticus".into())
        .interact_text()?;

    let formality_options = vec!["formal", "balanced", "casual"];
    let formality_idx = Select::new()
        .with_prompt("  Communication style")
        .items(&formality_options)
        .default(1)
        .interact()?;

    let proactive_options = vec![
        "wait (only act when told)",
        "suggest (flag opportunities, ask first)",
        "initiative (act proactively)",
    ];
    let proactive_idx = Select::new()
        .with_prompt("  Proactiveness level")
        .items(&proactive_options)
        .default(1)
        .interact()?;
    let proactive_val = match proactive_idx {
        0 => "wait",
        2 => "initiative",
        _ => "suggest",
    };

    let domain_options = vec!["general", "developer", "business", "creative", "research"];
    let domain_idx = Select::new()
        .with_prompt("  Primary domain")
        .items(&domain_options)
        .default(0)
        .interact()?;

    let boundaries: String = Input::new()
        .with_prompt("  Any hard boundaries? (topics/actions that are off-limits, or press Enter to skip)")
        .allow_empty(true)
        .interact_text()?;

    let os_toml = ironclad_core::personality::generate_os_toml(
        &name,
        formality_options[formality_idx],
        proactive_val,
        domain_options[domain_idx],
    );
    let fw_toml = ironclad_core::personality::generate_firmware_toml(&boundaries);

    std::fs::create_dir_all(workspace)?;
    std::fs::write(workspace.join("OS.toml"), &os_toml)?;
    std::fs::write(workspace.join("FIRMWARE.toml"), &fw_toml)?;

    println!("  \u{2705} Personality configured for {BOLD}{name}{RESET} (OS.toml + FIRMWARE.toml)");

    Ok(())
}

pub async fn cmd_logs(
    base_url: &str,
    lines: usize,
    follow: bool,
    level: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    if follow {
        println!("  {BOLD}Tailing logs{RESET} (level >= {level}, Ctrl+C to stop)\n");

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{base_url}/api/logs"))
            .query(&[("follow", "true"), ("level", level), ("lines", &lines.to_string())])
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
                        if line.starts_with("data: ") {
                            let data = &line["data: ".len()..];
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
        let resp = reqwest::get(format!(
            "{base_url}/api/logs?lines={lines}&level={level}"
        ))
        .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().await.unwrap_or_default();
                if let Some(entries) = body.get("entries").and_then(|v| v.as_array()) {
                    if entries.is_empty() {
                        println!("  No log entries found.");
                    }
                    for entry in entries {
                        let ts = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                        let lvl = entry.get("level").and_then(|v| v.as_str()).unwrap_or("info");
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

fn try_read_log_file(lines: usize, _level: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let log_dir = std::path::PathBuf::from(&home).join(".ironclad").join("logs");

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
                let start = if all_lines.len() > lines { all_lines.len() - lines } else { 0 };
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

pub async fn cmd_plugin_info(base_url: &str, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let resp = reqwest::get(format!("{base_url}/api/plugins")).await?;
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let plugins: Vec<serde_json::Value> = body
        .get("plugins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let plugin = plugins.iter().find(|p| {
        p.get("name").and_then(|v| v.as_str()) == Some(name)
    });

    match plugin {
        Some(p) => {
            println!("\n  {BOLD}Plugin: {name}{RESET}\n");
            if let Some(v) = p.get("version").and_then(|v| v.as_str()) {
                println!("  Version:     {v}");
            }
            if let Some(d) = p.get("description").and_then(|v| v.as_str()) {
                println!("  Description: {d}");
            }
            let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            println!("  Status:      {}", if enabled { format!("{GREEN}enabled{RESET}") } else { format!("{RED}disabled{RESET}") });
            if let Some(path) = p.get("manifest_path").and_then(|v| v.as_str()) {
                println!("  Manifest:    {path}");
            }
            if let Some(tools) = p.get("tools").and_then(|v| v.as_array()) {
                println!("  Tools:       {}", tools.len());
                for tool in tools {
                    if let Some(tn) = tool.get("name").and_then(|v| v.as_str()) {
                        println!("    - {tn}");
                    }
                }
            }
            println!();
        }
        None => {
            eprintln!("  Plugin not found: {name}");
        }
    }
    Ok(())
}

pub fn cmd_plugin_install(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let source_path = std::path::Path::new(source);
    if !source_path.exists() {
        eprintln!("  Source not found: {source}");
        return Ok(());
    }

    let manifest_path = source_path.join("plugin.toml");
    if !manifest_path.exists() {
        eprintln!("  No plugin.toml found in {source}");
        return Ok(());
    }

    let manifest_content = std::fs::read_to_string(&manifest_path)?;
    let manifest: toml::Value = manifest_content.parse()?;
    let plugin_name = manifest.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let plugins_dir = std::path::PathBuf::from(&home).join(".ironclad").join("plugins");
    let dest = plugins_dir.join(plugin_name);

    if dest.exists() {
        eprintln!("  Plugin already installed: {plugin_name}");
        eprintln!("  Uninstall first with: ironclad plugins uninstall {plugin_name}");
        return Ok(());
    }

    std::fs::create_dir_all(&dest)?;
    copy_dir_recursive(source_path, &dest)?;

    println!("  \u{2705} Installed plugin: {plugin_name}");
    println!("  Location: {}", dest.display());
    println!("  Restart the server to activate.\n");
    Ok(())
}

pub fn cmd_plugin_uninstall(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let plugin_dir = std::path::PathBuf::from(&home)
        .join(".ironclad")
        .join("plugins")
        .join(name);

    if !plugin_dir.exists() {
        eprintln!("  Plugin not found: {name}");
        return Ok(());
    }

    std::fs::remove_dir_all(&plugin_dir)?;
    println!("  \u{2705} Uninstalled plugin: {name}");
    println!("  Restart the server to apply.\n");
    Ok(())
}

pub async fn cmd_plugin_toggle(
    base_url: &str,
    name: &str,
    enable: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let action = if enable { "enable" } else { "disable" };
    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base_url}/api/plugins/{name}/toggle"))
        .json(&serde_json::json!({ "enabled": enable }))
        .send()
        .await?;

    if resp.status().is_success() {
        println!("  \u{2705} Plugin {name} {action}d");
    } else {
        eprintln!("  Failed to {action} plugin {name}: {}", resp.status());
    }
    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

pub fn cmd_security_audit(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    println!("\n  {BOLD}Ironclad Security Audit{RESET}\n");

    let mut pass_count = 0u32;
    let mut warn_count = 0u32;
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
                println!("  {RED}\u{26d3} FAIL{RESET} Config file is world/group-readable (mode {:o})", mode & 0o777);
                println!("         Fix: chmod 600 {config_path}");
                fail_count += 1;
            } else {
                println!("  \u{2705} Config file permissions (mode {:o})", mode & 0o777);
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  \u{26a0}\u{fe0f} Config file permission check (non-Unix)");
            warn_count += 1;
        }
    } else {
        println!("  \u{26a0}\u{fe0f} Config file not found: {config_path}");
        warn_count += 1;
    }

    // 2. Check for API keys in config
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        let has_plaintext_key = content.contains("api_key") && !content.contains("${") && !content.contains("env(");
        if has_plaintext_key {
            println!("  \u{26a0}\u{fe0f} Plaintext API keys found in config");
            println!("         Recommendation: Use environment variables instead");
            warn_count += 1;
        } else {
            println!("  \u{2705} No plaintext API keys in config");
            pass_count += 1;
        }
    }

    // 3. Check bind address
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("bind = \"0.0.0.0\"") {
            println!("  \u{26a0}\u{fe0f} Server bound to 0.0.0.0 (all interfaces)");
            println!("         Recommendation: Bind to 127.0.0.1 unless external access is needed");
            warn_count += 1;
        } else {
            println!("  \u{2705} Server not bound to all interfaces");
            pass_count += 1;
        }
    }

    // 4. Check wallet file permissions
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let wallet_path = std::path::PathBuf::from(&home).join(".ironclad").join("wallet.json");
    if wallet_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&wallet_path)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                println!("  {RED}\u{26d3} FAIL{RESET} Wallet file is world/group-readable (mode {:o})", mode & 0o777);
                println!("         Fix: chmod 600 {}", wallet_path.display());
                fail_count += 1;
            } else {
                println!("  \u{2705} Wallet file permissions (mode {:o})", mode & 0o777);
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  \u{26a0}\u{fe0f} Wallet permission check (non-Unix)");
            warn_count += 1;
        }
    } else {
        println!("  {DIM}  \u{2500}{RESET} No wallet file found (OK if not using wallet features)");
    }

    // 5. Check database file permissions
    let db_path = std::path::PathBuf::from(&home).join(".ironclad").join("state.db");
    if db_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&db_path)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                println!("  \u{26a0}\u{fe0f} Database is world/group-readable (mode {:o})", mode & 0o777);
                println!("         Fix: chmod 600 {}", db_path.display());
                warn_count += 1;
            } else {
                println!("  \u{2705} Database file permissions");
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  \u{26a0}\u{fe0f} Database permission check (non-Unix)");
            warn_count += 1;
        }
    }

    // 6. Check CORS settings
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("cors") && content.contains("\"*\"") {
            println!("  \u{26a0}\u{fe0f} CORS allows all origins (\"*\")");
            println!("         Recommendation: Restrict CORS to specific origins in production");
            warn_count += 1;
        } else {
            println!("  \u{2705} CORS configuration");
            pass_count += 1;
        }
    }

    // 7. Check PID file
    let pid_path = std::path::PathBuf::from(&home).join(".ironclad").join("ironclad.pid");
    if pid_path.exists() {
        println!("  \u{2705} PID file exists");
        pass_count += 1;
    }

    // Summary
    println!();
    let total = pass_count + warn_count + fail_count;
    if fail_count > 0 {
        println!("  {RED}\u{26d3}{RESET} {fail_count} failure(s), {warn_count} warning(s), {pass_count} passed out of {total} checks");
    } else if warn_count > 0 {
        println!("  \u{26a0}\u{fe0f} {warn_count} warning(s), {pass_count} passed out of {total} checks");
    } else {
        println!("  \u{2705} All {total} checks passed");
    }
    println!();

    Ok(())
}

fn which_binary(name: &str) -> Option<String> {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|dir| std::path::PathBuf::from(dir).join(name))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_construction() {
        let c = IroncladClient::new("http://localhost:18789");
        assert_eq!(c.base_url, "http://localhost:18789");
    }

    #[test]
    fn client_strips_trailing_slash() {
        let c = IroncladClient::new("http://localhost:18789/");
        assert_eq!(c.base_url, "http://localhost:18789");
    }

    #[test]
    fn truncate_id_short() {
        assert_eq!(truncate_id("abc", 10), "abc");
    }

    #[test]
    fn truncate_id_long() {
        assert_eq!(truncate_id("abcdefghijklmnop", 8), "abcdefgh...");
    }

    #[test]
    fn status_badges() {
        let ok = status_badge("ok");
        assert!(ok.contains("ok"));
        let dead = status_badge("dead");
        assert!(dead.contains("dead"));
        let unknown = status_badge("foo");
        assert!(unknown.contains("foo"));
    }

    #[test]
    fn strip_ansi_len_works() {
        assert_eq!(strip_ansi_len("hello"), 5);
        assert_eq!(strip_ansi_len("\x1b[32mhello\x1b[0m"), 5);
        assert_eq!(strip_ansi_len("\x1b[38;5;105mtest\x1b[0m"), 4);
    }

    #[test]
    fn urlencoding_encodes() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("a&b=c#d"), "a%26b%3Dc%23d");
    }

    #[test]
    fn format_json_val_types() {
        let s = format_json_val(&Value::String("test".into()));
        assert!(s.contains("test"));
        let n = format_json_val(&serde_json::json!(42));
        assert!(n.contains("42"));
        let b = format_json_val(&serde_json::json!(true));
        assert!(b.contains("true"));
        let null = format_json_val(&Value::Null);
        assert!(null.contains("null"));
    }

    #[test]
    fn which_binary_finds_sh() {
        let result = which_binary("sh");
        assert!(result.is_some(), "sh should be findable on any Unix system");
    }

    #[test]
    fn which_binary_returns_none_for_nonsense() {
        let result = which_binary("__ironclad_nonexistent_binary_98765__");
        assert!(result.is_none());
    }
}
