# Skills Catalog Contract

## Purpose

This document defines the canonical data contract for skills across:

- Ironclad runtime (`ironclad-server`)
- Downloadable registry package (`registry/*`)
- Ironclad website registry (`ironclad-site/public/registry/*`)

The goal is to prevent drift between built-in skills, downloadable skills, and user-facing catalog views.

## Canonical Sources

- Built-in skill metadata: `registry/builtin-skills.json`
- Downloadable skill inventory and hashes: `registry/manifest.json`
- Downloadable skill content: `registry/skills/*.md`
- Provider catalog: `registry/providers.toml`

## Data Model

### Built-in skills (`builtin-skills.json`)

Array of objects:

- `name` (string, unique, lowercase-hyphenated)
- `description` (string)

Built-ins are always enabled and cannot be disabled/deleted through runtime APIs.

### Downloadable skills (`skills/*.md`)

Each instruction skill markdown file uses YAML frontmatter:

- `name` (string, unique)
- `description` (string)
- `triggers.keywords` (string[])
- `triggers.tool_names` (string[], optional)
- `triggers.regex_patterns` (string[], optional)
- `priority` (number)

Body text is the runtime instruction payload.

### Registry manifest (`manifest.json`)

`packs.skills.files` maps filename to SHA-256 digest and must match the exact contents in `registry/skills/*.md`.

`packs.builtins` points to `builtin-skills.json` and includes its SHA-256 digest.

## Invariants

1. Skill names are case-insensitively unique across built-in and downloadable catalogs.
2. Every file in `packs.skills.files` exists and hash matches.
3. Every `registry/skills/*.md` file is listed in `packs.skills.files`.
4. Website `public/registry/*` mirrors source registry artifacts at release sync.
5. Runtime built-in list is loaded from `registry/builtin-skills.json` (canonical), not hand-maintained constants.

## Ownership and Update Flow

1. Edit registry source files in `ironclad/registry/*`.
2. Run registry validation checks (hashes + name collisions).
3. Release workflow publishes artifacts and dispatches site sync.
4. Site sync copies canonical registry files to `ironclad-site/public/registry/*`.
5. Website registry UI reads generated/served data derived from those files.

## Backward Compatibility Rules

- Existing skill names should remain stable.
- If a downloadable skill is promoted to built-in:
  - Remove name from downloadable registry to avoid collision.
  - Keep behavior notes in release docs.
- Runtime API output keeps `built_in` flags and immutability semantics unchanged.
