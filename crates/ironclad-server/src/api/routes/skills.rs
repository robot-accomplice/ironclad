use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::Path as FsPath;

use super::{AppState, internal_err};

struct BuiltinSkillDef {
    name: &'static str,
    description: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryManifest {
    version: String,
    packs: RegistryPacks,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryPacks {
    skills: RegistrySkillPack,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistrySkillPack {
    path: String,
    files: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct CatalogQuery {
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CatalogInstallRequest {
    pub skills: Vec<String>,
    #[serde(default)]
    pub activate: bool,
}

#[derive(Debug, Deserialize)]
pub struct CatalogActivateRequest {
    pub skills: Vec<String>,
}

const BUILTIN_SKILLS: &[BuiltinSkillDef] = &[
    BuiltinSkillDef {
        name: "context-continuity",
        description: "Preserve continuity across sessions and long-running workflows.",
    },
    BuiltinSkillDef {
        name: "conway-security",
        description: "Security guardrails for high-impact infrastructure workflows.",
    },
    BuiltinSkillDef {
        name: "ethereum-funding",
        description: "Operational treasury and Ethereum funding workflows.",
    },
    BuiltinSkillDef {
        name: "himalaya-email",
        description: "CLI-based email operations through a local mail bridge.",
    },
    BuiltinSkillDef {
        name: "knowledge-management",
        description: "Knowledge capture, curation, and retrieval conventions.",
    },
    BuiltinSkillDef {
        name: "local-subagents",
        description: "Subagent orchestration for parallelized task execution.",
    },
    BuiltinSkillDef {
        name: "model-management",
        description: "Model routing and fallback strategy management.",
    },
    BuiltinSkillDef {
        name: "obsidian-vault",
        description: "Obsidian-backed knowledge workflows and synchronization.",
    },
    BuiltinSkillDef {
        name: "scope-cli",
        description: "Scope and boundary management for CLI-driven workflows.",
    },
    BuiltinSkillDef {
        name: "search-management",
        description: "Search and retrieval strategy management for investigations.",
    },
    BuiltinSkillDef {
        name: "self-diagnostics",
        description: "Runtime diagnostics and self-healing operational checks.",
    },
    BuiltinSkillDef {
        name: "self-funding",
        description: "Autonomous funding and sustainability operational workflows.",
    },
    BuiltinSkillDef {
        name: "session-bloat-prevention",
        description: "Context-budget controls to prevent session bloat.",
    },
    BuiltinSkillDef {
        name: "supervisor-protocol",
        description: "Supervisor and delegation protocol for specialist execution.",
    },
];

fn is_builtin_skill_name(name: &str) -> bool {
    BUILTIN_SKILLS
        .iter()
        .any(|skill| skill.name.eq_ignore_ascii_case(name))
}

fn is_builtin_skill(s: &ironclad_db::skills::SkillRecord) -> bool {
    s.kind.eq_ignore_ascii_case("builtin") || is_builtin_skill_name(&s.name)
}

fn canonical_in_root(root: &FsPath, base: &FsPath, raw: &FsPath) -> Result<String, String> {
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(raw)
    };
    let canonical = std::fs::canonicalize(&candidate).map_err(|e| {
        format!(
            "script path '{}' cannot be resolved: {e}",
            candidate.display()
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(format!(
            "script path '{}' escapes skills_dir '{}'",
            canonical.display(),
            root.display()
        ));
    }
    if !canonical.is_file() {
        return Err(format!(
            "script path '{}' is not a file",
            canonical.display()
        ));
    }
    Ok(canonical.to_string_lossy().to_string())
}

fn validate_policy_overrides(value: &serde_json::Value) -> Result<(), String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "policy_overrides must be a JSON object".to_string())?;
    let allowed = ["require_creator", "deny_external", "disabled"];
    for (k, v) in obj {
        if !allowed.contains(&k.as_str()) {
            return Err(format!("unsupported policy_overrides key '{k}'"));
        }
        if !v.is_boolean() {
            return Err(format!(
                "policy_overrides key '{}' must be boolean, got {}",
                k, v
            ));
        }
    }
    Ok(())
}

fn normalize_risk_level(raw: &str) -> Result<&'static str, String> {
    match raw.to_ascii_lowercase().as_str() {
        "safe" => Ok("Safe"),
        "caution" => Ok("Caution"),
        "dangerous" => Ok("Dangerous"),
        "forbidden" => Ok("Forbidden"),
        _ => Err(format!("invalid risk_level '{raw}'")),
    }
}

fn registry_base_url(manifest_url: &str) -> String {
    if let Some(pos) = manifest_url.rfind('/') {
        manifest_url[..pos].to_string()
    } else {
        manifest_url.to_string()
    }
}

async fn fetch_catalog_manifest(state: &AppState) -> Result<(RegistryManifest, String), String> {
    let config = state.config.read().await;
    let registry_url = config.update.registry_url.clone();
    drop(config);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("catalog client init failed: {e}"))?;
    let resp = client
        .get(&registry_url)
        .send()
        .await
        .map_err(|e| format!("catalog manifest fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "catalog manifest endpoint returned HTTP {}",
            resp.status()
        ));
    }
    let manifest: RegistryManifest = resp
        .json()
        .await
        .map_err(|e| format!("invalid catalog manifest JSON: {e}"))?;
    Ok((manifest, registry_base_url(&registry_url)))
}

fn skill_item_matches_query(name: &str, query: Option<&str>) -> bool {
    if let Some(q) = query {
        return name.to_ascii_lowercase().contains(&q.to_ascii_lowercase());
    }
    true
}

async fn reload_skills_internal(
    state: &AppState,
) -> Result<Value, (axum::http::StatusCode, String)> {
    fn risk_level_str(r: ironclad_core::RiskLevel) -> &'static str {
        match r {
            ironclad_core::RiskLevel::Safe => "Safe",
            ironclad_core::RiskLevel::Caution => "Caution",
            ironclad_core::RiskLevel::Dangerous => "Dangerous",
            ironclad_core::RiskLevel::Forbidden => "Forbidden",
        }
    }
    let config = state.config.read().await;
    let skills_dir = std::fs::canonicalize(&config.skills.skills_dir).map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "failed to resolve skills_dir '{}': {e}",
                config.skills.skills_dir.display()
            ),
        )
    })?;
    drop(config);

    let loaded = ironclad_agent::skills::SkillLoader::load_from_dir(&skills_dir)
        .map_err(|e| internal_err(&e))?;

    let mut added = 0u32;
    let mut updated = 0u32;
    let mut rejected = 0u32;
    let mut issues: Vec<String> = Vec::new();

    let existing_by_name: std::collections::HashMap<String, ironclad_db::skills::SkillRecord> =
        ironclad_db::skills::list_skills(&state.db)
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.name.clone(), s))
            .collect();

    for skill in &loaded {
        let name = skill.name();
        let hash = skill.hash();
        let kind = match skill {
            ironclad_agent::skills::LoadedSkill::Structured(_, _, _) => "structured",
            ironclad_agent::skills::LoadedSkill::Instruction(_, _, _) => "instruction",
        };
        let triggers = serde_json::to_string(skill.triggers()).ok();
        let source = skill.source_path().to_string_lossy().to_string();
        let desc = skill.description();
        let (tool_chain_json, policy_overrides_json, script_path, risk_level) = if let Some(
            manifest,
        ) = skill.structured_manifest()
        {
            if manifest
                .tool_chain
                .as_ref()
                .is_some_and(|chain| !chain.is_empty())
            {
                rejected += 1;
                issues.push(format!(
                        "rejected skill '{}': tool_chain is not yet executable in runtime (remove it or keep empty)",
                        name
                    ));
                continue;
            }

            let tool_chain_json = manifest
                .tool_chain
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok());
            let policy_overrides_json = if let Some(v) = manifest.policy_overrides.as_ref() {
                if let Err(msg) = validate_policy_overrides(v) {
                    rejected += 1;
                    issues.push(format!("rejected skill '{}': {}", name, msg));
                    continue;
                }
                serde_json::to_string(v).ok()
            } else {
                None
            };
            let base = skill.source_path().parent().unwrap_or(skills_dir.as_path());
            let script_path = manifest
                .script_path
                .as_ref()
                .map(|p| canonical_in_root(&skills_dir, base, p))
                .transpose()
                .map_err(|msg| {
                    rejected += 1;
                    issues.push(format!("rejected skill '{}': {}", name, msg));
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        "invalid skill manifest".to_string(),
                    )
                })
                .ok()
                .flatten();
            if manifest.script_path.is_some() && script_path.is_none() {
                continue;
            }
            (
                tool_chain_json,
                policy_overrides_json,
                script_path,
                risk_level_str(manifest.risk_level).to_string(),
            )
        } else {
            (None, None, None, "Caution".to_string())
        };

        let existing = existing_by_name.get(name);

        if let Some(existing) = existing {
            if existing.content_hash != hash
                || existing.triggers_json.as_deref() != triggers.as_deref()
                || existing.tool_chain_json.as_deref() != tool_chain_json.as_deref()
                || existing.policy_overrides_json.as_deref() != policy_overrides_json.as_deref()
                || existing.script_path.as_deref() != script_path.as_deref()
                || existing.source_path != source
                || existing.risk_level != risk_level
            {
                if let Err(e) = ironclad_db::skills::update_skill_full(
                    &state.db,
                    &existing.id,
                    hash,
                    triggers.as_deref(),
                    tool_chain_json.as_deref(),
                    policy_overrides_json.as_deref(),
                    script_path.as_deref(),
                    &source,
                    &risk_level,
                ) {
                    tracing::warn!(error = %e, skill = name, "skill sync: update_skill failed");
                }
                updated += 1;
            }
        } else {
            if let Err(e) = ironclad_db::skills::register_skill_full(
                &state.db,
                name,
                kind,
                desc,
                &source,
                hash,
                triggers.as_deref(),
                tool_chain_json.as_deref(),
                policy_overrides_json.as_deref(),
                script_path.as_deref(),
                &risk_level,
            ) {
                tracing::warn!(error = %e, skill = name, "skill sync: register_skill failed");
            }
            added += 1;
        }
    }

    Ok(serde_json::json!({
        "reloaded": true,
        "scanned": loaded.len(),
        "added": added,
        "updated": updated,
        "rejected": rejected,
        "issues": issues,
    }))
}

pub async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::skills::list_skills(&state.db) {
        Ok(skills) => {
            let mut items: Vec<Value> = skills
                .into_iter()
                .map(|s| {
                    let built_in = is_builtin_skill(&s);
                    serde_json::json!({
                        "id": s.id,
                        "name": s.name,
                        "kind": s.kind,
                        "description": s.description,
                        "source_path": s.source_path,
                        "risk_level": s.risk_level,
                        "enabled": s.enabled || built_in,
                        "built_in": built_in,
                        "last_loaded_at": s.last_loaded_at,
                        "created_at": s.created_at,
                    })
                })
                .collect();
            let seen: HashSet<String> = items
                .iter()
                .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
                .map(|name| name.to_ascii_lowercase())
                .collect();
            for built_in in BUILTIN_SKILLS {
                if seen.contains(&built_in.name.to_ascii_lowercase()) {
                    continue;
                }
                items.push(serde_json::json!({
                    "id": format!("builtin:{}", built_in.name),
                    "name": built_in.name,
                    "kind": "builtin",
                    "description": built_in.description,
                    "source_path": Value::Null,
                    "risk_level": "Caution",
                    "enabled": true,
                    "built_in": true,
                    "last_loaded_at": Value::Null,
                    "created_at": Value::Null,
                }));
            }
            Ok(axum::Json(serde_json::json!({ "skills": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_skill(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match ironclad_db::skills::get_skill(&state.db, &id) {
        Ok(Some(s)) => {
            let built_in = is_builtin_skill(&s);
            Ok(axum::Json(serde_json::json!({
                "id": s.id,
                "name": s.name,
                "kind": s.kind,
                "description": s.description,
                "source_path": s.source_path,
                "content_hash": s.content_hash,
                "triggers_json": s.triggers_json,
                "tool_chain_json": s.tool_chain_json,
                "policy_overrides_json": s.policy_overrides_json,
                "script_path": s.script_path,
                "risk_level": s.risk_level,
                "enabled": s.enabled || built_in,
                "built_in": built_in,
                "last_loaded_at": s.last_loaded_at,
                "created_at": s.created_at,
            })))
        }
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("skill {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn reload_skills(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let payload = reload_skills_internal(&state).await?;
    Ok(axum::Json(payload))
}

pub async fn catalog_list(
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> impl IntoResponse {
    let q = query.q.as_deref();
    let mut items = Vec::<Value>::new();
    for built_in in BUILTIN_SKILLS {
        if skill_item_matches_query(built_in.name, q) {
            items.push(serde_json::json!({
                "name": built_in.name,
                "kind": "builtin",
                "description": built_in.description,
                "source": "builtin",
            }));
        }
    }

    if let Ok((manifest, _base_url)) = fetch_catalog_manifest(&state).await {
        for (filename, sha256) in &manifest.packs.skills.files {
            let name = filename.strip_suffix(".md").unwrap_or(filename).to_string();
            if skill_item_matches_query(&name, q) {
                items.push(serde_json::json!({
                    "name": name,
                    "kind": "instruction",
                    "source": "registry",
                    "filename": filename,
                    "sha256": sha256,
                    "version": manifest.version,
                }));
            }
        }
    }

    axum::Json(serde_json::json!({ "items": items }))
}

pub async fn catalog_install(
    State(state): State<AppState>,
    Json(req): Json<CatalogInstallRequest>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    if req.skills.is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "skills list is required".to_string(),
        ));
    }
    let (manifest, base_url) = fetch_catalog_manifest(&state).await.map_err(|e| {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            format!("failed to fetch catalog: {e}"),
        )
    })?;

    let config = state.config.read().await;
    let skills_dir = config.skills.skills_dir.clone();
    drop(config);
    std::fs::create_dir_all(&skills_dir)
        .map_err(|e| internal_err(&format!("failed to create skills dir: {e}")))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| internal_err(&format!("catalog client init failed: {e}")))?;

    let selected: Vec<(&String, &String)> = manifest
        .packs
        .skills
        .files
        .iter()
        .filter(|(filename, _)| {
            let name = filename.strip_suffix(".md").unwrap_or(filename.as_str());
            req.skills.iter().any(|s| s == name || s == *filename)
        })
        .collect();
    if selected.is_empty() {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "no matching catalog skills found".to_string(),
        ));
    }

    let mut rollback_existing: Vec<(std::path::PathBuf, Vec<u8>)> = Vec::new();
    let mut rollback_new: Vec<std::path::PathBuf> = Vec::new();
    let mut installed: Vec<String> = Vec::new();

    for (filename, expected_hash) in selected {
        let url = format!("{}/{}{}", base_url, manifest.packs.skills.path, filename);
        let bytes = client
            .get(&url)
            .send()
            .await
            .map_err(|e| internal_err(&format!("download failed for {filename}: {e}")))?
            .bytes()
            .await
            .map_err(|e| internal_err(&format!("download body failed for {filename}: {e}")))?;
        let actual_hash = hex::encode(Sha256::digest(&bytes));
        if actual_hash != *expected_hash {
            // Roll back any files already touched.
            for (path, old) in rollback_existing.drain(..) {
                let _ = std::fs::write(path, old);
            }
            for path in rollback_new.drain(..) {
                let _ = std::fs::remove_file(path);
            }
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                format!("checksum mismatch for {filename}"),
            ));
        }

        let target = skills_dir.join(filename);
        if target.exists() {
            let old = std::fs::read(&target)
                .map_err(|e| internal_err(&format!("failed to backup {filename}: {e}")))?;
            rollback_existing.push((target.clone(), old));
        } else {
            rollback_new.push(target.clone());
        }

        let tmp = skills_dir.join(format!(".{}.tmp", filename));
        std::fs::write(&tmp, &bytes)
            .map_err(|e| internal_err(&format!("failed to write temp {filename}: {e}")))?;
        std::fs::rename(&tmp, &target)
            .map_err(|e| internal_err(&format!("failed to install {filename}: {e}")))?;
        installed.push(filename.clone());
    }

    let mut activation = None;
    if req.activate {
        match reload_skills_internal(&state).await {
            Ok(payload) => activation = Some(payload),
            Err(e) => {
                // Roll back all file writes if activation failed.
                for (path, old) in rollback_existing {
                    let _ = std::fs::write(path, old);
                }
                for path in rollback_new {
                    let _ = std::fs::remove_file(path);
                }
                return Err(e);
            }
        }
    }

    Ok(axum::Json(serde_json::json!({
        "ok": true,
        "version": manifest.version,
        "installed": installed,
        "activated": req.activate,
        "activation": activation,
    })))
}

pub async fn catalog_activate(
    State(state): State<AppState>,
    Json(req): Json<CatalogActivateRequest>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let payload = reload_skills_internal(&state).await?;
    Ok(axum::Json(serde_json::json!({
        "ok": true,
        "requested_skills": req.skills,
        "activation": payload,
    })))
}

pub async fn audit_skills(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let config = state.config.read().await;
    let skills_dir = std::fs::canonicalize(&config.skills.skills_dir).map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "failed to resolve skills_dir '{}': {e}",
                config.skills.skills_dir.display()
            ),
        )
    })?;
    drop(config);

    let loaded = ironclad_agent::skills::SkillLoader::load_from_dir(&skills_dir)
        .map_err(|e| internal_err(&e))?;
    let loaded_by_name: std::collections::HashMap<
        String,
        (&ironclad_agent::skills::LoadedSkill, String),
    > = loaded
        .iter()
        .map(|s| (s.name().to_string(), (s, s.hash().to_string())))
        .collect();

    let db_skills = ironclad_db::skills::list_skills(&state.db).map_err(|e| internal_err(&e))?;
    let mut drifted = 0usize;
    let mut skills_report = Vec::new();

    for s in &db_skills {
        let (drift_status, drift_reason) = if let Err(msg) = normalize_risk_level(&s.risk_level) {
            drifted += 1;
            ("invalid_metadata", msg)
        } else if let Some((loaded_skill, loaded_hash)) = loaded_by_name.get(&s.name) {
            if &s.content_hash != loaded_hash {
                drifted += 1;
                (
                    "drifted",
                    format!("hash mismatch (db={} disk={})", s.content_hash, loaded_hash),
                )
            } else {
                let mut issues = Vec::new();
                if let Some(manifest) = loaded_skill.structured_manifest() {
                    if manifest.tool_chain.as_ref().is_some_and(|c| !c.is_empty()) {
                        issues.push(
                            "tool_chain present but runtime does not execute skill tool chains"
                                .to_string(),
                        );
                    }
                    if let Some(v) = manifest.policy_overrides.as_ref()
                        && let Err(msg) = validate_policy_overrides(v)
                    {
                        issues.push(msg);
                    }
                    if let Some(script) = manifest.script_path.as_ref() {
                        let base = loaded_skill
                            .source_path()
                            .parent()
                            .unwrap_or(skills_dir.as_path());
                        if let Err(msg) = canonical_in_root(&skills_dir, base, script) {
                            issues.push(msg);
                        }
                    }
                }
                if issues.is_empty() {
                    ("in_sync", String::new())
                } else {
                    drifted += 1;
                    ("invalid_metadata", issues.join("; "))
                }
            }
        } else {
            drifted += 1;
            (
                "missing_on_disk",
                "present in DB but not found in skills_dir scan".to_string(),
            )
        };

        skills_report.push(serde_json::json!({
            "id": s.id,
            "name": s.name,
            "enabled": s.enabled,
            "risk_level": s.risk_level,
            "source_path": s.source_path,
            "drift_status": drift_status,
            "drift_reason": drift_reason,
        }));
    }

    let tool_names: Vec<String> = state
        .tools
        .list()
        .into_iter()
        .map(|t| t.name().to_string())
        .collect();
    let key_tools = [
        ("run_script", serde_json::json!({"path":"sample.sh"})),
        ("read_file", serde_json::json!({"path":"README.md"})),
        (
            "write_file",
            serde_json::json!({"path":"tmp/audit.txt","content":"x"}),
        ),
        (
            "edit_file",
            serde_json::json!({"path":"tmp/audit.txt","old":"x","new":"y"}),
        ),
        ("list_directory", serde_json::json!({"path":"."})),
        ("glob_files", serde_json::json!({"pattern":"*.md"})),
        ("search_files", serde_json::json!({"query":"TODO"})),
    ];
    let mut capability_rows = Vec::new();
    for (tool_name, sample_params) in key_tools {
        let Some(tool) = state.tools.get(tool_name) else {
            capability_rows.push(serde_json::json!({
                "tool_name": tool_name,
                "present": false,
            }));
            continue;
        };
        let normal_tier = ironclad_core::SurvivalTier::Normal;
        let creator_allowed = super::agent::check_tool_policy(
            &state.policy_engine,
            tool_name,
            &sample_params,
            ironclad_core::InputAuthority::Creator,
            normal_tier,
            tool.risk_level(),
        )
        .is_ok();
        let external_allowed = super::agent::check_tool_policy(
            &state.policy_engine,
            tool_name,
            &sample_params,
            ironclad_core::InputAuthority::External,
            normal_tier,
            tool.risk_level(),
        )
        .is_ok();
        let approval_classification = state
            .approvals
            .check_tool(tool_name)
            .map(|c| format!("{c:?}"))
            .unwrap_or_else(|e| format!("error:{e}"));
        capability_rows.push(serde_json::json!({
            "tool_name": tool_name,
            "present": true,
            "risk_level": format!("{:?}", tool.risk_level()),
            "creator_allowed": creator_allowed,
            "external_allowed": external_allowed,
            "approval_classification": approval_classification,
        }));
    }

    Ok(axum::Json(serde_json::json!({
        "skills_dir": skills_dir,
        "summary": {
            "db_skills": db_skills.len(),
            "disk_skills": loaded.len(),
            "drifted_skills": drifted,
        },
        "runtime": {
            "registered_tools": tool_names,
            "capabilities": capability_rows,
        },
        "skills": skills_report,
    })))
}

pub async fn toggle_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let existing = ironclad_db::skills::get_skill(&state.db, &id).map_err(|e| internal_err(&e))?;
    if let Some(s) = existing.as_ref()
        && is_builtin_skill(s)
    {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            format!("skill {} is built-in and cannot be disabled", s.name),
        ));
    }
    match ironclad_db::skills::toggle_skill_enabled(&state.db, &id) {
        Ok(Some(new_enabled)) => Ok(axum::Json(serde_json::json!({
            "id": id,
            "enabled": new_enabled,
        }))),
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("skill {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn delete_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    match ironclad_db::skills::get_skill(&state.db, &id) {
        Ok(Some(skill)) => {
            if is_builtin_skill(&skill) {
                return Err((
                    axum::http::StatusCode::FORBIDDEN,
                    format!("skill {} is built-in and cannot be deleted", skill.name),
                ));
            }
            ironclad_db::skills::delete_skill(&state.db, &id).map_err(|e| internal_err(&e))?;
            Ok(axum::Json(serde_json::json!({
                "id": id,
                "name": skill.name,
                "deleted": true,
            })))
        }
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("skill {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}
