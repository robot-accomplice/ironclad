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
}
