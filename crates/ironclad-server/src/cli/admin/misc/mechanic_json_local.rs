fn collect_mechanic_json_local_findings(
    ironclad_dir: &Path,
    repair: bool,
    findings: &mut Vec<MechanicFinding>,
    actions: &mut RepairActionSummary,
) -> Result<(), Box<dyn std::error::Error>> {
    let ironclad_dir = ironclad_dir.to_path_buf();
    let dirs = [
        ironclad_dir.clone(),
        ironclad_dir.join("workspace"),
        ironclad_dir.join("skills"),
        ironclad_dir.join("plugins"),
        ironclad_dir.join("logs"),
    ];
    for dir in &dirs {
        if !dir.exists() {
            let mut f = finding(
                "missing-directory",
                "medium",
                0.99,
                format!("Missing directory: {}", dir.display()),
                "Required runtime directory is absent.",
                "Create required Ironclad directory tree.",
                vec![format!("mkdir -p \"{}\"", dir.display())],
                true,
                false,
            );
            if repair {
                std::fs::create_dir_all(dir)?;
                f.auto_repaired = true;
                actions.directories_created.push(dir.display().to_string());
            }
            findings.push(f);
        }
    }

    let config_path = std::path::Path::new("ironclad.toml");
    let alt_config = ironclad_dir.join("ironclad.toml");
    if !config_path.exists() && !alt_config.exists() {
        let mut f = finding(
            "missing-config",
            "high",
            0.98,
            "No Ironclad config file found",
            "Neither local ./ironclad.toml nor ~/.ironclad/ironclad.toml exists.",
            "Initialize or restore runtime configuration.",
            vec!["ironclad init".to_string()],
            true,
            false,
        );
        if repair {
            let default_config = format!(
                concat!(
                    "[agent]\n",
                    "name = \"Ironclad\"\n",
                    "id = \"ironclad-dev\"\n\n",
                    "[server]\n",
                    "port = 18789\n",
                    "bind = \"127.0.0.1\"\n\n",
                    "[database]\n",
                    "path = \"{}/state.db\"\n\n",
                    "[models]\n",
                    "primary = \"ollama/qwen3:8b\"\n",
                    "fallbacks = [\"openai/gpt-4o\"]\n",
                ),
                ironclad_dir.display()
            );
            std::fs::create_dir_all(&ironclad_dir)?;
            std::fs::write(&alt_config, default_config)?;
            f.auto_repaired = true;
            actions.config_created = true;
        }
        findings.push(f);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [
            ironclad_dir.join("wallet.json"),
            ironclad_dir.join("state.db"),
        ] {
            if file.exists() {
                let meta = std::fs::metadata(&file)?;
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    let mut f = finding(
                        "loose-permissions",
                        "high",
                        0.97,
                        format!("Loose permissions on {}", file.display()),
                        format!("Current mode {:o} allows group/other access.", mode),
                        "Harden file permissions to owner-only (0600).",
                        vec![format!("chmod 600 \"{}\"", file.display())],
                        true,
                        false,
                    );
                    if repair {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o600);
                        std::fs::set_permissions(&file, perms)?;
                        f.auto_repaired = true;
                        actions
                            .permissions_hardened
                            .push(file.display().to_string());
                    }
                    findings.push(f);
                }
            }
        }
    }

    let oauth_health = check_and_repair_oauth_storage(repair);
    if oauth_health.needs_attention() {
        let mut details = Vec::new();
        if oauth_health.legacy_plaintext_exists {
            details.push("legacy plaintext OAuth token file is present".to_string());
        }
        if !oauth_health.keystore_available {
            details.push("keystore is unavailable".to_string());
        }
        if oauth_health.malformed_keystore_entries > 0 {
            details.push(format!(
                "{} malformed keystore OAuth entr{}",
                oauth_health.malformed_keystore_entries,
                if oauth_health.malformed_keystore_entries == 1 {
                    "y"
                } else {
                    "ies"
                }
            ));
        }
        if oauth_health.legacy_parse_failed {
            details.push("legacy OAuth token file could not be parsed".to_string());
        }

        let mut finding = finding(
            "oauth-storage-drift",
            if !oauth_health.keystore_available {
                "high"
            } else {
                "medium"
            },
            0.97,
            "OAuth token storage needs migration/repair",
            details.join("; "),
            "Migrate OAuth tokens to encrypted keystore and remove legacy plaintext artifacts.",
            vec!["ironclad mechanic --repair".to_string()],
            true,
            false,
        );
        if repair && oauth_health.repaired {
            finding.auto_repaired = true;
        }
        findings.push(finding);
    }

    let skills_cleanup = cleanup_internalized_skill_artifacts(
        &ironclad_dir.join("state.db"),
        &ironclad_dir.join("skills"),
        repair,
    );
    if !skills_cleanup.stale_db_skills.is_empty()
        || !skills_cleanup.stale_files.is_empty()
        || !skills_cleanup.stale_dirs.is_empty()
    {
        let stale_db = if skills_cleanup.stale_db_skills.is_empty() {
            "none".to_string()
        } else {
            skills_cleanup.stale_db_skills.join(", ")
        };
        let stale_fs_items: Vec<String> = skills_cleanup
            .stale_files
            .iter()
            .chain(skills_cleanup.stale_dirs.iter())
            .map(|p| p.display().to_string())
            .collect();
        let stale_fs = if stale_fs_items.is_empty() {
            "none".to_string()
        } else {
            stale_fs_items.join(", ")
        };
        let mut f = finding(
            "internalized-skill-drift",
            "medium",
            0.98,
            "Internalized skills still exist as external artifacts",
            format!("stale_db=[{stale_db}]; stale_fs=[{stale_fs}]"),
            "Remove obsolete externalized skill entries/files for internalized skills.",
            vec!["ironclad mechanic --repair".to_string()],
            true,
            false,
        );
        if repair
            && (!skills_cleanup.removed_db_skills.is_empty()
                || !skills_cleanup.removed_paths.is_empty())
        {
            f.auto_repaired = true;
            actions.internalized_skills_cleaned.extend(
                skills_cleanup
                    .removed_db_skills
                    .iter()
                    .map(|s| format!("db:{s}"))
                    .chain(
                        skills_cleanup
                            .removed_paths
                            .iter()
                            .map(|p| format!("fs:{}", p.display())),
                    ),
            );
        }
        findings.push(f);
    }

    let capability_skill_parity = evaluate_capability_skill_parity(&ironclad_dir.join("state.db"));
    if !capability_skill_parity.missing_in_registry.is_empty() {
        findings.push(finding(
            "capability-skill-parity-registry-gap",
            "high",
            0.97,
            "Capability-to-skill parity gap in builtin skill registry",
            capability_skill_parity.missing_in_registry.join("; "),
            "Add missing builtin skills to registry/builtin-skills.json for declared runtime capabilities.",
            vec!["Update registry/builtin-skills.json and reload skills".to_string()],
            false,
            false,
        ));
    }
    if !capability_skill_parity.missing_in_db.is_empty() {
        findings.push(finding(
            "capability-skill-parity-db-gap",
            "medium",
            0.95,
            "Capability-to-skill parity gap in loaded skill database",
            capability_skill_parity.missing_in_db.join("; "),
            "Reload/reconcile skills so builtin capability skills are active in DB.",
            vec![
                "ironclad skills reload".to_string(),
                "ironclad mechanic --repair".to_string(),
            ],
            true,
            false,
        ));
    }
    Ok(())
}

