# Plugin Authoring Guide

> Create, test, package, and distribute plugins for the Ironclad agent runtime.

---

## Overview

An Ironclad **plugin** is a directory containing:

1. A `plugin.toml` manifest declaring the plugin's identity, tools, requirements, and permissions
2. One or more **script files** (`.sh`, `.py`, `.gosh`, `.rb`, `.js`, or extensionless with shebang) — each implements a tool
3. Optional **companion skills** (`.md` files) that teach the agent how to use the plugin's tools effectively
4. Optional supporting files (guides, templates, etc.)

Plugins are loaded from `~/.ironclad/plugins/<plugin-name>/` at runtime and executed as sandboxed subprocesses.

---

## Quick Start

```bash
# 1. Create the plugin directory
mkdir -p my-plugin/skills

# 2. Write the manifest
cat > my-plugin/plugin.toml << 'EOF'
name = "my-plugin"
version = "0.1.0"
description = "A sample plugin that greets users"
author = "Your Name"
permissions = []

[[tools]]
name = "greet"
description = "Return a greeting for the given name"
EOF

# 3. Write the tool script
cat > my-plugin/greet.sh << 'SCRIPT'
#!/bin/sh
# Input arrives as JSON in $IRONCLAD_INPUT
name=$(echo "$IRONCLAD_INPUT" | jq -r '.name // "world"')
echo "Hello, ${name}!"
SCRIPT
chmod +x my-plugin/greet.sh

# 4. Pack into an archive
ironclad plugins pack my-plugin/

# 5. Install locally for testing
ironclad plugins install my-plugin/
```

---

## The `plugin.toml` Manifest

Every plugin must have a `plugin.toml` at the root of its directory. This is the single source of truth for the plugin's identity and capabilities.

### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Plugin identifier. Alphanumeric, hyphens, underscores only. Max 128 chars. |
| `version` | `String` | Semver version (e.g., `"1.2.3"`). Max 64 chars. |

### Optional Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `description` | `String` | `""` | Human-readable summary. |
| `author` | `String` | `""` | Author or organization. |
| `permissions` | `[String]` | `[]` | Declared capabilities (see [Permissions](#permissions)). |
| `timeout_seconds` | `u64` | `30` | Per-tool script execution timeout. |
| `requirements` | `[[requirements]]` | `[]` | External dependencies (see [Requirements](#requirements)). |
| `companion_skills` | `[String]` | `[]` | Relative paths to companion skill files (see [Companion Skills](#companion-skills)). |
| `tools` | `[[tools]]` | `[]` | Tool definitions (see [Tools](#tools)). |

### Full Example

```toml
name = "claude-code"
version = "0.1.0"
description = "Delegate complex coding tasks to Claude Code CLI"
author = "Ironclad"
permissions = ["filesystem", "process"]
timeout_seconds = 300
companion_skills = ["skills/claude-code.md"]

[[requirements]]
name = "Claude Code CLI"
command = "claude"
install_hint = "Install Claude Code: https://docs.anthropic.com/en/docs/claude-code"

[[requirements]]
name = "jq"
command = "jq"
install_hint = "Install jq: https://jqlang.github.io/jq/download/"
optional = true

[[tools]]
name = "claude-code"
description = "Invoke Claude Code CLI to perform a coding task"
dangerous = true
permissions = ["filesystem", "process"]
```

---

## Tools

Each `[[tools]]` entry declares a tool the agent can invoke. The tool name maps to a script file in the plugin directory.

### Tool Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | `String` | *required* | Tool identifier. Must match a script file name (minus extension). |
| `description` | `String` | *required* | What the tool does. The agent uses this to decide when to invoke it. |
| `dangerous` | `bool` | `false` | If `true`, requires explicit user confirmation before execution. |
| `permissions` | `[String]` | `[]` | Per-tool permissions. If empty, inherits from the plugin-level `permissions`. |

### Script Resolution

When a tool is invoked, the runtime searches for a matching script in this order:

1. `<plugin-dir>/<tool-name>.gosh`
2. `<plugin-dir>/<tool-name>.go`
3. `<plugin-dir>/<tool-name>.sh`
4. `<plugin-dir>/<tool-name>.py`
5. `<plugin-dir>/<tool-name>.rb`
6. `<plugin-dir>/<tool-name>.js`
7. `<plugin-dir>/<tool-name>` (extensionless — must have a valid shebang)

### Script Interface

Scripts receive input and produce output through these conventions:

| Channel | Direction | Format |
|---------|-----------|--------|
| `IRONCLAD_INPUT` env var | Input | JSON object |
| `stdout` | Output | Text (returned as `ToolResult.output`) |
| `stderr` | Diagnostic | Logged at `warn` level; not returned to the agent |
| Exit code | Status | `0` = success; non-zero = failure |

**Stdout limit**: 10 MB. Output exceeding this is truncated.

### Example Script (Shell)

```bash
#!/bin/sh
set -e

# Parse input
query=$(echo "$IRONCLAD_INPUT" | jq -r '.query // empty')
if [ -z "$query" ]; then
  echo "Error: 'query' parameter is required" >&2
  exit 1
fi

# Do work
result=$(curl -s "https://api.example.com/search?q=${query}")

# Output result
echo "$result"
```

### Example Script (Python)

```python
#!/usr/bin/env python3
import json, os, sys

inp = json.loads(os.environ.get("IRONCLAD_INPUT", "{}"))
name = inp.get("name", "world")

print(f"Hello, {name}!")
```

---

## Permissions

Permissions declare what capabilities a plugin needs. They are checked at install time and at runtime against the server's allow/deny lists.

| Permission | Description |
|------------|-------------|
| `filesystem` | Read/write files on the host |
| `process` | Spawn subprocesses |
| `network` | Make outbound network requests |

The runtime performs **input capability scanning** on each tool invocation, checking that the tool's declared permissions cover the operations implied by its input. Undeclared capabilities cause the invocation to be rejected.

> **Note**: Permissions are currently policy-enforced (allow-list + input scanning), not syscall-sandboxed. Full sandboxing is on the roadmap.

---

## Requirements

Requirements declare external dependencies that must be present on the host for the plugin to function. They are checked at install time and during periodic health checks.

```toml
[[requirements]]
name = "Claude Code CLI"    # Human-readable name
command = "claude"           # Binary checked via `command -v`
install_hint = "https://..."  # URL or instructions shown on failure
optional = false             # false = blocks install; true = warns only
```

### Behavior

| Stage | Required Dependency Missing | Optional Dependency Missing |
|-------|----------------------------|-----------------------------|
| Install | Installation blocked with error | Warning printed, install continues |
| Health check (`ironclad mechanic`) | Reported as error | Reported as warning |
| Runtime | Tool invocation may fail | Degraded functionality |

---

## Companion Skills

Companion skills are markdown files bundled within the plugin directory that teach the agent **when and how** to use the plugin's tools. They follow the same format as standalone Ironclad skills.

```toml
companion_skills = ["skills/claude-code.md"]
```

The path is relative to the plugin root. On installation, companion skills are:

1. Copied to `~/.ironclad/skills/` with a namespaced filename: `plugin--<plugin-name>--<original-name>`
2. Automatically removed when the plugin is uninstalled

### Skill File Format

Companion skills use YAML frontmatter + markdown body:

```markdown
---
name: my-plugin-guide
description: Guide the agent on using the my-plugin tools
triggers:
  keywords: [my-plugin, my tool, do the thing]
  regex_patterns:
    - "use my.?plugin"
priority: 5
---

# When to use this tool

Use the `my-tool` tool when the user asks to ...

# How to invoke

Call the tool with:
- `query` (required): The search query
- `limit` (optional): Maximum results (default: 10)

# Examples

...
```

---

## Packaging & Distribution

### Pack into an Archive

```bash
ironclad plugins pack <plugin-dir> [--output <dir>]
```

This:

1. **Vets** the plugin (validates manifest, checks script shebangs and permissions)
2. Creates `<name>-<version>.ic.zip` (standard ZIP with Deflate compression)
3. Prints the SHA-256 checksum for the archive

The `.ic.zip` format is a standard ZIP archive with one requirement: `plugin.toml` must be at the root.

### Install from Different Sources

```bash
# From a local directory (dev mode)
ironclad plugins install ./my-plugin

# From a .ic.zip archive
ironclad plugins install my-plugin-0.1.0.ic.zip

# From the online catalog
ironclad plugins install my-plugin
```

Source detection:

| Input Pattern | Source Type | Description |
|---------------|------------|-------------|
| Contains `/` or `\` | Directory | Local directory install (dev mode) |
| Ends with `.zip` | Archive | Local `.ic.zip` install with staging + verification |
| Bare name | Catalog | Remote download from the Ironclad plugin catalog |

### Search the Catalog

```bash
ironclad plugins search <query>
```

Searches the remote plugin catalog by name, description, or author.

---

## Plugin Lifecycle

### Installation Flow

```
detect source → fetch/extract → vet manifest → check requirements
    → confirm with user → deploy to plugins dir → install companion skills
```

1. **Detection**: Source string is classified as directory, archive, or catalog name
2. **Staging**: Archives are extracted to `~/.ironclad/staging/` for verification
3. **Vetting**: Manifest validation, script shebang checks, permission declarations
4. **Requirements**: Required dependencies are checked via `command -v`; missing required deps block installation
5. **Confirmation**: User is prompted with a summary before files are deployed
6. **Deployment**: Plugin files are copied to `~/.ironclad/plugins/<name>/`
7. **Companion Skills**: Any declared companion skills are installed to `~/.ironclad/skills/`

### Runtime

At server startup, plugins are:

1. Discovered from `~/.ironclad/plugins/`
2. Vetted (manifest + script validation)
3. Registered in the `PluginRegistry`
4. Initialized (the `init()` method is called)

When the agent invokes a tool:

1. The tool is resolved from the registry
2. Input capability scanning checks permissions
3. The script is executed as a subprocess with `IRONCLAD_INPUT`
4. Output is captured and returned as a `ToolResult`

### Health Checks

```bash
ironclad mechanic --fix
```

The mechanic command checks:

- Plugin directories have valid `plugin.toml`
- Declared tools have matching script files
- Script files have valid shebangs and are executable
- Requirements are present on the host
- Companion skills are properly installed

With `--fix`, it repairs common issues like missing execute permissions and missing companion skills.

---

## Development Workflow

### Local Development

1. Create your plugin directory with `plugin.toml` and scripts
2. Install in dev mode: `ironclad plugins install ./my-plugin`
3. Test by talking to the agent and asking it to use your tool
4. Edit scripts in place (they're copied to `~/.ironclad/plugins/`)
5. Restart the server to pick up manifest changes

### Testing Tips

- Set `timeout_seconds` high during development to avoid premature kills
- Write scripts defensively — check for required input fields
- Use `stderr` for diagnostic output (logged but not returned to the agent)
- Exit with non-zero codes on failure so the agent can handle errors
- Test with `echo '{"param":"value"}' | IRONCLAD_INPUT='{"param":"value"}' ./my-script.sh`

### Pre-Distribution Checklist

- [ ] `plugin.toml` validates cleanly (`ironclad plugins pack` will verify)
- [ ] All tools have matching script files
- [ ] Scripts have proper shebangs (`#!/bin/sh`, `#!/usr/bin/env python3`, etc.)
- [ ] Scripts are executable (`chmod +x`)
- [ ] Requirements list all external dependencies
- [ ] Permissions accurately reflect what the plugin does
- [ ] Description is clear and helps the agent decide when to use each tool
- [ ] Companion skill (if any) teaches the agent how to construct tool inputs

---

## Publishing to the Catalog

To submit a plugin to the official Ironclad plugin catalog:

1. Pack your plugin: `ironclad plugins pack ./my-plugin`
2. Note the SHA-256 checksum printed by the pack command
3. Open a pull request to the [ironclad registry](https://github.com/robot-accomplice/ironclad) adding:
   - Your `.ic.zip` archive to `registry/plugins/`
   - A catalog entry in `registry/manifest.json` under `packs.plugins.catalog`

### Catalog Entry Format

```json
{
  "name": "my-plugin",
  "version": "0.1.0",
  "description": "What the plugin does",
  "author": "Your Name",
  "sha256": "<sha256-from-pack-command>",
  "path": "plugins/my-plugin-0.1.0.ic.zip",
  "min_version": "0.9.4",
  "tier": "community"
}
```

| Tier | Description |
|------|-------------|
| `official` | Maintained by the Ironclad team |
| `community` | Community-contributed, reviewed |
| `third-party` | Third-party, minimal review |

---

## Name Constraints

| Element | Rules |
|---------|-------|
| Plugin name | Alphanumeric, `-`, `_` only. Max 128 chars. No `/`, `\`, `..`, or null bytes. |
| Plugin version | Non-empty. Max 64 chars. |
| Tool name | Alphanumeric, `-`, `_` only. Max 64 chars. |
| Permission name | Alphanumeric, `-`, `_` only. Max 64 chars. |
| Companion skill path | Must be relative (no leading `/`), must end with `.md`, no `..` traversal. |
