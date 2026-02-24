#![allow(non_snake_case, unused_variables)]

use std::sync::OnceLock;

use reqwest::Client;
use serde_json::Value;

use ironclad_core::style::Theme;

pub(crate) const CRT_DRAW_MS: u64 = 4;

#[macro_export]
macro_rules! println {
    () => {{ use std::io::Write; std::io::stdout().write_all(b"\n").ok(); std::io::stdout().flush().ok(); }};
    ($($arg:tt)*) => {{ let __text = format!($($arg)*); theme().typewrite_line_stdout(&__text, CRT_DRAW_MS); }};
}

#[macro_export]
macro_rules! eprintln {
    () => {{ use std::io::Write; std::io::stderr().write_all(b"\n").ok(); }};
    ($($arg:tt)*) => {{ let __text = format!($($arg)*); theme().typewrite_line(&__text, CRT_DRAW_MS); }};
}

static THEME: OnceLock<Theme> = OnceLock::new();

pub fn init_theme(color_flag: &str, theme_flag: &str, no_draw: bool, nerdmode: bool) {
    let t = Theme::from_flags(color_flag, theme_flag);
    let t = if nerdmode {
        t.with_nerdmode(true)
    } else if no_draw {
        t.with_draw(false)
    } else {
        t
    };
    let _ = THEME.set(t);
}

pub fn theme() -> &'static Theme {
    THEME.get_or_init(Theme::detect)
}

#[allow(clippy::type_complexity)]
pub(crate) fn colors() -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    let t = theme();
    (
        t.dim(),
        t.bold(),
        t.accent(),
        t.success(),
        t.warn(),
        t.error(),
        t.info(),
        t.reset(),
        t.mono(),
    )
}

pub(crate) fn icons() -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    let t = theme();
    (
        t.icon_ok(),
        t.icon_action(),
        t.icon_warn(),
        t.icon_detail(),
        t.icon_error(),
    )
}

pub struct IroncladClient {
    client: Client,
    base_url: String,
}

impl IroncladClient {
    pub fn new(base_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
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
    async fn post(&self, path: &str, body: Value) -> Result<Value, Box<dyn std::error::Error>> {
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
        let (OK, ACTION, WARN, DETAIL, ERR) = icons();
        let msg = format!("{e:?}");
        if msg.contains("Connection refused")
            || msg.contains("ConnectionRefused")
            || msg.contains("ConnectError")
            || msg.contains("connect error")
        {
            eprintln!();
            eprintln!(
                "  {WARN} Is the Ironclad server running? Start it with: {BOLD}ironclad serve{RESET}"
            );
        }
    }
}

pub(crate) fn heading(text: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    eprintln!();
    eprintln!("  {OK} {BOLD}{text}{RESET}");
    eprintln!("  {DIM}{}{RESET}", "\u{2500}".repeat(60));
}

pub(crate) fn kv(key: &str, value: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    eprintln!("    {DIM}{key:<20}{RESET} {value}");
}

pub(crate) fn kv_accent(key: &str, value: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    eprintln!("    {DIM}{key:<20}{RESET} {ACCENT}{value}{RESET}");
}

pub(crate) fn kv_mono(key: &str, value: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    eprintln!("    {DIM}{key:<20}{RESET} {MONO}{value}{RESET}");
}

pub(crate) fn badge(text: &str, color: &str) -> String {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    format!("{color}\u{25cf} {text}{RESET}")
}

pub(crate) fn status_badge(status: &str) -> String {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    match status {
        "ok" | "running" | "success" => badge(status, GREEN),
        "sleeping" | "pending" | "warning" => badge(status, YELLOW),
        "dead" | "error" | "failed" => badge(status, RED),
        _ => badge(status, DIM),
    }
}

pub(crate) fn truncate_id(id: &str, len: usize) -> String {
    if id.len() > len {
        format!("{}...", &id[..len])
    } else {
        id.to_string()
    }
}

pub(crate) fn table_separator(widths: &[usize]) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let parts: Vec<String> = widths.iter().map(|w| "\u{2500}".repeat(*w)).collect();
    eprintln!("    {DIM}\u{251c}{}\u{2524}{RESET}", parts.join("\u{253c}"));
}

pub(crate) fn table_header(headers: &[&str], widths: &[usize]) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let cells: Vec<String> = headers
        .iter()
        .zip(widths)
        .map(|(h, w)| format!("{BOLD}{h:<width$}{RESET}", width = w))
        .collect();
    eprintln!(
        "    {DIM}\u{2502}{RESET}{}{DIM}\u{2502}{RESET}",
        cells.join(&format!("{DIM}\u{2502}{RESET}"))
    );
    table_separator(widths);
}

pub(crate) fn table_row(cells: &[String], widths: &[usize]) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
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
    eprintln!(
        "    {DIM}\u{2502}{RESET}{}{DIM}\u{2502}{RESET}",
        formatted.join(&format!("{DIM}\u{2502}{RESET}"))
    );
}

pub(crate) fn strip_ansi_len(s: &str) -> usize {
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

pub(crate) fn empty_state(msg: &str) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    eprintln!("    {DIM}\u{2500}\u{2500} {msg}{RESET}");
}

pub(crate) fn print_json_section(val: &Value, indent: usize) {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
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
                        let items: Vec<String> =
                            arr.iter().map(|i| format_json_val(i).to_string()).collect();
                        eprintln!(
                            "{pad}{DIM}{k:<22}{RESET} [{MONO}{}{RESET}]",
                            items.join(", ")
                        );
                    }
                    _ => eprintln!("{pad}{DIM}{k:<22}{RESET} {}", format_json_val(v)),
                }
            }
        }
        _ => eprintln!("{pad}{}", format_json_val(val)),
    }
}

pub(crate) fn format_json_val(v: &Value) -> String {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
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

pub(crate) fn urlencoding(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('#', "%23")
}

pub(crate) fn which_binary(name: &str) -> Option<String> {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|dir| std::path::PathBuf::from(dir).join(name))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

mod admin;
mod memory;
mod schedule;
mod sessions;
mod status;
mod update;
mod wallet;

pub use admin::*;
pub use memory::*;
pub use schedule::*;
pub use sessions::*;
pub use status::*;
pub use update::*;
pub use wallet::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_construction() {
        let c = IroncladClient::new("http://localhost:18789").unwrap();
        assert_eq!(c.base_url, "http://localhost:18789");
    }
    #[test]
    fn client_strips_trailing_slash() {
        let c = IroncladClient::new("http://localhost:18789/").unwrap();
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
        assert!(status_badge("ok").contains("ok"));
        assert!(status_badge("dead").contains("dead"));
        assert!(status_badge("foo").contains("foo"));
    }
    #[test]
    fn strip_ansi_len_works() {
        assert_eq!(strip_ansi_len("hello"), 5);
        assert_eq!(strip_ansi_len("\x1b[32mhello\x1b[0m"), 5);
    }
    #[test]
    fn urlencoding_encodes() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("a&b=c#d"), "a%26b%3Dc%23d");
    }
    #[test]
    fn format_json_val_types() {
        assert!(format_json_val(&Value::String("test".into())).contains("test"));
        assert!(format_json_val(&serde_json::json!(42)).contains("42"));
        assert!(format_json_val(&Value::Null).contains("null"));
    }
    #[test]
    fn which_binary_finds_sh() {
        assert!(which_binary("sh").is_some());
    }
    #[test]
    fn which_binary_returns_none_for_nonsense() {
        assert!(which_binary("__ironclad_nonexistent_binary_98765__").is_none());
    }

    #[test]
    fn format_json_val_bool_true() {
        let result = format_json_val(&serde_json::json!(true));
        assert!(result.contains("true"));
    }

    #[test]
    fn format_json_val_bool_false() {
        let result = format_json_val(&serde_json::json!(false));
        assert!(result.contains("false"));
    }

    #[test]
    fn format_json_val_array_uses_to_string() {
        let result = format_json_val(&serde_json::json!([1, 2, 3]));
        assert!(result.contains("1"));
    }

    #[test]
    fn strip_ansi_len_empty() {
        assert_eq!(strip_ansi_len(""), 0);
    }

    #[test]
    fn strip_ansi_len_only_ansi() {
        assert_eq!(strip_ansi_len("\x1b[32m\x1b[0m"), 0);
    }

    #[test]
    fn status_badge_sleeping() {
        assert!(status_badge("sleeping").contains("sleeping"));
    }

    #[test]
    fn status_badge_pending() {
        assert!(status_badge("pending").contains("pending"));
    }

    #[test]
    fn status_badge_running() {
        assert!(status_badge("running").contains("running"));
    }

    #[test]
    fn badge_contains_text_and_bullet() {
        let b = badge("running", "\x1b[32m");
        assert!(b.contains("running"));
        assert!(b.contains("\u{25cf}"));
    }

    #[test]
    fn truncate_id_exact_length() {
        assert_eq!(truncate_id("abc", 3), "abc");
    }

    // ── wiremock-based CLI command tests ─────────────────────────

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn mock_get(server: &MockServer, p: &str, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    async fn mock_post(server: &MockServer, p: &str, body: serde_json::Value) {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    // ── Skills ────────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_skills_list_with_skills() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/skills", serde_json::json!({
            "skills": [
                {"name": "greet", "kind": "builtin", "description": "Says hello", "enabled": true},
                {"name": "calc", "kind": "gosh", "description": "Math stuff", "enabled": false}
            ]
        })).await;
        super::cmd_skills_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_skills_list_empty() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/skills", serde_json::json!({"skills": []})).await;
        super::cmd_skills_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_skills_list_null_skills() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/skills", serde_json::json!({})).await;
        super::cmd_skills_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_skill_detail_enabled_with_triggers() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/skills/greet",
            serde_json::json!({
                "id": "greet-001", "name": "greet", "kind": "builtin",
                "description": "Says hello", "source_path": "/skills/greet.gosh",
                "content_hash": "abc123", "enabled": true,
                "triggers_json": "[\"on_start\"]", "script_path": "/scripts/greet.gosh"
            }),
        )
        .await;
        super::cmd_skill_detail(&s.uri(), "greet").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_skill_detail_disabled_no_triggers() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/skills/calc",
            serde_json::json!({
                "id": "calc-001", "name": "calc", "kind": "gosh",
                "description": "Math", "source_path": "", "content_hash": "",
                "enabled": false, "triggers_json": "null", "script_path": "null"
            }),
        )
        .await;
        super::cmd_skill_detail(&s.uri(), "calc").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_skill_detail_enabled_as_int() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/skills/x",
            serde_json::json!({
                "id": "x", "name": "x", "kind": "builtin",
                "description": "", "source_path": "", "content_hash": "",
                "enabled": 1
            }),
        )
        .await;
        super::cmd_skill_detail(&s.uri(), "x").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_skills_reload_ok() {
        let s = MockServer::start().await;
        mock_post(&s, "/api/skills/reload", serde_json::json!({"ok": true})).await;
        super::cmd_skills_reload(&s.uri()).await.unwrap();
    }

    // ── Wallet ────────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_wallet_full() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/wallet/balance",
            serde_json::json!({
                "balance": "42.50", "currency": "USDC", "note": "Testnet balance"
            }),
        )
        .await;
        mock_get(
            &s,
            "/api/wallet/address",
            serde_json::json!({
                "address": "0xdeadbeef"
            }),
        )
        .await;
        super::cmd_wallet(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_wallet_no_note() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/wallet/balance",
            serde_json::json!({
                "balance": "0.00", "currency": "USDC"
            }),
        )
        .await;
        mock_get(
            &s,
            "/api/wallet/address",
            serde_json::json!({
                "address": "0xabc"
            }),
        )
        .await;
        super::cmd_wallet(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_wallet_address_ok() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/wallet/address",
            serde_json::json!({
                "address": "0x1234"
            }),
        )
        .await;
        super::cmd_wallet_address(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_wallet_balance_ok() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/wallet/balance",
            serde_json::json!({
                "balance": "100.00", "currency": "ETH"
            }),
        )
        .await;
        super::cmd_wallet_balance(&s.uri()).await.unwrap();
    }

    // ── Schedule ──────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_schedule_list_with_jobs() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/cron/jobs",
            serde_json::json!({
                "jobs": [
                    {
                        "name": "backup", "schedule_kind": "cron", "schedule_expr": "0 * * * *",
                        "last_run_at": "2025-01-01T12:00:00.000Z", "last_status": "ok",
                        "consecutive_errors": 0
                    },
                    {
                        "name": "cleanup", "schedule_kind": "interval", "schedule_expr": "30m",
                        "last_run_at": null, "last_status": "pending",
                        "consecutive_errors": 3
                    }
                ]
            }),
        )
        .await;
        super::cmd_schedule_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_schedule_list_empty() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/cron/jobs", serde_json::json!({"jobs": []})).await;
        super::cmd_schedule_list(&s.uri()).await.unwrap();
    }

    // ── Memory ────────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_memory_working_with_entries() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/memory/working/sess-1", serde_json::json!({
            "entries": [
                {"id": "e1", "entry_type": "fact", "content": "The sky is blue", "importance": 5}
            ]
        })).await;
        super::cmd_memory(&s.uri(), "working", Some("sess-1"), None, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_working_empty() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/memory/working/sess-2",
            serde_json::json!({"entries": []}),
        )
        .await;
        super::cmd_memory(&s.uri(), "working", Some("sess-2"), None, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_working_no_session_errors() {
        let result = super::cmd_memory("http://unused", "working", None, None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cmd_memory_episodic_with_entries() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/memory/episodic"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entries": [
                    {"id": "ep1", "classification": "conversation", "content": "User asked about weather", "importance": 3}
                ]
            })))
            .mount(&s)
            .await;
        super::cmd_memory(&s.uri(), "episodic", None, None, Some(10))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_episodic_empty() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/memory/episodic"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"entries": []})),
            )
            .mount(&s)
            .await;
        super::cmd_memory(&s.uri(), "episodic", None, None, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_semantic_with_entries() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/memory/semantic/general",
            serde_json::json!({
                "entries": [
                    {"key": "favorite_color", "value": "blue", "confidence": 0.95}
                ]
            }),
        )
        .await;
        super::cmd_memory(&s.uri(), "semantic", None, None, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_semantic_custom_category() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/memory/semantic/prefs",
            serde_json::json!({
                "entries": []
            }),
        )
        .await;
        super::cmd_memory(&s.uri(), "semantic", Some("prefs"), None, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_search_with_results() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/memory/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": ["result one", "result two"]
            })))
            .mount(&s)
            .await;
        super::cmd_memory(&s.uri(), "search", None, Some("hello"), None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_search_empty() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/memory/search"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": []})),
            )
            .mount(&s)
            .await;
        super::cmd_memory(&s.uri(), "search", None, Some("nope"), None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_memory_search_no_query_errors() {
        let result = super::cmd_memory("http://unused", "search", None, None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cmd_memory_unknown_tier_errors() {
        let result = super::cmd_memory("http://unused", "bogus", None, None, None).await;
        assert!(result.is_err());
    }

    // ── Sessions ──────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_sessions_list_with_sessions() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/sessions", serde_json::json!({
            "sessions": [
                {"id": "s-001", "agent_id": "ironclad", "created_at": "2025-01-01T00:00:00Z", "updated_at": "2025-01-01T01:00:00Z"},
                {"id": "s-002", "agent_id": "duncan", "created_at": "2025-01-02T00:00:00Z", "updated_at": "2025-01-02T01:00:00Z"}
            ]
        })).await;
        super::cmd_sessions_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_sessions_list_empty() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/sessions", serde_json::json!({"sessions": []})).await;
        super::cmd_sessions_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_session_detail_with_messages() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/sessions/s-001",
            serde_json::json!({
                "id": "s-001", "agent_id": "ironclad",
                "created_at": "2025-01-01T00:00:00Z", "updated_at": "2025-01-01T01:00:00Z"
            }),
        )
        .await;
        mock_get(&s, "/api/sessions/s-001/messages", serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello!", "created_at": "2025-01-01T00:00:05.123Z"},
                {"role": "assistant", "content": "Hi there!", "created_at": "2025-01-01T00:00:06.456Z"},
                {"role": "system", "content": "Init", "created_at": "2025-01-01T00:00:00Z"},
                {"role": "tool", "content": "Result", "created_at": "2025-01-01T00:00:07Z"}
            ]
        })).await;
        super::cmd_session_detail(&s.uri(), "s-001").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_session_detail_no_messages() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/sessions/s-002",
            serde_json::json!({
                "id": "s-002", "agent_id": "ironclad",
                "created_at": "2025-01-01T00:00:00Z", "updated_at": "2025-01-01T01:00:00Z"
            }),
        )
        .await;
        mock_get(
            &s,
            "/api/sessions/s-002/messages",
            serde_json::json!({"messages": []}),
        )
        .await;
        super::cmd_session_detail(&s.uri(), "s-002").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_session_create_ok() {
        let s = MockServer::start().await;
        mock_post(
            &s,
            "/api/sessions",
            serde_json::json!({"session_id": "new-001"}),
        )
        .await;
        super::cmd_session_create(&s.uri(), "ironclad")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_session_export_json() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/sessions/s-001",
            serde_json::json!({
                "id": "s-001", "agent_id": "ironclad", "created_at": "2025-01-01T00:00:00Z"
            }),
        )
        .await;
        mock_get(&s, "/api/sessions/s-001/messages", serde_json::json!({
            "messages": [{"role": "user", "content": "Hi", "created_at": "2025-01-01T00:00:01Z"}]
        })).await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("export.json");
        super::cmd_session_export(&s.uri(), "s-001", "json", Some(out.to_str().unwrap()))
            .await
            .unwrap();
        assert!(out.exists());
        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("s-001"));
    }

    #[tokio::test]
    async fn cmd_session_export_markdown() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/sessions/s-001",
            serde_json::json!({
                "id": "s-001", "agent_id": "ironclad", "created_at": "2025-01-01T00:00:00Z"
            }),
        )
        .await;
        mock_get(&s, "/api/sessions/s-001/messages", serde_json::json!({
            "messages": [{"role": "user", "content": "Hi", "created_at": "2025-01-01T00:00:01Z"}]
        })).await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("export.md");
        super::cmd_session_export(&s.uri(), "s-001", "markdown", Some(out.to_str().unwrap()))
            .await
            .unwrap();
        assert!(out.exists());
        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("# Session"));
    }

    #[tokio::test]
    async fn cmd_session_export_html() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/sessions/s-001",
            serde_json::json!({
                "id": "s-001", "agent_id": "ironclad", "created_at": "2025-01-01T00:00:00Z"
            }),
        )
        .await;
        mock_get(&s, "/api/sessions/s-001/messages", serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello <world> & \"friends\"", "created_at": "2025-01-01T00:00:01Z"},
                {"role": "assistant", "content": "Hi", "created_at": "2025-01-01T00:00:02Z"},
                {"role": "system", "content": "Sys", "created_at": "2025-01-01T00:00:00Z"},
                {"role": "tool", "content": "Tool output", "created_at": "2025-01-01T00:00:03Z"}
            ]
        })).await;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("export.html");
        super::cmd_session_export(&s.uri(), "s-001", "html", Some(out.to_str().unwrap()))
            .await
            .unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("<!DOCTYPE html>"));
        assert!(content.contains("&amp;"));
        assert!(content.contains("&lt;"));
    }

    #[tokio::test]
    async fn cmd_session_export_to_stdout() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/sessions/s-001",
            serde_json::json!({
                "id": "s-001", "agent_id": "ironclad", "created_at": "2025-01-01T00:00:00Z"
            }),
        )
        .await;
        mock_get(
            &s,
            "/api/sessions/s-001/messages",
            serde_json::json!({"messages": []}),
        )
        .await;
        super::cmd_session_export(&s.uri(), "s-001", "json", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_session_export_unknown_format() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/sessions/s-001",
            serde_json::json!({
                "id": "s-001", "agent_id": "ironclad", "created_at": "2025-01-01T00:00:00Z"
            }),
        )
        .await;
        mock_get(
            &s,
            "/api/sessions/s-001/messages",
            serde_json::json!({"messages": []}),
        )
        .await;
        super::cmd_session_export(&s.uri(), "s-001", "csv", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_session_export_not_found() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/sessions/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s)
            .await;
        super::cmd_session_export(&s.uri(), "missing", "json", None)
            .await
            .unwrap();
    }

    // ── Circuit breaker ───────────────────────────────────────

    #[tokio::test]
    async fn cmd_circuit_status_with_providers() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/breaker/status",
            serde_json::json!({
                "providers": {
                    "ollama": {"state": "closed"},
                    "openai": {"state": "open"}
                },
                "note": "All good"
            }),
        )
        .await;
        super::cmd_circuit_status(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_circuit_status_empty_providers() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/breaker/status",
            serde_json::json!({"providers": {}}),
        )
        .await;
        super::cmd_circuit_status(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_circuit_status_no_providers_key() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/breaker/status", serde_json::json!({})).await;
        super::cmd_circuit_status(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_circuit_reset_success() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/breaker/status",
            serde_json::json!({
                "providers": {
                    "ollama": {"state": "open"},
                    "moonshot": {"state": "open"}
                }
            }),
        )
        .await;
        mock_post(
            &s,
            "/api/breaker/reset/ollama",
            serde_json::json!({"ok": true}),
        )
        .await;
        mock_post(
            &s,
            "/api/breaker/reset/moonshot",
            serde_json::json!({"ok": true}),
        )
        .await;
        super::cmd_circuit_reset(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_circuit_reset_server_error() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/breaker/status"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&s)
            .await;
        super::cmd_circuit_reset(&s.uri()).await.unwrap();
    }

    // ── Agents ────────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_agents_list_with_agents() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/agents",
            serde_json::json!({
                "agents": [
                    {"id": "ironclad", "name": "Ironclad", "state": "running", "model": "qwen3:8b"},
                    {"id": "duncan", "name": "Duncan", "state": "sleeping", "model": "gpt-4o"}
                ]
            }),
        )
        .await;
        super::cmd_agents_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_agents_list_empty() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/agents", serde_json::json!({"agents": []})).await;
        super::cmd_agents_list(&s.uri()).await.unwrap();
    }

    // ── Channels ──────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_channels_status_with_channels() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/channels/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "telegram", "connected": true, "messages_received": 100, "messages_sent": 50},
                {"name": "whatsapp", "connected": false, "messages_received": 0, "messages_sent": 0}
            ])))
            .mount(&s)
            .await;
        super::cmd_channels_status(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_channels_status_empty() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/channels/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&s)
            .await;
        super::cmd_channels_status(&s.uri()).await.unwrap();
    }

    // ── Plugins ───────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_plugins_list_with_plugins() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/plugins", serde_json::json!({
            "plugins": [
                {"name": "weather", "version": "1.0", "status": "active", "tools": [{"name": "get_weather"}]},
                {"name": "empty", "version": "0.1", "status": "inactive", "tools": []}
            ]
        })).await;
        super::cmd_plugins_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_plugins_list_empty() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/plugins", serde_json::json!({"plugins": []})).await;
        super::cmd_plugins_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_plugin_info_found() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/plugins",
            serde_json::json!({
                "plugins": [
                    {
                        "name": "weather", "version": "1.0", "description": "Weather plugin",
                        "enabled": true, "manifest_path": "/plugins/weather/plugin.toml",
                        "tools": [{"name": "get_weather"}, {"name": "get_forecast"}]
                    }
                ]
            }),
        )
        .await;
        super::cmd_plugin_info(&s.uri(), "weather").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_plugin_info_disabled() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/plugins",
            serde_json::json!({
                "plugins": [{"name": "old", "version": "0.1", "enabled": false}]
            }),
        )
        .await;
        super::cmd_plugin_info(&s.uri(), "old").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_plugin_info_not_found() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/plugins", serde_json::json!({"plugins": []})).await;
        super::cmd_plugin_info(&s.uri(), "nonexistent")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_plugin_toggle_enable() {
        let s = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/plugins/weather/toggle"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&s)
            .await;
        super::cmd_plugin_toggle(&s.uri(), "weather", true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_plugin_toggle_disable_fails() {
        let s = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/plugins/weather/toggle"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s)
            .await;
        super::cmd_plugin_toggle(&s.uri(), "weather", false)
            .await
            .unwrap();
    }

    #[test]
    fn cmd_plugin_install_missing_source() {
        super::cmd_plugin_install("/tmp/ironclad_test_nonexistent_plugin_dir").unwrap();
    }

    #[test]
    fn cmd_plugin_install_no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        super::cmd_plugin_install(dir.path().to_str().unwrap()).unwrap();
    }

    #[test]
    fn cmd_plugin_install_valid() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("plugin.toml");
        std::fs::write(&manifest, "name = \"test-plugin\"\nversion = \"0.1\"").unwrap();
        std::fs::write(dir.path().join("main.gosh"), "print(\"hi\")").unwrap();

        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("helper.gosh"), "// helper").unwrap();

        unsafe { std::env::set_var("HOME", dir.path().to_str().unwrap()) };
        let _ = super::cmd_plugin_install(dir.path().to_str().unwrap());
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn cmd_plugin_uninstall_not_found() {
        unsafe { std::env::set_var("HOME", "/tmp/ironclad_test_uninstall_home") };
        super::cmd_plugin_uninstall("nonexistent").unwrap();
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn cmd_plugin_uninstall_exists() {
        let dir = tempfile::tempdir().unwrap();
        let plugins_dir = dir
            .path()
            .join(".ironclad")
            .join("plugins")
            .join("myplugin");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        std::fs::write(plugins_dir.join("plugin.toml"), "name = \"myplugin\"").unwrap();
        unsafe { std::env::set_var("HOME", dir.path().to_str().unwrap()) };
        super::cmd_plugin_uninstall("myplugin").unwrap();
        assert!(!plugins_dir.exists());
        unsafe { std::env::remove_var("HOME") };
    }

    // ── Models ────────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_models_list_full_config() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/config", serde_json::json!({
            "models": {
                "primary": "qwen3:8b",
                "fallbacks": ["gpt-4o", "claude-3"],
                "routing": { "mode": "adaptive", "confidence_threshold": 0.85, "local_first": false }
            }
        })).await;
        super::cmd_models_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_models_list_minimal_config() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/config", serde_json::json!({})).await;
        super::cmd_models_list(&s.uri()).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_models_scan_no_providers() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/config", serde_json::json!({"providers": {}})).await;
        super::cmd_models_scan(&s.uri(), None).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_models_scan_with_local_provider() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/config",
            serde_json::json!({
                "providers": {
                    "ollama": {"url": &format!("{}/ollama", s.uri())}
                }
            }),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/ollama/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "qwen3:8b"}, {"id": "llama3:70b"}]
            })))
            .mount(&s)
            .await;
        super::cmd_models_scan(&s.uri(), None).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_models_scan_local_ollama() {
        let s = MockServer::start().await;
        let _ollama_url = s.uri().to_string().replace("http://", "http://localhost:");
        mock_get(
            &s,
            "/api/config",
            serde_json::json!({
                "providers": {
                    "ollama": {"url": &s.uri()}
                }
            }),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [{"name": "qwen3:8b"}, {"model": "llama3"}]
            })))
            .mount(&s)
            .await;
        super::cmd_models_scan(&s.uri(), Some("ollama"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_models_scan_provider_filter_skips_others() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/config",
            serde_json::json!({
                "providers": {
                    "ollama": {"url": "http://localhost:11434"},
                    "openai": {"url": "https://api.openai.com"}
                }
            }),
        )
        .await;
        super::cmd_models_scan(&s.uri(), Some("openai"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_models_scan_empty_url() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/config",
            serde_json::json!({
                "providers": { "test": {"url": ""} }
            }),
        )
        .await;
        super::cmd_models_scan(&s.uri(), None).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_models_scan_error_response() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/config",
            serde_json::json!({
                "providers": {
                    "bad": {"url": &s.uri()}
                }
            }),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&s)
            .await;
        super::cmd_models_scan(&s.uri(), None).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_models_scan_no_models_found() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/config",
            serde_json::json!({
                "providers": {
                    "empty": {"url": &s.uri()}
                }
            }),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&s)
            .await;
        super::cmd_models_scan(&s.uri(), None).await.unwrap();
    }

    // ── Metrics ───────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_metrics_costs_with_data() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/stats/costs", serde_json::json!({
            "costs": [
                {"model": "qwen3:8b", "provider": "ollama", "tokens_in": 100, "tokens_out": 50, "cost": 0.001, "cached": false},
                {"model": "gpt-4o", "provider": "openai", "tokens_in": 200, "tokens_out": 100, "cost": 0.01, "cached": true}
            ]
        })).await;
        super::cmd_metrics(&s.uri(), "costs", None).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_metrics_costs_empty() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/stats/costs", serde_json::json!({"costs": []})).await;
        super::cmd_metrics(&s.uri(), "costs", None).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_metrics_transactions_with_data() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/stats/transactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "transactions": [
                    {"id": "tx-001", "tx_type": "inference", "amount": 0.01, "currency": "USD",
                     "counterparty": "openai", "created_at": "2025-01-01T12:00:00.000Z"},
                    {"id": "tx-002", "tx_type": "transfer", "amount": 5.00, "currency": "USDC",
                     "counterparty": "user", "created_at": "2025-01-01T13:00:00Z"}
                ]
            })))
            .mount(&s)
            .await;
        super::cmd_metrics(&s.uri(), "transactions", Some(48))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_metrics_transactions_empty() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/stats/transactions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"transactions": []})),
            )
            .mount(&s)
            .await;
        super::cmd_metrics(&s.uri(), "transactions", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cmd_metrics_cache_stats() {
        let s = MockServer::start().await;
        mock_get(
            &s,
            "/api/stats/cache",
            serde_json::json!({
                "hits": 42, "misses": 8, "entries": 100, "hit_rate": 84.0
            }),
        )
        .await;
        super::cmd_metrics(&s.uri(), "cache", None).await.unwrap();
    }

    #[tokio::test]
    async fn cmd_metrics_unknown_kind() {
        let s = MockServer::start().await;
        let result = super::cmd_metrics(&s.uri(), "bogus", None).await;
        assert!(result.is_err());
    }

    // ── Completion ────────────────────────────────────────────

    #[test]
    fn cmd_completion_bash() {
        super::cmd_completion("bash").unwrap();
    }

    #[test]
    fn cmd_completion_zsh() {
        super::cmd_completion("zsh").unwrap();
    }

    #[test]
    fn cmd_completion_fish() {
        super::cmd_completion("fish").unwrap();
    }

    #[test]
    fn cmd_completion_unknown() {
        super::cmd_completion("powershell").unwrap();
    }

    // ── Logs ──────────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_logs_static_with_entries() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/logs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entries": [
                    {"timestamp": "2025-01-01T00:00:00Z", "level": "INFO", "message": "Started", "target": "ironclad"},
                    {"timestamp": "2025-01-01T00:00:01Z", "level": "WARN", "message": "Low memory", "target": "system"},
                    {"timestamp": "2025-01-01T00:00:02Z", "level": "ERROR", "message": "Failed", "target": "api"},
                    {"timestamp": "2025-01-01T00:00:03Z", "level": "DEBUG", "message": "Trace", "target": "db"},
                    {"timestamp": "2025-01-01T00:00:04Z", "level": "TRACE", "message": "Deep", "target": "core"}
                ]
            })))
            .mount(&s)
            .await;
        super::cmd_logs(&s.uri(), 50, false, "info").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_logs_static_empty() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/logs"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"entries": []})),
            )
            .mount(&s)
            .await;
        super::cmd_logs(&s.uri(), 10, false, "info").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_logs_static_no_entries_key() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/logs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&s)
            .await;
        super::cmd_logs(&s.uri(), 10, false, "info").await.unwrap();
    }

    #[tokio::test]
    async fn cmd_logs_server_error_falls_back() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/logs"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&s)
            .await;
        super::cmd_logs(&s.uri(), 10, false, "info").await.unwrap();
    }

    // ── Security audit (filesystem) ──────────────────────────

    #[test]
    fn cmd_security_audit_missing_config() {
        super::cmd_security_audit("/tmp/ironclad_test_nonexistent_config.toml").unwrap();
    }

    #[test]
    fn cmd_security_audit_clean_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("ironclad.toml");
        std::fs::write(&config, "[server]\nbind = \"127.0.0.1\"\nport = 18789\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        super::cmd_security_audit(config.to_str().unwrap()).unwrap();
    }

    #[test]
    fn cmd_security_audit_plaintext_keys() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("ironclad.toml");
        std::fs::write(&config, "[providers.openai]\napi_key = \"sk-secret123\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        super::cmd_security_audit(config.to_str().unwrap()).unwrap();
    }

    #[test]
    fn cmd_security_audit_env_var_keys() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("ironclad.toml");
        std::fs::write(&config, "[providers.openai]\napi_key = \"${OPENAI_KEY}\"\n").unwrap();
        super::cmd_security_audit(config.to_str().unwrap()).unwrap();
    }

    #[test]
    fn cmd_security_audit_wildcard_cors() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("ironclad.toml");
        std::fs::write(
            &config,
            "[server]\nbind = \"0.0.0.0\"\n\n[cors]\norigins = \"*\"\n",
        )
        .unwrap();
        super::cmd_security_audit(config.to_str().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cmd_security_audit_loose_config_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("ironclad.toml");
        std::fs::write(&config, "[server]\nport = 18789\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::cmd_security_audit(config.to_str().unwrap()).unwrap();
    }

    // ── Reset (with --yes to skip stdin) ─────────────────────

    #[test]
    fn cmd_reset_yes_no_db() {
        let dir = tempfile::tempdir().unwrap();
        let ironclad_dir = dir.path().join(".ironclad");
        std::fs::create_dir_all(&ironclad_dir).unwrap();
        unsafe { std::env::set_var("HOME", dir.path().to_str().unwrap()) };
        super::cmd_reset(true).unwrap();
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn cmd_reset_yes_with_db_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let ironclad_dir = dir.path().join(".ironclad");
        std::fs::create_dir_all(&ironclad_dir).unwrap();
        std::fs::write(ironclad_dir.join("state.db"), "fake db").unwrap();
        std::fs::write(ironclad_dir.join("state.db-wal"), "wal").unwrap();
        std::fs::write(ironclad_dir.join("state.db-shm"), "shm").unwrap();
        std::fs::write(ironclad_dir.join("ironclad.toml"), "[server]").unwrap();
        std::fs::create_dir_all(ironclad_dir.join("logs")).unwrap();
        std::fs::write(ironclad_dir.join("wallet.json"), "{}").unwrap();
        unsafe { std::env::set_var("HOME", dir.path().to_str().unwrap()) };
        super::cmd_reset(true).unwrap();
        assert!(!ironclad_dir.join("state.db").exists());
        assert!(!ironclad_dir.join("ironclad.toml").exists());
        assert!(!ironclad_dir.join("logs").exists());
        assert!(ironclad_dir.join("wallet.json").exists());
        unsafe { std::env::remove_var("HOME") };
    }

    // ── Mechanic ──────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_mechanic_gateway_up() {
        let s = MockServer::start().await;
        mock_get(&s, "/api/health", serde_json::json!({"status": "ok"})).await;
        mock_get(&s, "/api/config", serde_json::json!({"models": {}})).await;
        mock_get(
            &s,
            "/api/skills",
            serde_json::json!({"skills": [{"id": "s1"}]}),
        )
        .await;
        mock_get(
            &s,
            "/api/wallet/balance",
            serde_json::json!({"balance": "1.00"}),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/api/channels/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"connected": true}, {"connected": false}
            ])))
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let ironclad_dir = dir.path().join(".ironclad");
        for sub in &["workspace", "skills", "plugins", "logs"] {
            std::fs::create_dir_all(ironclad_dir.join(sub)).unwrap();
        }
        std::fs::write(ironclad_dir.join("ironclad.toml"), "[server]").unwrap();
        unsafe { std::env::set_var("HOME", dir.path().to_str().unwrap()) };
        let _ = super::cmd_mechanic(&s.uri(), false).await;
        unsafe { std::env::remove_var("HOME") };
    }

    #[tokio::test]
    async fn cmd_mechanic_gateway_down() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/health"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", dir.path().to_str().unwrap()) };
        let _ = super::cmd_mechanic(&s.uri(), false).await;
        unsafe { std::env::remove_var("HOME") };
    }

    #[tokio::test]
    #[ignore = "sets HOME globally, racy with parallel tests — run with --ignored"]
    async fn cmd_mechanic_repair_creates_dirs() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})),
            )
            .mount(&s)
            .await;
        mock_get(&s, "/api/config", serde_json::json!({})).await;
        mock_get(&s, "/api/skills", serde_json::json!({"skills": []})).await;
        mock_get(&s, "/api/wallet/balance", serde_json::json!({})).await;
        Mock::given(method("GET"))
            .and(path("/api/channels/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", dir.path().to_str().unwrap()) };
        let _ = super::cmd_mechanic(&s.uri(), true).await;
        assert!(dir.path().join(".ironclad").join("workspace").exists());
        unsafe { std::env::remove_var("HOME") };
    }
}
