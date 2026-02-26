use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use ironclad_core::IroncladConfig;

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
    if let Ok(home) = std::env::var("HOME") {
        let home_cfg = Path::new(&home).join(".ironclad").join("ironclad.toml");
        if home_cfg.exists() {
            return home_cfg;
        }
    }
    local
}

pub fn parse_and_validate_toml(content: &str) -> Result<IroncladConfig, ConfigRuntimeError> {
    let cfg: IroncladConfig = toml::from_str(content)?;
    cfg.validate()
        .map_err(|e| ConfigRuntimeError::Validation(e.to_string()))?;
    Ok(cfg)
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
    Ok(Some(backup_path))
}

pub fn write_config_atomic(path: &Path, cfg: &IroncladConfig) -> Result<(), ConfigRuntimeError> {
    let parent = path
        .parent()
        .ok_or_else(|| ConfigRuntimeError::MissingParent(path.to_path_buf()))?;
    std::fs::create_dir_all(parent)?;
    let content = toml::to_string_pretty(cfg)?;
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

    let mut deferred_apply = Vec::new();
    deferred_apply.push("server.bind".to_string());
    deferred_apply.push("server.port".to_string());
    deferred_apply.push("wallet".to_string());
    deferred_apply.push("treasury.policy_engine".to_string());
    deferred_apply.push("browser.runtime".to_string());

    let apply_result: Result<(), ConfigRuntimeError> = async {
        {
            let mut config = state.config.write().await;
            *config = updated.clone();
        }
        {
            let mut llm = state.llm.write().await;
            llm.router.sync_runtime(
                updated.models.primary.clone(),
                updated.models.fallbacks.clone(),
                updated.models.routing.clone(),
            );
        }
        {
            let mut a2a = state.a2a.write().await;
            a2a.config = updated.a2a.clone();
        }
        Ok(())
    }
    .await;

    if let Err(err) = apply_result {
        if let Some(ref backup) = backup_path {
            let _ = restore_from_backup(&config_path, backup);
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
}
