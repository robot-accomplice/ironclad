use ironclad_core::{IroncladError, Result, input_capability_scan};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, warn};

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

        for export in module.exports() {
            if let wasmer::ExternType::Memory(mem_type) = export.ty() {
                let min_bytes = mem_type.minimum.0 as u64 * 65_536;
                if min_bytes > self.config.memory_limit_bytes {
                    return Err(IroncladError::Config(format!(
                        "WASM module minimum memory ({min_bytes} bytes) exceeds limit ({} bytes)",
                        self.config.memory_limit_bytes
                    )));
                }
            }
        }

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
    pub fn execute(&mut self, input: &serde_json::Value) -> Result<serde_json::Value> {
        if !self.loaded {
            return Err(IroncladError::Config("WASM plugin not loaded".into()));
        }
        self.enforce_capabilities(input)?;

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

        if let Ok(memory) = instance.exports.get_memory("memory")
            && let Ok(input_bytes) = serde_json::to_vec(input)
        {
            let view = memory.view(&store);
            let mem_size = view.data_size() as usize;
            if input_bytes.len() <= mem_size {
                if let Err(e) = view.write(0, &input_bytes) {
                    warn!(
                        plugin = %self.config.name,
                        error = %e,
                        "failed to write input to WASM memory"
                    );
                }
            } else {
                warn!(
                    plugin = %self.config.name,
                    input_len = input_bytes.len(),
                    mem_size,
                    "input exceeds WASM memory size, skipping write"
                );
            }
        }

        let deadline = std::time::Duration::from_millis(self.config.execution_timeout_ms);

        if let Ok(func) = instance.exports.get_function("process") {
            let start = std::time::Instant::now();
            let results = func
                .call(&mut store, &[])
                .map_err(|e| IroncladError::Config(format!("WASM execution failed: {e}")))?;

            let elapsed = start.elapsed();
            if elapsed > deadline {
                warn!(
                    plugin = %self.config.name,
                    elapsed_ms = elapsed.as_millis() as u64,
                    deadline_ms = self.config.execution_timeout_ms,
                    "WASM execution exceeded configured timeout"
                );
            }

            let result_values: Vec<serde_json::Value> =
                results.iter().map(wasmer_value_to_json).collect();

            if let Ok(memory) = instance.exports.get_memory("memory")
                && result_values.len() == 2
                && let Some(ptr) = result_values[0].as_i64().filter(|&v| v >= 0)
                && let Some(len) = result_values[1]
                    .as_i64()
                    .filter(|&v| v > 0 && v <= 10_000_000)
            {
                let view = memory.view(&store);
                let mem_size = view.data_size();
                let end = (ptr as u64).saturating_add(len as u64);
                if end > mem_size {
                    return Err(IroncladError::Config(format!(
                        "WASM memory read out of bounds: ptr={ptr}, len={len}, memory_size={mem_size}"
                    )));
                }

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
            let start = std::time::Instant::now();
            func.call(&mut store, &[])
                .map_err(|e| IroncladError::Config(format!("WASM execution failed: {e}")))?;

            let elapsed = start.elapsed();
            if elapsed > deadline {
                warn!(
                    plugin = %self.config.name,
                    elapsed_ms = elapsed.as_millis() as u64,
                    deadline_ms = self.config.execution_timeout_ms,
                    "WASM execution exceeded configured timeout"
                );
            }
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

    fn enforce_capabilities(&self, input: &serde_json::Value) -> Result<()> {
        let mut required: Vec<WasmCapability> = vec![];
        if let Some(explicit) = input
            .get("required_capabilities")
            .and_then(|v| v.as_array())
        {
            for cap in explicit.iter().filter_map(|v| v.as_str()) {
                match cap.to_ascii_lowercase().as_str() {
                    "readfilesystem" | "read_filesystem" | "filesystem_read" => {
                        if !required.contains(&WasmCapability::ReadFilesystem) {
                            required.push(WasmCapability::ReadFilesystem);
                        }
                    }
                    "writefilesystem" | "write_filesystem" | "filesystem_write" => {
                        if !required.contains(&WasmCapability::WriteFilesystem) {
                            required.push(WasmCapability::WriteFilesystem);
                        }
                    }
                    "network" => {
                        if !required.contains(&WasmCapability::Network) {
                            required.push(WasmCapability::Network);
                        }
                    }
                    "environment" | "env" => {
                        if !required.contains(&WasmCapability::Environment) {
                            required.push(WasmCapability::Environment);
                        }
                    }
                    _ => {}
                }
            }
        }

        let scan = input_capability_scan::scan_input_capabilities(input);
        if scan.requires_filesystem && !required.contains(&WasmCapability::ReadFilesystem) {
            required.push(WasmCapability::ReadFilesystem);
        }
        if scan.requires_network && !required.contains(&WasmCapability::Network) {
            required.push(WasmCapability::Network);
        }
        if scan.requires_environment && !required.contains(&WasmCapability::Environment) {
            required.push(WasmCapability::Environment);
        }

        for cap in required {
            if !self.has_capability(&cap) {
                return Err(IroncladError::Tool {
                    tool: self.config.name.clone(),
                    message: format!("missing required WASM capability: {:?}", cap),
                });
            }
        }
        Ok(())
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

    fn plugin_with_capabilities(capabilities: Vec<WasmCapability>) -> WasmPlugin {
        WasmPlugin::new(WasmPluginConfig {
            name: "scan-matrix".to_string(),
            wasm_path: PathBuf::from("/tmp/scan-matrix.wasm"),
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities,
        })
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
    fn capability_enforcement_blocks_network_access() {
        let dir = TempDir::new().unwrap();
        let config = test_config(dir.path(), "caps-enforced");
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();
        let err = plugin
            .execute(&serde_json::json!({"url": "https://example.com"}))
            .unwrap_err();
        assert!(err.to_string().contains("missing required WASM capability"));
    }

    #[test]
    fn capability_enforcement_allows_declared_network_access() {
        let dir = TempDir::new().unwrap();
        let mut config = test_config(dir.path(), "caps-network");
        config.capabilities = vec![WasmCapability::Network];
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();
        let result = plugin
            .execute(&serde_json::json!({"url": "https://example.com"}))
            .unwrap();
        assert_eq!(result["status"], "executed");
    }

    #[test]
    fn capability_enforcement_blocks_filesystem_access_for_path_keys() {
        let dir = TempDir::new().unwrap();
        let config = test_config(dir.path(), "caps-fs");
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();
        let err = plugin
            .execute(&serde_json::json!({"path": "src/main.rs"}))
            .unwrap_err();
        assert!(err.to_string().contains("missing required WASM capability"));
    }

    #[test]
    fn capability_enforcement_ignores_regex_backslashes_without_path_context() {
        let dir = TempDir::new().unwrap();
        let config = test_config(dir.path(), "caps-regex");
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();
        let result = plugin
            .execute(&serde_json::json!({"pattern": "\\d+\\w+\\s*"}))
            .unwrap();
        assert_eq!(result["status"], "executed");
    }

    #[test]
    fn capability_enforcement_matches_shared_scan_for_input_matrix() {
        let cases = vec![
            serde_json::json!({}),
            serde_json::json!({"endpoint": "https://example.com/v1"}),
            serde_json::json!({"socket": "wss://example.com/stream"}),
            serde_json::json!({"model": "openai/gpt-4o"}),
            serde_json::json!({"model": "/etc/passwd"}),
            serde_json::json!({"path": "src/main.rs"}),
            serde_json::json!({"input": "secrets/config.yaml"}),
            serde_json::json!({"pattern": "\\d+\\w+\\s*"}),
            serde_json::json!({"env_var": "SECRET_TOKEN"}),
        ];

        for input in cases {
            let scan = input_capability_scan::scan_input_capabilities(&input);
            let mut required_caps = Vec::new();
            if scan.requires_filesystem {
                required_caps.push(WasmCapability::ReadFilesystem);
            }
            if scan.requires_network {
                required_caps.push(WasmCapability::Network);
            }
            if scan.requires_environment {
                required_caps.push(WasmCapability::Environment);
            }

            let no_caps = plugin_with_capabilities(vec![]);
            let no_caps_ok = no_caps.enforce_capabilities(&input).is_ok();
            assert_eq!(
                no_caps_ok,
                required_caps.is_empty(),
                "no-capability behavior mismatch for input: {input}"
            );

            let with_required = plugin_with_capabilities(required_caps);
            assert!(
                with_required.enforce_capabilities(&input).is_ok(),
                "required-capability behavior mismatch for input: {input}"
            );
        }
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

    // ── Memory read/write path tests (BUG-079) ─────────────────────

    /// WAT module that exports memory, stores JSON `{"ok":true}` at offset 4096
    /// (beyond input write area), and returns (ptr=4096, len=11) from `process`.
    fn wasm_bytes_memory_json() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (data (i32.const 4096) "{\"ok\":true}")
                (func (export "process") (result i32 i32)
                    i32.const 4096  ;; ptr (beyond input write zone)
                    i32.const 11    ;; len of {"ok":true}
                )
            )"#,
        )
        .unwrap()
    }

    /// WAT module that exports memory, stores plain text at offset 4096
    /// (beyond input write area), and returns (ptr=4096, len=5).
    fn wasm_bytes_memory_text() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (data (i32.const 4096) "hello")
                (func (export "process") (result i32 i32)
                    i32.const 4096  ;; ptr (beyond input write zone)
                    i32.const 5     ;; len
                )
            )"#,
        )
        .unwrap()
    }

    /// WAT module returning (ptr, len) that goes out of bounds.
    fn wasm_bytes_memory_oob() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "process") (result i32 i32)
                    i32.const 0
                    i32.const 99999  ;; len far exceeds 1 page (65536 bytes)
                )
            )"#,
        )
        .unwrap()
    }

    /// WAT module with memory that returns a single i32 — exercises the
    /// "memory exists but not 2-value return" path.
    fn wasm_bytes_memory_single_return() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "process") (result i32)
                    i32.const 99
                )
            )"#,
        )
        .unwrap()
    }

    /// WAT module returning 3 values — exercises the multi-result fallback path.
    fn wasm_bytes_multi_return() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (func (export "process") (result i32 i32 i32)
                    i32.const 1
                    i32.const 2
                    i32.const 3
                )
            )"#,
        )
        .unwrap()
    }

    /// WAT module returning no values — exercises the 0-result path.
    fn wasm_bytes_void_return() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (func (export "process") nop)
            )"#,
        )
        .unwrap()
    }

    #[test]
    fn execute_memory_json_output() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("mem-json.wasm");
        fs::write(&wasm_path, wasm_bytes_memory_json()).unwrap();

        let config = WasmPluginConfig {
            name: "mem-json".into(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let result = plugin.execute(&serde_json::json!({})).unwrap();
        assert_eq!(result["status"], "executed");
        assert_eq!(result["plugin"], "mem-json");
        // The output should be parsed JSON: {"ok":true}
        assert_eq!(result["output"]["ok"], true);
    }

    #[test]
    fn execute_memory_text_output() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("mem-text.wasm");
        fs::write(&wasm_path, wasm_bytes_memory_text()).unwrap();

        let config = WasmPluginConfig {
            name: "mem-text".into(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let result = plugin.execute(&serde_json::json!({})).unwrap();
        assert_eq!(result["status"], "executed");
        assert_eq!(result["output"], "hello");
    }

    #[test]
    fn execute_memory_out_of_bounds() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("mem-oob.wasm");
        fs::write(&wasm_path, wasm_bytes_memory_oob()).unwrap();

        let config = WasmPluginConfig {
            name: "mem-oob".into(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let err = plugin.execute(&serde_json::json!({})).unwrap_err();
        assert!(
            err.to_string().contains("out of bounds"),
            "expected out-of-bounds error, got: {err}"
        );
    }

    #[test]
    fn execute_memory_single_return_with_exported_memory() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("mem-single.wasm");
        fs::write(&wasm_path, wasm_bytes_memory_single_return()).unwrap();

        let config = WasmPluginConfig {
            name: "mem-single".into(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let result = plugin.execute(&serde_json::json!({})).unwrap();
        assert_eq!(result["status"], "executed");
        // Single return value should go to "result", not "output"
        assert_eq!(result["result"], 99);
    }

    #[test]
    fn execute_multi_return_values() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("multi.wasm");
        fs::write(&wasm_path, wasm_bytes_multi_return()).unwrap();

        let config = WasmPluginConfig {
            name: "multi".into(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let result = plugin.execute(&serde_json::json!({})).unwrap();
        assert_eq!(result["status"], "executed");
        // No exported memory, 3 return values -> result is an array
        let arr = result["result"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0], 1);
        assert_eq!(arr[1], 2);
        assert_eq!(arr[2], 3);
    }

    #[test]
    fn execute_void_return() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("void.wasm");
        fs::write(&wasm_path, wasm_bytes_void_return()).unwrap();

        let config = WasmPluginConfig {
            name: "void".into(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let result = plugin.execute(&serde_json::json!({})).unwrap();
        assert_eq!(result["status"], "executed");
        // 0 return values -> result is null
        assert!(result["result"].is_null());
    }

    #[test]
    fn execute_writes_input_to_memory() {
        // Use a module with memory to confirm input write path is exercised
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("mem-write.wasm");
        fs::write(&wasm_path, wasm_bytes_memory_text()).unwrap();

        let config = WasmPluginConfig {
            name: "mem-write".into(),
            wasm_path,
            memory_limit_bytes: default_memory_limit(),
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        // Execute with large-ish input; this exercises the memory write path
        let big_input = serde_json::json!({"data": "x".repeat(100)});
        let result = plugin.execute(&big_input).unwrap();
        assert_eq!(result["status"], "executed");
    }

    // ── wasmer_value_to_json coverage ────────────────────────────────

    #[test]
    fn wasmer_value_to_json_i32() {
        let v = wasmer::Value::I32(42);
        assert_eq!(wasmer_value_to_json(&v), serde_json::json!(42));
    }

    #[test]
    fn wasmer_value_to_json_i64() {
        let v = wasmer::Value::I64(9_999_999_999);
        assert_eq!(
            wasmer_value_to_json(&v),
            serde_json::json!(9_999_999_999i64)
        );
    }

    #[test]
    fn wasmer_value_to_json_f32() {
        let v = wasmer::Value::F32(3.14);
        let json = wasmer_value_to_json(&v);
        assert!(json.is_number());
        let n = json.as_f64().unwrap();
        assert!((n - 3.14).abs() < 0.01);
    }

    #[test]
    fn wasmer_value_to_json_f64() {
        let v = wasmer::Value::F64(2.71828);
        let json = wasmer_value_to_json(&v);
        let n = json.as_f64().unwrap();
        assert!((n - 2.71828).abs() < 0.001);
    }

    // ── enforce_capabilities explicit capability parsing ─────────────

    #[test]
    fn enforce_capabilities_explicit_read_filesystem_aliases() {
        let dir = TempDir::new().unwrap();
        let mut config = test_config(dir.path(), "cap-explicit");
        config.capabilities = vec![WasmCapability::ReadFilesystem];
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        for alias in ["readfilesystem", "read_filesystem", "filesystem_read"] {
            let input = serde_json::json!({"required_capabilities": [alias]});
            assert!(
                plugin.execute(&input).is_ok(),
                "ReadFilesystem alias '{alias}' should be granted"
            );
        }
    }

    #[test]
    fn enforce_capabilities_explicit_write_filesystem_aliases() {
        let dir = TempDir::new().unwrap();
        let mut config = test_config(dir.path(), "cap-write");
        config.capabilities = vec![WasmCapability::WriteFilesystem];
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        for alias in ["writefilesystem", "write_filesystem", "filesystem_write"] {
            let input = serde_json::json!({"required_capabilities": [alias]});
            assert!(
                plugin.execute(&input).is_ok(),
                "WriteFilesystem alias '{alias}' should be granted"
            );
        }
    }

    #[test]
    fn enforce_capabilities_explicit_network() {
        let dir = TempDir::new().unwrap();
        let mut config = test_config(dir.path(), "cap-net");
        config.capabilities = vec![WasmCapability::Network];
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let input = serde_json::json!({"required_capabilities": ["network"]});
        assert!(plugin.execute(&input).is_ok());
    }

    #[test]
    fn enforce_capabilities_explicit_environment_aliases() {
        let dir = TempDir::new().unwrap();
        let mut config = test_config(dir.path(), "cap-env");
        config.capabilities = vec![WasmCapability::Environment];
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        for alias in ["environment", "env"] {
            let input = serde_json::json!({"required_capabilities": [alias]});
            assert!(
                plugin.execute(&input).is_ok(),
                "Environment alias '{alias}' should be granted"
            );
        }
    }

    #[test]
    fn enforce_capabilities_explicit_unknown_ignored() {
        let dir = TempDir::new().unwrap();
        let config = test_config(dir.path(), "cap-unknown");
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        // Unknown capabilities in the array are silently ignored
        let input = serde_json::json!({"required_capabilities": ["nonexistent_capability"]});
        assert!(plugin.execute(&input).is_ok());
    }

    #[test]
    fn enforce_capabilities_explicit_denied_without_grant() {
        let dir = TempDir::new().unwrap();
        // No capabilities granted
        let config = test_config(dir.path(), "cap-deny");
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        let input = serde_json::json!({"required_capabilities": ["network"]});
        let err = plugin.execute(&input).unwrap_err();
        assert!(err.to_string().contains("missing required WASM capability"));
    }

    #[test]
    fn enforce_capabilities_deduplicates() {
        let dir = TempDir::new().unwrap();
        let mut config = test_config(dir.path(), "cap-dedup");
        config.capabilities = vec![WasmCapability::Network];
        let mut plugin = WasmPlugin::new(config);
        plugin.load().unwrap();

        // Same capability requested via two aliases — should not cause double deny
        let input = serde_json::json!({
            "required_capabilities": ["network", "network"],
            "url": "https://example.com"
        });
        assert!(plugin.execute(&input).is_ok());
    }

    // ── Memory limit enforcement on load ─────────────────────────────

    #[test]
    fn load_rejects_oversized_memory() {
        let dir = TempDir::new().unwrap();
        // Module requests 256 pages = 16 MB of memory
        let wasm = wat::parse_str(
            r#"(module (memory (export "memory") 256) (func (export "process") nop))"#,
        )
        .unwrap();
        let wasm_path = dir.path().join("big-mem.wasm");
        fs::write(&wasm_path, wasm).unwrap();

        let config = WasmPluginConfig {
            name: "big-mem".into(),
            wasm_path,
            memory_limit_bytes: 1024 * 1024, // 1 MB limit
            execution_timeout_ms: default_execution_timeout_ms(),
            capabilities: vec![],
        };
        let mut plugin = WasmPlugin::new(config);
        let err = plugin.load().unwrap_err();
        assert!(
            err.to_string().contains("exceeds limit"),
            "expected memory limit error, got: {err}"
        );
    }

    #[test]
    fn debug_impl_for_plugin() {
        let config = WasmPluginConfig {
            name: "debug-test".into(),
            wasm_path: PathBuf::from("/tmp/debug.wasm"),
            memory_limit_bytes: 1024,
            execution_timeout_ms: 5000,
            capabilities: vec![],
        };
        let plugin = WasmPlugin::new(config);
        let dbg = format!("{:?}", plugin);
        assert!(dbg.contains("debug-test"));
        assert!(dbg.contains("has_engine"));
    }
}
