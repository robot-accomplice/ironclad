use ironclad_agent::skills::{SkillLoader, SkillRegistry};
use std::fs;

#[test]
fn reload_detects_content_change() {
    let dir = tempfile::tempdir().unwrap();

    let v1 = r#"
name = "deploy"
description = "Deploys services to production"
kind = "Structured"
risk_level = "Caution"

[triggers]
keywords = ["deploy", "ship"]
"#;
    fs::write(dir.path().join("deploy.toml"), v1).unwrap();

    let skills_v1 = SkillLoader::load_from_dir(dir.path()).unwrap();
    assert_eq!(skills_v1.len(), 1);
    let hash_v1 = skills_v1[0].hash().to_string();

    let mut registry = SkillRegistry::new();
    for skill in skills_v1 {
        registry.register(skill);
    }
    let matches = registry.match_skills(&["deploy"]);
    assert_eq!(matches.len(), 1);

    let v2 = r#"
name = "deploy"
description = "Deploys services to staging and production with rollback"
kind = "Structured"
risk_level = "Caution"

[triggers]
keywords = ["deploy", "ship", "release", "rollback"]
"#;
    fs::write(dir.path().join("deploy.toml"), v2).unwrap();

    let skills_v2 = SkillLoader::load_from_dir(dir.path()).unwrap();
    assert_eq!(skills_v2.len(), 1);
    let hash_v2 = skills_v2[0].hash().to_string();

    assert_ne!(
        hash_v1, hash_v2,
        "hash should change after file modification"
    );

    let mut registry_v2 = SkillRegistry::new();
    for skill in skills_v2 {
        registry_v2.register(skill);
    }

    let rollback_matches = registry_v2.match_skills(&["rollback"]);
    assert_eq!(rollback_matches.len(), 1);
    assert_eq!(rollback_matches[0].name(), "deploy");
}

#[test]
fn added_skill_file_picked_up_on_reload() {
    let dir = tempfile::tempdir().unwrap();

    let skills_before = SkillLoader::load_from_dir(dir.path()).unwrap();
    assert!(skills_before.is_empty());

    let new_skill = r#"
name = "monitor"
description = "Monitors system health"
kind = "Structured"
risk_level = "Safe"

[triggers]
keywords = ["monitor", "health"]
"#;
    fs::write(dir.path().join("monitor.toml"), new_skill).unwrap();

    let skills_after = SkillLoader::load_from_dir(dir.path()).unwrap();
    assert_eq!(skills_after.len(), 1);
    assert_eq!(skills_after[0].name(), "monitor");
}
