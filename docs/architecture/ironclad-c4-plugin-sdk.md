# C4 Level 3: Component Diagram -- ironclad-plugin-sdk

*SDK for defining and running plugins that expose tools to the agent. Plugins are loaded from a directory (one `plugin.toml` per plugin), registered in a central registry, and executed on demand. The built-in `ScriptPlugin` runs external scripts (e.g. `.gosh`, `.py`, `.sh`) per tool. The SDK also provides packaging (`.ic.zip` archives), checksum verification, and remote catalog discovery for plugin distribution.*

---

## Component Diagram

```mermaid
flowchart TB
    subgraph IroncladPluginSdk ["ironclad-plugin-sdk"]
        TRAIT["Plugin trait<br/>name, version, tools,<br/>init, execute_tool, shutdown"]
        TOOL_DEF["ToolDef<br/>name, description, parameters,<br/>risk_level: RiskLevel (default Caution),<br/>permissions: Vec&lt;String&gt;"]
        TOOL_RESULT["ToolResult<br/>success, output, metadata"]
        MANIFEST["PluginManifest<br/>name, version, permissions,<br/>tools (ManifestToolDef)"]
        REGISTRY["PluginRegistry<br/>register, init_all,<br/>execute_tool, find_tool,<br/>list_plugins, disable/enable"]
        LOADER["loader.rs<br/>discover_plugins(dir)<br/>plugin.toml per directory"]
        SCRIPT["script.rs<br/>ScriptPlugin<br/>executes external scripts"]
        ARCHIVE["archive.rs<br/>pack/unpack .ic.zip<br/>SHA-256 verification"]
        CATALOG["catalog.rs<br/>PluginCatalog<br/>remote search + lookup"]
    end

    subgraph ManifestDetail ["PluginManifest"]
        M_FROM_FILE["from_file(path)"]
        M_FROM_STR["from_str(toml)"]
        M_VALIDATE["validate()"]
    end

    subgraph RegistryDetail ["PluginRegistry"]
        R_ALLOW["allow_list / deny_list<br/>is_allowed(name)"]
        R_REG["register(Box<dyn Plugin>)"]
        R_INIT["init_all() → errors"]
        R_EXEC["execute_tool(name, input)"]
        R_LIST["list_plugins(), list_all_tools()"]
        R_TOGGLE["disable_plugin(name), enable_plugin(name)"]
    end

    subgraph LoaderDetail ["loader.rs"]
        DISCOVER["discover_plugins(plugins_dir)<br/>Scan dirs for plugin.toml<br/>Return Vec<DiscoveredPlugin>"]
        SORT["Sort by plugin name"]
    end

    subgraph ScriptDetail ["ScriptPlugin"]
        SCRIPT_MAP["tools → script files<br/>(.gosh, .go, .sh, .py, .rb, .js)"]
        SCRIPT_RUN["execute_tool: spawn process<br/>IRONCLAD_INPUT, IRONCLAD_TOOL,<br/>IRONCLAD_PLUGIN env"]
        SCRIPT_TIMEOUT["Default 30s timeout"]
    end

    TRAIT --> REGISTRY
    MANIFEST --> LOADER
    MANIFEST --> SCRIPT
    TOOL_DEF --> TRAIT
    TOOL_RESULT --> TRAIT
    LOADER --> DISCOVER
    SCRIPT --> TRAIT
    REGISTRY --> R_REG
```

## How Plugins Are Loaded, Registered, and Executed

1. **Discovery**  
   The server (or bootstrap code) calls `discover_plugins(plugins_dir)`. Each subdirectory that contains a `plugin.toml` is parsed into a `PluginManifest`; invalid manifests are skipped with a warning. Results are sorted by plugin name.

2. **Registration**  
   For each `DiscoveredPlugin`, the loader typically builds a `ScriptPlugin::new(manifest, dir)` and passes it to `PluginRegistry::register(Box::new(plugin))`. The registry checks `is_allowed(name)` (allow_list/deny_list); if denied, registration fails. Otherwise the plugin is stored with status `Loaded`.

3. **Initialization**  
   `PluginRegistry::init_all()` is called once after all plugins are registered. Each plugin’s `init()` is run; on success the status becomes `Active`, on failure `Error`. Init errors are collected and returned; the registry does not remove failed plugins.

4. **Execution**  
   When the agent (or API) invokes a tool by name, the server calls `PluginRegistry::execute_tool(tool_name, input)`. The registry finds the first *Active* plugin that declares that tool and calls `plugin.execute_tool(tool_name, input)`. For `ScriptPlugin`, this runs the corresponding script with `IRONCLAD_INPUT` (JSON), `IRONCLAD_TOOL`, and `IRONCLAD_PLUGIN` set; stdout is captured as the result output.

5. **Toggle**  
   `disable_plugin(name)` / `enable_plugin(name)` set status to `Disabled` or `Active`. Disabled plugins are skipped for `execute_tool` and `list_all_tools`.

## Plugin Development Sequences

### 1) Install + Companion Skill Deployment

```mermaid
sequenceDiagram
    participant Op as Operator
    participant CLI as "ironclad plugins install"
    participant FS as Filesystem
    participant M as PluginManifest

    Op->>CLI: install /path/to/plugin
    CLI->>M: parse + validate plugin.toml
    CLI->>M: check_requirements()
    M-->>CLI: required deps present
    CLI->>FS: copy plugin dir to ~/.ironclad/plugins/<name>
    CLI->>FS: copy companion skills to ~/.ironclad/skills/<plugin>--<skill>.md
    CLI-->>Op: install success + restart required
```

### 2) Server Startup Registration Path

```mermaid
sequenceDiagram
    participant S as Server Boot
    participant L as loader::discover_plugins
    participant V as manifest.vet()
    participant R as PluginRegistry
    participant P as ScriptPlugin

    S->>L: scan plugins dir
    L-->>S: DiscoveredPlugin[]
    loop each discovered plugin
        S->>V: integrity + requirements vet
        alt vet has errors
            S-->>S: skip plugin, log warnings/errors
        else vet ok
            S->>P: ScriptPlugin::new(manifest, dir)
            S->>R: register(plugin)
        end
    end
    S->>R: init_all()
    R-->>S: active/error status per plugin
```

### 3) Tool Execution + Permission Gates

```mermaid
sequenceDiagram
    participant API as /api/plugins/{name}/execute/{tool}
    participant PE as PolicyEngine
    participant R as PluginRegistry
    participant SP as ScriptPlugin
    participant Proc as Script Process

    API->>R: find_tool(tool)
    API->>API: infer required permissions from input scan
    API->>PE: evaluate risk policy (authority=External)
    alt denied by policy/permissions
        API-->>API: return 403
    else allowed
        API->>R: execute_tool(tool, input)
        R->>SP: execute_tool(...)
        SP->>SP: enforce declared permissions (process + input-derived caps)
        SP->>Proc: spawn script/interpreter with IRONCLAD_* env
        Proc-->>SP: stdout/stderr/exit
        SP-->>R: ToolResult
        R-->>API: ToolResult
    end
```

## Types

| Type | Location | Purpose |
|------|----------|---------|
| `Plugin` | `lib.rs` | Async trait: name, version, tools(), init(), execute_tool(), shutdown() |
| `ToolDef` | `lib.rs` | name, description, parameters (JSON schema), risk_level (RiskLevel, default Caution), permissions (Vec\<String\>) |
| `ToolResult` | `lib.rs` | success, output, optional metadata |
| `PluginStatus` | `lib.rs` | Loaded, Active, Disabled, Error |
| `PluginManifest` | `manifest.rs` | TOML: name, version, description, author, permissions, requirements, companion_skills, tools |
| `ManifestToolDef` | `manifest.rs` | name, description, dangerous |
| `Requirement` | `manifest.rs` | External dependency: name, command, install_hint, optional |
| `PackResult` | `archive.rs` | Result of packing: archive_path, sha256, name, version, file_count |
| `UnpackResult` | `archive.rs` | Result of unpacking: dest_dir, manifest, sha256, file_count |
| `ArchiveError` | `archive.rs` | IO, ZIP, manifest, checksum, path traversal errors |
| `PluginCatalog` | `catalog.rs` | Remote catalog: search(query), find(name) |
| `PluginCatalogEntry` | `catalog.rs` | Catalog entry: name, version, sha256, path, tier |
| `PluginRegistry` | `registry.rs` | In-memory map of plugins, allow/deny lists, init/execute/list/disable/enable |
| `PluginInfo` | `registry.rs` | name, version, status, tools (for API listing) |
| `DiscoveredPlugin` | `loader.rs` | manifest + directory path |
| `ScriptPlugin` | `script.rs` | Plugin impl that runs scripts per tool; interpreters: gosh, go run, python3, ruby, node, sh |

## Dependencies

**External crates**: `async-trait`, `serde`, `serde_json`, `tokio`, `tracing`, `toml`, `zip`, `sha2`, `hex`, `thiserror`

**Internal crates**: `ironclad-core` (Result, IroncladError, config)

**Depended on by**: `ironclad-server` (wires discovery, registry, and `/api/plugins/*`). Note: `ironclad-agent` does NOT directly depend on `ironclad-plugin-sdk`; plugin-to-agent integration is server-mediated.
