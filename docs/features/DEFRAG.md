# Feature: `ironclad defrag`

*Workspace coherence scanner and auto-fixer.*

---

## Problem

Ironclad workspaces accumulate entropy. Skill imports bring stale references. Platform migrations leave behind old paths, CLI commands, and brand strings. Config files drift from documentation. Runtime artifacts land in wrong directories. Over time the workspace degrades — not broken enough to error, but incoherent enough to confuse the agent, the operator, and the updater.

Today this requires manual grep-and-sed archaeology across dozens of files. The `mechanic` command checks runtime health (database, providers, cron jobs) but has no awareness of workspace *content* coherence.

## Solution

A new top-level CLI command — `ironclad defrag` — that scans the workspace for structural incoherence and optionally fixes it. Sits alongside `mechanic` under the **Operations** heading: `mechanic` owns runtime health, `defrag` owns content health.

```
ironclad defrag              # scan and report
ironclad defrag --fix        # scan and auto-repair
ironclad defrag --fix --yes  # non-interactive (for scheduled runs)
ironclad defrag --pass refs  # run only the reference pass
ironclad defrag --json       # machine-readable output
```

## CLI Definition

```rust
/// Scan workspace for stale references, config drift, and orphaned artifacts
#[command(next_help_heading = "Operations")]
Defrag {
    /// Auto-fix discovered issues (default: report only)
    #[arg(long, short = 'f')]
    fix: bool,
    /// Skip confirmation prompts (requires --fix)
    #[arg(long)]
    yes: bool,
    /// Run specific passes only (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pass: Vec<String>,
    /// Emit machine-readable JSON findings
    #[arg(long)]
    json: bool,
    /// Verbose output (show every file touched)
    #[arg(long, short)]
    verbose: bool,
},
```

## Defrag Passes

Each pass is an independent scanner that produces findings. Passes run in order; each can be invoked individually via `--pass <name>`.

### Pass 1: `refs` — Dead Reference Elimination

Scan all `.md`, `.sh`, `.py`, `.js`, `.toml`, `.json` files in the workspace and skills directories for references that don't resolve.

**Checks:**
- Filesystem paths that don't exist (e.g., `<old-workspace-path>` after migration)
- References to deleted skill directories
- CLI commands for platforms that aren't installed
- URLs pointing at localhost ports that don't match config

**Fix strategy:** Pattern-based replacement using a migration map. The map is built from:
1. **Hardcoded known migrations** (legacy→ironclad paths, CLI commands)
2. **Config-derived mappings** (read `ironclad.toml` to know current paths, ports, model names)
3. **User-supplied overrides** via `[defrag.mappings]` in config

**Derived from:** Phase 3-6 of the manual cleanup (271 legacy→ironclad replacements across 40 files).

### Pass 2: `drift` — Config Drift Detection

Compare what documentation/skill files claim vs. what the actual config files say.

**Checks:**
- Model names in SKILL.md tier tables vs. `ironclad.toml [models]`
- Fallback chain in docs vs. actual config
- Provider names/URLs in docs vs. provider config
- Port numbers in docs vs. `[server].port`
- Database paths in docs vs. `[database].path`
- Identity fields (`generated_by`, agent name) inconsistencies

**Fix strategy:** Update the documentation to match config (config is authoritative). Uses structured TOML parsing to extract ground truth, then pattern-matches against prose in `.md` files.

**Derived from:** Phase 7 (kimi-k2.5→kimi-k2-turbo-preview version drift, fallback chain mismatch).

### Pass 3: `artifacts` — Orphaned Artifact Cleanup

Find files that shouldn't be where they are.

**Checks:**
- Log files (`*.log`) in skills directories
- Duplicate config files (byte-identical or subset of `ironclad.toml`)
- Orphan flat files in skill parent directory (skills should be directories)
- Empty directories left after deletions
- `__pycache__`, `.pyc`, `node_modules` in skill dirs
- Backup files (`*.bak`, `*.orig`, `*~`)

**Fix strategy:** Delete with confirmation (or `--yes` for batch). Never deletes anything in `.git/`.

**Derived from:** Phase 1 (gateway.log in skills/, duplicate duncan.toml, orphan ghola.md).

### Pass 4: `stale` — Stale State Cleanup

Find updater/cache/index entries pointing at things that no longer exist.

**Checks:**
- `update_state.json` skill hashes referencing deleted files
- Cached embeddings for deleted memories
- Session references to removed agents
- Schedule entries for skills that don't exist

**Fix strategy:** Remove the stale entries. For `update_state.json`, clear hashes for nonexistent files. For database entries, mark as expired rather than deleting.

**Derived from:** Phase 2a (19 ghost skill hashes in update_state.json).

### Pass 5: `identity` — Identity Coherence

Verify that identity strings are consistent across the workspace.

**Checks:**
- Agent name in `ironclad.toml` vs. OS.toml vs. skill references
- `generated_by` fields that reference old platforms
- Brand/product name consistency (no stale "Legacy" or other platform names)
- Workspace path consistency in scripts vs. actual path

**Fix strategy:** Replace with canonical values from `ironclad.toml` and `OS.toml`.

**Derived from:** Phase 2b (OS.toml `generated_by = "legacy-migration"` → `"ironclad"`).

### Pass 6: `scripts` — Script Health

Validate that executable scripts in skills reference real commands and paths.

**Checks:**
- Shebang lines point to installed interpreters
- Hardcoded paths exist on the current system
- CLI commands referenced in scripts are available (`which` check)
- Config file format assumptions match reality (e.g., script uses `jq` to edit `.json` but config is now `.toml`)

**Fix strategy:** Report only — script fixes require manual review. Flag with severity.

**Derived from:** Phase 5 (model-switch.sh referencing legacy.json format, collect_verified.sh calling `has_cmd legacy`).

## Output Format

### Default (human-readable)

```
  DEFRAG — Workspace Coherence Scan
  ══════════════════════════════════

  refs ·····················  4 findings (3 fixable)
  drift ····················  2 findings (2 fixable)
  artifacts ················  1 finding  (1 fixable)
  stale ····················  0 findings
  identity ·················  1 finding  (1 fixable)
  scripts ··················  2 findings (0 fixable, manual review)
  ──────────────────────────────────
  Total: 10 findings, 7 auto-fixable

  Run `ironclad defrag --fix` to repair.
```

### JSON (`--json`)

```json
{
  "passes": {
    "refs": {
      "findings": [
        {
          "file": "skills/cron-efficiency/SKILL.md",
          "line": 42,
          "severity": "warn",
          "message": "Reference to an old cron/jobs.json path — path does not exist",
          "fixable": true,
          "fix": "Replace with `ironclad schedule list`"
        }
      ]
    }
  },
  "summary": { "total": 10, "fixable": 7 }
}
```

## Trigger Points

`defrag` is not only a manual command — it hooks into the lifecycle:

| Trigger | Behavior | Flag |
|---------|----------|------|
| `ironclad defrag` | Full scan, report only | Manual |
| `ironclad defrag --fix` | Full scan + auto-fix | Manual |
| After `ironclad migrate import` | Auto-run defrag, prompt to fix | Automatic |
| After `ironclad update` (if skills changed) | Run `refs` + `stale` passes | Automatic |
| After `ironclad skills install <skill>` | Run `refs` + `artifacts` on new skill only | Automatic |
| Heartbeat (if `defrag.schedule` set) | Lightweight scan (`stale` + `artifacts`) | Scheduled |
| `ironclad mechanic` | Show defrag summary in mechanic output | Cross-reference |

## Configuration

```toml
[defrag]
# Auto-run after migrations (default: true)
auto_after_migrate = true

# Auto-run lightweight passes after skill install (default: true)
auto_after_install = true

# Schedule periodic defrag (cron expression, empty = disabled)
schedule = ""

# Custom replacement mappings for refs pass
[defrag.mappings]
"~/.oldplatform/" = "~/.ironclad/"
"oldplatform status" = "ironclad status"
```

## Architecture

### Crate placement

New module: `ironclad-server/src/cli/defrag.rs`

Uses:
- `ironclad-core` for config parsing (ground truth for drift detection)
- `ironclad-db` for stale state checks
- `std::fs` for artifact scanning
- `regex` for reference pattern matching

### Integration with `migrate`

After `cmd_migrate_import()` completes, call `cmd_defrag()` with `fix=false` to produce a report. If findings > 0, prompt:

```
  Migration complete. Defrag found 14 coherence issues.
  Run `ironclad defrag --fix` to auto-repair? [Y/n]
```

### Integration with `mechanic`

`mechanic` already outputs a health report. Add a section:

```
  Workspace Coherence ···  3 stale refs (run `ironclad defrag --fix`)
```

This is a summary only — `mechanic` doesn't run full defrag passes, it just checks if the last defrag report had unresolved findings.

## Implementation Sequence

1. **Define `DefragFinding` struct** — file, line, severity, message, fix description, fixable flag
2. **Implement Pass 3 (`artifacts`)** — simplest, pure filesystem, no config parsing needed
3. **Implement Pass 4 (`stale`)** — read `update_state.json`, cross-reference filesystem
4. **Implement Pass 5 (`identity`)** — read `ironclad.toml` + `OS.toml`, grep workspace
5. **Implement Pass 1 (`refs`)** — most complex, needs migration map + regex engine
6. **Implement Pass 2 (`drift`)** — needs TOML parsing + markdown pattern matching
7. **Implement Pass 6 (`scripts`)** — shebang/path/command validation
8. **Wire `--fix` mode** — apply fixes from passes 1-5 (pass 6 is report-only)
9. **Wire lifecycle hooks** — post-migrate, post-install, mechanic cross-ref
10. **Add `[defrag]` config section** to `ironclad-core/src/config.rs`

## Prior Art

This feature was designed from three manual cleanup sessions on a real Ironclad deployment:

| Session | Scope | Findings | Fixes |
|---------|-------|----------|-------|
| Skills deduplication | 19 duplicate/stub skill files | 19 deletions | artifacts, stale |
| PostgreSQL purge | ~35 stale database references | 35 edits across 16 files | refs, drift |
| Legacy→Ironclad migration | 271 stale platform references | 271 replacements across 40 files, 6 config fixes, 3 file deletions | refs, identity, artifacts, stale, drift, scripts |

Every defrag pass directly corresponds to a category of manual work performed during these sessions.
