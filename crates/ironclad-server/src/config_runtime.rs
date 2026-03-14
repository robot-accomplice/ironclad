use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use ironclad_core::{IroncladConfig, home_dir};

use crate::api::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigApplyStatus {
    pub config_path: String,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub last_backup_path: Option<String>,
    pub deferred_apply: Vec<String>,
}

impl ConfigApplyStatus {
    pub fn new(config_path: &Path) -> Self {
        Self {
            config_path: config_path.display().to_string(),
            last_attempt_at: None,
            last_success_at: None,
            last_error: None,
            last_backup_path: None,
            deferred_apply: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeApplyReport {
    pub backup_path: Option<String>,
    pub deferred_apply: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigRuntimeError {
    Io(std::io::Error),
    TomlDeserialize(toml::de::Error),
    TomlSerialize(toml::ser::Error),
    JsonSerialize(serde_json::Error),
    Validation(String),
    MissingParent(PathBuf),
}

impl std::fmt::Display for ConfigRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::TomlDeserialize(e) => write!(f, "TOML parse error: {e}"),
            Self::TomlSerialize(e) => write!(f, "TOML serialize error: {e}"),
            Self::JsonSerialize(e) => write!(f, "JSON serialize error: {e}"),
            Self::Validation(e) => write!(f, "validation failed: {e}"),
            Self::MissingParent(p) => {
                write!(
                    f,
                    "config parent directory is missing for '{}'",
                    p.display()
                )
            }
        }
    }
}

impl std::error::Error for ConfigRuntimeError {}

impl From<std::io::Error> for ConfigRuntimeError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<toml::de::Error> for ConfigRuntimeError {
    fn from(value: toml::de::Error) -> Self {
        Self::TomlDeserialize(value)
    }
}

impl From<toml::ser::Error> for ConfigRuntimeError {
    fn from(value: toml::ser::Error) -> Self {
        Self::TomlSerialize(value)
    }
}

impl From<serde_json::Error> for ConfigRuntimeError {
    fn from(value: serde_json::Error) -> Self {
        Self::JsonSerialize(value)
    }
}

pub fn resolve_default_config_path() -> PathBuf {
    let local = PathBuf::from("ironclad.toml");
    if local.exists() {
        return local;
    }
    let home_cfg = home_dir().join(".ironclad").join("ironclad.toml");
    if home_cfg.exists() {
        return home_cfg;
    }
    local
}

pub fn parse_and_validate_toml(content: &str) -> Result<IroncladConfig, ConfigRuntimeError> {
    // Delegate to IroncladConfig::from_str which runs normalize_paths(),
    // merge_bundled_providers(), and validate() — matching the startup path.
    // Without this, hot-reloaded configs would have raw ~ paths and missing
    // bundled providers.
    IroncladConfig::from_str(content).map_err(|e| ConfigRuntimeError::Validation(e.to_string()))
}

pub fn parse_and_validate_file(path: &Path) -> Result<IroncladConfig, ConfigRuntimeError> {
    let content = std::fs::read_to_string(path)?;
    parse_and_validate_toml(&content)
}

pub fn backup_config_file(path: &Path) -> Result<Option<PathBuf>, ConfigRuntimeError> {
    if !path.exists() {
        return Ok(None);
    }
    let parent = path
        .parent()
        .ok_or_else(|| ConfigRuntimeError::MissingParent(path.to_path_buf()))?;
    std::fs::create_dir_all(parent)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let file_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("ironclad.toml");
    let backup_name = format!("{file_name}.bak.{stamp}");
    let backup_path = parent.join(backup_name);
    std::fs::copy(path, &backup_path)?;
    prune_old_backups(path, 10);
    Ok(Some(backup_path))
}

/// Remove old config backups, keeping only the most recent `keep` files.
fn prune_old_backups(config_path: &Path, keep: usize) {
    let Some(dir) = config_path.parent() else {
        return;
    };
    let Some(filename) = config_path.file_name().and_then(|f| f.to_str()) else {
        return;
    };
    let prefix = format!("{filename}.bak.");

    let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .map(|e| e.path())
        .collect();

    if backups.len() <= keep {
        return;
    }

    // Sort by name ascending — the timestamp suffix is ISO-8601 so
    // lexicographic order == chronological order (oldest first).
    backups.sort();

    let to_remove = backups.len() - keep;
    for path in backups.into_iter().take(to_remove) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Recursively normalize Windows backslash paths to forward slashes in JSON
/// string values that look like filesystem paths.
fn normalize_backslash_paths(value: &mut Value) {
    match value {
        Value::String(s) => {
            // Heuristic: looks like a Windows absolute path (e.g., C:\Users\...)
            // or contains backslash-separated segments that resemble paths.
            if s.contains('\\')
                && (s.starts_with("C:\\") || s.starts_with("D:\\") || s.contains(":\\"))
            {
                *s = s.replace('\\', "/");
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                normalize_backslash_paths(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                normalize_backslash_paths(v);
            }
        }
        _ => {}
    }
}

pub fn write_config_atomic(path: &Path, cfg: &IroncladConfig) -> Result<(), ConfigRuntimeError> {
    let parent = path
        .parent()
        .ok_or_else(|| ConfigRuntimeError::MissingParent(path.to_path_buf()))?;
    std::fs::create_dir_all(parent)?;
    // BUG-031: Normalize Windows backslash paths to forward slashes before
    // TOML serialization.  TOML basic strings treat `\U` as a unicode
    // escape, which breaks `C:\Users\...` paths.  Round-trip through JSON
    // to normalize path-like string values.
    let mut json_val = serde_json::to_value(cfg).map_err(ConfigRuntimeError::JsonSerialize)?;
    normalize_backslash_paths(&mut json_val);
    let normalized: IroncladConfig =
        serde_json::from_value(json_val).map_err(ConfigRuntimeError::JsonSerialize)?;
    let content = toml::to_string_pretty(&normalized)?;
    let tmp_name = format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("ironclad"),
        uuid::Uuid::new_v4()
    );
    let tmp_path = parent.join(tmp_name);
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

pub fn restore_from_backup(path: &Path, backup_path: &Path) -> Result<(), ConfigRuntimeError> {
    let content = std::fs::read(backup_path)?;
    std::fs::write(path, content)?;
    Ok(())
}

pub fn merge_patch(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                let entry = base_map.entry(k.clone()).or_insert(Value::Null);
                merge_patch(entry, v);
            }
        }
        (base, patch) => {
            *base = patch.clone();
        }
    }
}

pub async fn apply_runtime_config(
    state: &AppState,
    updated: IroncladConfig,
) -> Result<RuntimeApplyReport, ConfigRuntimeError> {
    let config_path = state.config_path.as_ref().clone();
    let old_config = state.config.read().await.clone();
    let backup_path = backup_config_file(&config_path)?;
    write_config_atomic(&config_path, &updated)?;

    // Only settings that genuinely require a process restart belong here.
    // server.bind/port: requires rebinding the TCP listener socket.
    // wallet: holds crypto keys + chain state; partial swap risks fund loss.
    let deferred_apply = vec![
        "server.bind".to_string(),
        "server.port".to_string(),
        "wallet".to_string(),
    ];

    let apply_result: Result<(), ConfigRuntimeError> = async {
        // Core config swap — all subsequent reads see the new config.
        {
            let mut config = state.config.write().await;
            *config = updated.clone();
        }
        // LLM routing: primary/fallback chain, routing mode, timeout budgets.
        {
            let mut llm = state.llm.write().await;
            llm.router.sync_runtime(
                updated.models.primary.clone(),
                updated.models.fallbacks.clone(),
                updated.models.routing.clone(),
            );
            llm.breakers.sync_config(&updated.circuit_breaker);
        }
        // A2A protocol config.
        {
            let mut a2a = state.a2a.write().await;
            a2a.config = updated.a2a.clone();
        }
        // Personality: agent name, persona, tone — already behind RwLock.
        state.reload_personality().await;
        Ok(())
    }
    .await;

    if let Err(err) = apply_result {
        if let Some(ref backup) = backup_path
            && let Err(e) = restore_from_backup(&config_path, backup)
        {
            tracing::error!(error = %e, path = %config_path.display(), "failed to restore config from backup — config file may be corrupted");
        }
        {
            let mut config = state.config.write().await;
            *config = old_config;
        }
        return Err(err);
    }

    Ok(RuntimeApplyReport {
        backup_path: backup_path.map(|p| p.display().to_string()),
        deferred_apply,
    })
}

pub fn config_value_from_file_or_runtime(
    path: &Path,
    runtime_cfg: &IroncladConfig,
) -> Result<Value, ConfigRuntimeError> {
    if path.exists() {
        let parsed = parse_and_validate_file(path)?;
        return Ok(serde_json::to_value(parsed)?);
    }
    Ok(serde_json::to_value(runtime_cfg)?)
}

pub fn status_for_path(path: &Path) -> Arc<RwLock<ConfigApplyStatus>> {
    Arc::new(RwLock::new(ConfigApplyStatus::new(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> &'static str {
        r#"
[agent]
name = "Test"
id = "test"

[server]
port = 18789

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"
"#
    }

    #[test]
    fn parse_and_validate_toml_accepts_valid_content() {
        let cfg = parse_and_validate_toml(test_config()).expect("valid config");
        assert_eq!(cfg.agent.id, "test");
    }

    #[test]
    fn parse_and_validate_toml_rejects_invalid_content() {
        let err = parse_and_validate_toml("[agent]\nname = 1").expect_err("must fail");
        assert!(err.to_string().contains("TOML"));
    }

    #[test]
    fn backup_config_file_creates_timestamped_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ironclad.toml");
        std::fs::write(&path, test_config()).expect("seed config");
        let backup = backup_config_file(&path)
            .expect("backup ok")
            .expect("backup path");
        assert!(backup.exists());
        let name = backup.file_name().and_then(|v| v.to_str()).unwrap_or("");
        assert!(name.starts_with("ironclad.toml.bak."));
    }

    #[test]
    fn write_config_atomic_persists_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ironclad.toml");
        let cfg = parse_and_validate_toml(test_config()).expect("parse");
        write_config_atomic(&path, &cfg).expect("write");
        let written = std::fs::read_to_string(path).expect("read");
        assert!(written.contains("[agent]"));
        assert!(written.contains("primary = \"ollama/qwen3:8b\""));
    }

    #[test]
    fn restore_from_backup_restores_original_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ironclad.toml");
        let backup_path = dir.path().join("ironclad.toml.bak");

        let original = "original-content";
        let overwritten = "overwritten-content";

        std::fs::write(&path, original).expect("seed original");
        std::fs::write(&backup_path, original).expect("seed backup");
        std::fs::write(&path, overwritten).expect("overwrite");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), overwritten);

        restore_from_backup(&path, &backup_path).expect("restore");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn merge_patch_deep_merge_objects() {
        let mut base = serde_json::json!({"a": {"inner": 1, "keep": true}});
        merge_patch(
            &mut base,
            &serde_json::json!({"a": {"inner": 99, "new": "val"}}),
        );
        assert_eq!(base["a"]["inner"], 99);
        assert_eq!(base["a"]["keep"], true);
        assert_eq!(base["a"]["new"], "val");
    }

    #[test]
    fn merge_patch_replaces_scalar() {
        let mut base = serde_json::json!({"key": "old"});
        merge_patch(&mut base, &serde_json::json!({"key": "new"}));
        assert_eq!(base["key"], "new");
    }

    #[test]
    fn merge_patch_adds_new_keys() {
        let mut base = serde_json::json!({"existing": 1});
        merge_patch(&mut base, &serde_json::json!({"added": 2}));
        assert_eq!(base["existing"], 1);
        assert_eq!(base["added"], 2);
    }

    #[test]
    fn merge_patch_replaces_array() {
        let mut base = serde_json::json!({"arr": [1, 2, 3]});
        merge_patch(&mut base, &serde_json::json!({"arr": [4, 5]}));
        assert_eq!(base["arr"], serde_json::json!([4, 5]));
    }

    #[test]
    fn merge_patch_replaces_scalar_with_object() {
        let mut base = serde_json::json!({"val": "string"});
        merge_patch(&mut base, &serde_json::json!({"val": {"nested": true}}));
        assert_eq!(base["val"]["nested"], true);
    }

    #[test]
    fn config_value_from_file_or_runtime_uses_file_when_it_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ironclad.toml");
        std::fs::write(&path, test_config()).expect("seed config");

        let runtime_cfg = parse_and_validate_toml(test_config()).expect("parse");
        let val = config_value_from_file_or_runtime(&path, &runtime_cfg).expect("read");
        assert_eq!(val["agent"]["id"], "test");
    }

    #[test]
    fn config_value_from_file_or_runtime_uses_runtime_when_no_file() {
        let path = std::path::PathBuf::from("/nonexistent/ironclad.toml");
        let runtime_cfg = parse_and_validate_toml(test_config()).expect("parse");
        let val = config_value_from_file_or_runtime(&path, &runtime_cfg).expect("read");
        assert_eq!(val["agent"]["id"], "test");
    }

    #[test]
    fn config_runtime_error_display_variants() {
        let io_err = ConfigRuntimeError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert!(io_err.to_string().contains("I/O error"));

        let validation_err = ConfigRuntimeError::Validation("bad field".into());
        assert!(validation_err.to_string().contains("validation failed"));

        let missing_parent = ConfigRuntimeError::MissingParent(PathBuf::from("/some/path"));
        assert!(
            missing_parent
                .to_string()
                .contains("config parent directory is missing")
        );
    }

    #[test]
    fn config_runtime_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err: ConfigRuntimeError = io_err.into();
        assert!(err.to_string().contains("I/O error"));
    }

    #[test]
    fn config_apply_status_new_initializes_empty() {
        let status = ConfigApplyStatus::new(std::path::Path::new("/tmp/test.toml"));
        assert_eq!(status.config_path, "/tmp/test.toml");
        assert!(status.last_attempt_at.is_none());
        assert!(status.last_success_at.is_none());
        assert!(status.last_error.is_none());
        assert!(status.last_backup_path.is_none());
        assert!(status.deferred_apply.is_empty());
    }

    #[test]
    fn status_for_path_returns_arc_rwlock() {
        let arc = status_for_path(std::path::Path::new("/tmp/status.toml"));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let status = rt.block_on(arc.read());
        assert_eq!(status.config_path, "/tmp/status.toml");
    }

    #[test]
    fn backup_config_file_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does_not_exist.toml");
        let result = backup_config_file(&path).expect("ok");
        assert!(result.is_none());
    }

    #[test]
    fn parse_and_validate_file_works_for_valid_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ironclad.toml");
        std::fs::write(&path, test_config()).expect("seed config");
        let cfg = parse_and_validate_file(&path).expect("parse");
        assert_eq!(cfg.agent.id, "test");
    }

    #[test]
    fn parse_and_validate_file_errors_for_missing_file() {
        let err = parse_and_validate_file(std::path::Path::new("/nonexistent/file.toml"));
        assert!(err.is_err());
    }

    #[test]
    fn prune_old_backups_keeps_newest() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("test.toml");
        std::fs::write(&config, "").unwrap();

        // Create 15 backups with lexicographically ordered timestamps.
        for i in 0..15 {
            let name = format!("test.toml.bak.20260301T12{i:02}00.000Z");
            std::fs::write(dir.path().join(&name), "").unwrap();
        }

        prune_old_backups(&config, 10);

        let remaining: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().unwrap_or("").contains(".bak."))
            .collect();

        assert_eq!(remaining.len(), 10);
    }
}
