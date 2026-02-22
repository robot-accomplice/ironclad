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
pub struct WasmPlugin {
    pub config: WasmPluginConfig,
    pub loaded: bool,
    pub invocation_count: u64,
    pub last_error: Option<String>,
    engine: Option<wasmer::Engine>,
    module: Option<wasmer::Module>,
}

impl std::fmt::Debug for WasmPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmPlugin")
            .field("config", &self.config)
            .field("loaded", &self.loaded)
            .field("invocation_count", &self.invocation_count)
            .field("last_error", &self.last_error)
            .field("has_engine", &self.engine.is_some())
            .field("has_module", &self.module.is_some())
            .finish()
    }
}

impl WasmPlugin {
    pub fn new(config: WasmPluginConfig) -> Self {
        Self {
            config,
            loaded: false,
            invocation_count: 0,
            last_error: None,
            engine: None,
            module: None,
        }
    }

    /// Load and compile the WASM module from disk.
    pub fn load(&mut self) -> Result<()> {
        if !self.config.wasm_path.exists() {
            return Err(IroncladError::Config(format!(
                "WASM file not found: {}",
                self.config.wasm_path.display()
            )));
        }

        let wasm_bytes = std::fs::read(&self.config.wasm_path)
            .map_err(|e| IroncladError::Config(format!("cannot read WASM file: {e}")))?;

        if wasm_bytes.is_empty() {
            return Err(IroncladError::Config("WASM file is empty".into()));
        }

        let engine = wasmer::Engine::default();
        let module = wasmer::Module::new(&engine, &wasm_bytes)
            .map_err(|e| IroncladError::Config(format!("WASM compilation failed: {e}")))?;

        let size = wasm_bytes.len();
        self.engine = Some(engine);
        self.module = Some(module);
        self.loaded = true;

        info!(
            name = %self.config.name,
            size,
            "loaded WASM plugin"
        );
        Ok(())
    }

    /// Execute the plugin with JSON input, returning JSON output.
    pub fn execute(&mut self, _input: &serde_json::Value) -> Result<serde_json::Value> {
        if !self.loaded {
            return Err(IroncladError::Config("WASM plugin not loaded".into()));
        }

        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| IroncladError::Config("WASM engine not initialized".into()))?;
        let module = self
            .module
            .as_ref()
            .ok_or_else(|| IroncladError::Config("WASM module not compiled".into()))?;

        self.invocation_count += 1;
        debug!(
            name = %self.config.name,
            invocations = self.invocation_count,
            "executing WASM plugin"
        );

        let mut store = wasmer::Store::new(engine.clone());
        let imports = wasmer::Imports::new();
        let instance = wasmer::Instance::new(&mut store, module, &imports)
            .map_err(|e| IroncladError::Config(format!("WASM instantiation failed: {e}")))?;

        if let Ok(func) = instance.exports.get_function("process") {
            let results = func
                .call(&mut store, &[])
                .map_err(|e| IroncladError::Config(format!("WASM execution failed: {e}")))?;

            let result_values: Vec<serde_json::Value> =
                results.iter().map(wasmer_value_to_json).collect();

            if let Ok(memory) = instance.exports.get_memory("memory")
                && result_values.len() == 2
                && let (Some(ptr), Some(len)) =
                    (result_values[0].as_i64(), result_values[1].as_i64())
            {
                let view = memory.view(&store);
                let mut buf = vec![0u8; len as usize];
                if view.read(ptr as u64, &mut buf).is_ok()
                    && let Ok(text) = String::from_utf8(buf)
                {
                    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&text) {
                        return Ok(serde_json::json!({
                            "status": "executed",
                            "plugin": self.config.name,
                            "output": json_val,
                        }));
                    }
                    return Ok(serde_json::json!({
                        "status": "executed",
                        "plugin": self.config.name,
                        "output": text,
                    }));
                }
            }

            let result_json = match result_values.len() {
                0 => serde_json::Value::Null,
                1 => result_values.into_iter().next().unwrap(),
                _ => serde_json::json!(result_values),
            };

            return Ok(serde_json::json!({
                "status": "executed",
                "plugin": self.config.name,
                "result": result_json,
            }));
        }

        if let Ok(func) = instance.exports.get_function("_start") {
            func.call(&mut store, &[])
                .map_err(|e| IroncladError::Config(format!("WASM execution failed: {e}")))?;
            return Ok(serde_json::json!({
                "status": "executed",
                "plugin": self.config.name,
            }));
        }

        let export_names: Vec<String> = instance
            .exports
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();

        Ok(serde_json::json!({
            "status": "no_entry_point",
            "plugin": self.config.name,
            "available_exports": export_names,
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
        self.engine = None;
        self.module = None;
        debug!(name = %self.config.name, "unloaded WASM plugin");
    }
}

fn wasmer_value_to_json(val: &wasmer::Value) -> serde_json::Value {
    match val {
        wasmer::Value::I32(v) => serde_json::json!(v),
        wasmer::Value::I64(v) => serde_json::json!(v),
        wasmer::Value::F32(v) => serde_json::json!(v),
        wasmer::Value::F64(v) => serde_json::json!(v),
        other => serde_json::json!(format!("{:?}", other)),
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

    fn test_wasm_bytes() -> Vec<u8> {
        wat::parse_str(r#"(module (func (export "process") (result i32) i32.const 42))"#).unwrap()
    }

    fn test_config(dir: &Path, name: &str) -> WasmPluginConfig {
        let wasm_path = dir.join(format!("{name}.wasm"));
        fs::write(&wasm_path, test_wasm_bytes()).unwrap();
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
        assert_eq!(result["result"], 42);
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
    fn plugin_load_invalid_wasm() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("invalid.wasm");
        fs::write(&wasm_path, b"not valid wasm bytes").unwrap();

        let config = WasmPluginConfig {
            name: "invalid".to_string(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        let err = plugin.load().unwrap_err();
        assert!(err.to_string().contains("WASM compilation failed"));
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
        assert!(plugin.engine.is_none());
        assert!(plugin.module.is_none());
    }

    #[test]
    fn plugin_no_entry_point() {
        let dir = TempDir::new().unwrap();
        let wasm_bytes =
            wat::parse_str(r#"(module (func (export "other_fn") (result i32) i32.const 1))"#)
                .unwrap();
        let wasm_path = dir.path().join("no-entry.wasm");
        fs::write(&wasm_path, wasm_bytes).unwrap();

        let config = WasmPluginConfig {
            name: "no-entry".to_string(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let result = plugin.execute(&serde_json::json!({})).unwrap();
        assert_eq!(result["status"], "no_entry_point");
        let exports = result["available_exports"].as_array().unwrap();
        assert!(exports.iter().any(|e| e == "other_fn"));
    }

    #[test]
    fn plugin_start_entry_point() {
        let dir = TempDir::new().unwrap();
        let wasm_bytes = wat::parse_str(r#"(module (func (export "_start") nop))"#).unwrap();
        let wasm_path = dir.path().join("start.wasm");
        fs::write(&wasm_path, wasm_bytes).unwrap();

        let config = WasmPluginConfig {
            name: "start".to_string(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let result = plugin.execute(&serde_json::json!({})).unwrap();
        assert_eq!(result["status"], "executed");
        assert_eq!(result["plugin"], "start");
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
        assert_eq!(result["result"], 42);
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
