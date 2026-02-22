use ironclad_core::{IroncladError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info};

/// Configuration for a WASM plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmPluginConfig {
    pub name: String,
    pub wasm_path: PathBuf,
    #[serde(default = "default_memory_limit")]
    pub memory_limit_bytes: u64,
    #[serde(default = "default_execution_timeout_ms")]
    pub execution_timeout_ms: u64,
    #[serde(default)]
    pub capabilities: Vec<WasmCapability>,
}

fn default_memory_limit() -> u64 {
    64 * 1024 * 1024
}
fn default_execution_timeout_ms() -> u64 {
    30_000
}

/// Capabilities granted to a WASM plugin (deny-by-default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WasmCapability {
    ReadFilesystem,
    WriteFilesystem,
    Network,
    Environment,
}

/// Represents a loaded WASM plugin (the sandbox instance).
#[derive(Debug)]
pub struct WasmPlugin {
    pub config: WasmPluginConfig,
    pub loaded: bool,
    pub invocation_count: u64,
    pub last_error: Option<String>,
}

impl WasmPlugin {
    pub fn new(config: WasmPluginConfig) -> Self {
        Self {
            config,
            loaded: false,
            invocation_count: 0,
            last_error: None,
        }
    }

    /// Load the WASM module (validates the file exists and is readable).
    pub fn load(&mut self) -> Result<()> {
        if !self.config.wasm_path.exists() {
            return Err(IroncladError::Config(format!(
                "WASM file not found: {}",
                self.config.wasm_path.display()
            )));
        }

        let metadata = std::fs::metadata(&self.config.wasm_path)
            .map_err(|e| IroncladError::Config(format!("cannot read WASM file: {e}")))?;

        if metadata.len() == 0 {
            return Err(IroncladError::Config("WASM file is empty".into()));
        }

        self.loaded = true;
        info!(
            name = %self.config.name,
            size = metadata.len(),
            "loaded WASM plugin"
        );
        Ok(())
    }

    /// Execute the plugin with JSON input, returning JSON output.
    /// In a real implementation, this would invoke the wasmtime runtime.
    pub fn execute(&mut self, input: &serde_json::Value) -> Result<serde_json::Value> {
        if !self.loaded {
            return Err(IroncladError::Config("WASM plugin not loaded".into()));
        }

        self.invocation_count += 1;
        debug!(
            name = %self.config.name,
            invocations = self.invocation_count,
            "executing WASM plugin"
        );

        Ok(serde_json::json!({
            "status": "executed",
            "plugin": self.config.name,
            "input_keys": input.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
        }))
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn has_capability(&self, cap: &WasmCapability) -> bool {
        self.config.capabilities.contains(cap)
    }

    pub fn unload(&mut self) {
        self.loaded = false;
        debug!(name = %self.config.name, "unloaded WASM plugin");
    }
}

/// Manages multiple WASM plugins.
#[derive(Debug, Default)]
pub struct WasmPluginRegistry {
    plugins: HashMap<String, WasmPlugin>,
}

impl WasmPluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, config: WasmPluginConfig) -> Result<()> {
        let name = config.name.clone();
        let plugin = WasmPlugin::new(config);
        self.plugins.insert(name, plugin);
        Ok(())
    }

    pub fn load_plugin(&mut self, name: &str) -> Result<()> {
        let plugin = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| IroncladError::Config(format!("plugin '{}' not registered", name)))?;
        plugin.load()
    }

    pub fn execute(&mut self, name: &str, input: &serde_json::Value) -> Result<serde_json::Value> {
        let plugin = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| IroncladError::Config(format!("plugin '{}' not found", name)))?;
        plugin.execute(input)
    }

    pub fn get(&self, name: &str) -> Option<&WasmPlugin> {
        self.plugins.get(name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    pub fn loaded_count(&self) -> usize {
        self.plugins.values().filter(|p| p.loaded).count()
    }

    pub fn total_count(&self) -> usize {
        self.plugins.len()
    }

    pub fn unload_all(&mut self) {
        for plugin in self.plugins.values_mut() {
            plugin.unload();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn test_config(dir: &Path, name: &str) -> WasmPluginConfig {
        let wasm_path = dir.join(format!("{name}.wasm"));
        fs::write(&wasm_path, b"fake wasm bytes").unwrap();
        WasmPluginConfig {
            name: name.to_string(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        }
    }

    #[test]
    fn plugin_load_and_execute() {
        let dir = TempDir::new().unwrap();
        let config = test_config(dir.path(), "test-plugin");
        let mut plugin = WasmPlugin::new(config);

        assert!(!plugin.is_loaded());
        plugin.load().unwrap();
        assert!(plugin.is_loaded());

        let result = plugin
            .execute(&serde_json::json!({"key": "value"}))
            .unwrap();
        assert_eq!(result["status"], "executed");
        assert_eq!(plugin.invocation_count, 1);
    }

    #[test]
    fn plugin_load_missing_file() {
        let config = WasmPluginConfig {
            name: "missing".to_string(),
            wasm_path: PathBuf::from("/nonexistent/plugin.wasm"),
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        assert!(plugin.load().is_err());
    }

    #[test]
    fn plugin_load_empty_file() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("empty.wasm");
        fs::write(&wasm_path, b"").unwrap();

        let config = WasmPluginConfig {
            name: "empty".to_string(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        assert!(plugin.load().is_err());
    }

    #[test]
    fn plugin_execute_without_load() {
        let config = WasmPluginConfig {
            name: "not-loaded".to_string(),
            wasm_path: PathBuf::from("/fake.wasm"),
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        assert!(plugin.execute(&serde_json::json!({})).is_err());
    }

    #[test]
    fn plugin_capabilities() {
        let config = WasmPluginConfig {
            name: "caps".to_string(),
            wasm_path: PathBuf::from("/fake.wasm"),
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![WasmCapability::ReadFilesystem, WasmCapability::Network],
        };
        let plugin = WasmPlugin::new(config);
        assert!(plugin.has_capability(&WasmCapability::ReadFilesystem));
        assert!(plugin.has_capability(&WasmCapability::Network));
        assert!(!plugin.has_capability(&WasmCapability::WriteFilesystem));
    }

    #[test]
    fn plugin_unload() {
        let dir = TempDir::new().unwrap();
        let config = test_config(dir.path(), "unload-test");
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();
        assert!(plugin.is_loaded());
        plugin.unload();
        assert!(!plugin.is_loaded());
    }

    #[test]
    fn registry_register_and_list() {
        let dir = TempDir::new().unwrap();
        let mut reg = WasmPluginRegistry::new();
        reg.register(test_config(dir.path(), "a")).unwrap();
        reg.register(test_config(dir.path(), "b")).unwrap();
        assert_eq!(reg.total_count(), 2);
        assert_eq!(reg.loaded_count(), 0);
    }

    #[test]
    fn registry_load_and_execute() {
        let dir = TempDir::new().unwrap();
        let mut reg = WasmPluginRegistry::new();
        reg.register(test_config(dir.path(), "plugin")).unwrap();
        reg.load_plugin("plugin").unwrap();
        assert_eq!(reg.loaded_count(), 1);

        let result = reg
            .execute("plugin", &serde_json::json!({"q": "test"}))
            .unwrap();
        assert_eq!(result["status"], "executed");
    }

    #[test]
    fn registry_execute_unknown() {
        let mut reg = WasmPluginRegistry::new();
        assert!(reg.execute("nope", &serde_json::json!({})).is_err());
    }

    #[test]
    fn registry_unload_all() {
        let dir = TempDir::new().unwrap();
        let mut reg = WasmPluginRegistry::new();
        reg.register(test_config(dir.path(), "a")).unwrap();
        reg.register(test_config(dir.path(), "b")).unwrap();
        reg.load_plugin("a").unwrap();
        reg.load_plugin("b").unwrap();
        assert_eq!(reg.loaded_count(), 2);
        reg.unload_all();
        assert_eq!(reg.loaded_count(), 0);
    }

    #[test]
    fn config_serde() {
        let config = WasmPluginConfig {
            name: "test".to_string(),
            wasm_path: PathBuf::from("/tmp/test.wasm"),
            memory_limit_bytes: 1024,
            execution_timeout_ms: 5000,
            capabilities: vec![WasmCapability::Network],
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: WasmPluginConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "test");
        assert_eq!(back.capabilities, vec![WasmCapability::Network]);
    }
}
