//! Health and logs API handlers.

use axum::{extract::State, response::IntoResponse};
use serde_json::Value;

use super::{AppState, internal_err};

/// Structured log entry returned by the logs API (from tracing JSON lines).
#[derive(Debug, serde::Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
    }))
}

pub async fn get_logs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let lines_limit = params
        .get("lines")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(10_000);
    let level_filter = params
        .get("level")
        .map(|s| s.to_lowercase())
        .filter(|s| matches!(s.as_str(), "info" | "warn" | "error" | "debug" | "trace"));

    let log_dir = {
        let config = state.config.read().await;
        config.server.log_dir.clone()
    };

    let entries = match read_log_entries(&log_dir, lines_limit, level_filter.as_deref()) {
        Ok(entries) => entries,
        Err(e) => return Err(internal_err(&e)),
    };
    Ok(axum::Json(serde_json::json!({ "entries": entries })))
}

/// Read the most recent log file in `log_dir`, tail up to `lines` lines, optionally filter by level.
pub fn read_log_entries(
    log_dir: &std::path::Path,
    lines: usize,
    level_filter: Option<&str>,
) -> Result<Vec<LogEntry>, String> {
    let mut log_files: Vec<std::path::PathBuf> = std::fs::read_dir(log_dir)
        .map_err(|e| format!("failed to read log directory: {}", e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("ironclad.log") || n.ends_with(".log"))
        })
        .collect();
    if log_files.is_empty() {
        return Ok(vec![]);
    }
    log_files.sort_by(|a, b| {
        let ma = std::fs::metadata(a)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mb = std::fs::metadata(b)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        ma.cmp(&mb).reverse().then_with(|| a.cmp(b).reverse())
    });
    let path = log_files
        .first()
        .cloned()
        .ok_or_else(|| "no log file path (empty list after sort)".to_string())?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read log file: {}", e))?;
    let raw_lines: Vec<&str> = content.lines().rev().take(lines).collect();
    let raw_lines: Vec<&str> = raw_lines.into_iter().rev().collect();
    let mut entries = Vec::with_capacity(raw_lines.len());
    for line in raw_lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let obj: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let level = obj
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if let Some(filter) = level_filter
            && level != filter
        {
            continue;
        }
        let message = obj
            .get("fields")
            .and_then(|f| f.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let timestamp = obj
            .get("timestamp")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let target = obj.get("target").and_then(|t| t.as_str()).map(String::from);
        entries.push(LogEntry {
            timestamp,
            level,
            message,
            target,
        });
    }
    Ok(entries)
}
