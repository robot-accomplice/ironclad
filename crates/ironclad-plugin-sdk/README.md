# ironclad-plugin-sdk

> **Version 0.5.0**

Plugin system for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime. Provides the `Plugin` trait, TOML manifest parsing, script execution with declared-capability checks, plugin packaging (`.ic.zip` archives), remote catalog discovery, and a plugin registry with directory discovery and lifecycle management.

## Key Types & Traits

| Type | Module | Description |
|------|--------|-------------|
| `Plugin` | `lib` | Async trait: `name()`, `version()`, `tools()`, `init()`, `execute_tool()`, `shutdown()` |
| `ToolDef` | `lib` | Tool definition (name, description, JSON Schema parameters) |
| `ToolResult` | `lib` | Tool execution result (success, output, metadata) |
| `PluginStatus` | `lib` | Loaded / Active / Disabled / Error |
| `PluginManifest` | `manifest` | TOML manifest parsing, validation, requirements, companion skills |
| `PluginRegistry` | `registry` | Plugin discovery, registration, and lifecycle |
| `PluginLoader` | `loader` | Load plugins from directory with auto-discovery |
| `ScriptPlugin` | `script` | Script-based plugin execution |
| `PackResult` / `UnpackResult` | `archive` | Pack/unpack `.ic.zip` archives with SHA-256 verification |
| `PluginCatalog` | `catalog` | Remote plugin catalog search and lookup |

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

- **[Plugin Authoring Guide](../../docs/PLUGIN_AUTHORING.md)** — Create, test, package, and distribute plugins
- API docs are available on [docs.rs](https://docs.rs/ironclad-plugin-sdk)

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
