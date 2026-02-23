# ironclad-plugin-sdk

> **Version 0.5.0**

Plugin system for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime. Provides the `Plugin` trait, TOML manifest parsing, sandboxed script execution, and a plugin registry with auto-discovery and hot-reload.

## Key Types & Traits

| Type | Module | Description |
|------|--------|-------------|
| `Plugin` | `lib` | Async trait: `name()`, `version()`, `tools()`, `init()`, `execute_tool()`, `shutdown()` |
| `ToolDef` | `lib` | Tool definition (name, description, JSON Schema parameters) |
| `ToolResult` | `lib` | Tool execution result (success, output, metadata) |
| `PluginStatus` | `lib` | Loaded / Active / Disabled / Error |
| `PluginManifest` | `manifest` | TOML manifest parsing and validation |
| `PluginRegistry` | `registry` | Plugin discovery, registration, and lifecycle |
| `PluginLoader` | `loader` | Load plugins from directory with auto-discovery |
| `ScriptPlugin` | `script` | Script-based plugin execution |

## Usage

```toml
[dependencies]
ironclad-plugin-sdk = "0.5"
```

```rust
use ironclad_plugin_sdk::{Plugin, ToolDef, ToolResult, PluginStatus};

// Implement the Plugin trait for custom plugins
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-plugin-sdk).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
