use ironclad_agent::skills::{SkillLoader, SkillRegistry};
use std::fs;

#[test]
fn load_and_match_skills_from_filesystem() {
    let dir = tempfile::tempdir().unwrap();

    let toml_content = r#"
name = "weather_lookup"
description = "Looks up current weather for a location"
kind = "Structured"
priority = 3
risk_level = "Safe"

[triggers]
keywords = ["weather", "forecast", "temperature"]
tool_names = []
regex_patterns = []
"#;
    fs::write(dir.path().join("weather.toml"), toml_content).unwrap();

    let md_content = r#"---
name: code_review
description: Reviews code for quality and best practices
triggers:
  keywords:
    - review
    - code review
    - audit code
priority: 4
---
When reviewing code, check for:
1. Correctness
2. Performance
3. Security vulnerabilities
4. Code style consistency
"#;
    fs::write(dir.path().join("code_review.md"), md_content).unwrap();

    let skills = SkillLoader::load_from_dir(dir.path()).unwrap();
    assert_eq!(skills.len(), 2);

    let mut registry = SkillRegistry::new();
    for skill in skills {
        registry.register(skill);
    }

    let weather_matches = registry.match_skills(&["weather"]);
    assert_eq!(weather_matches.len(), 1);
    assert_eq!(weather_matches[0].name(), "weather_lookup");

    let review_matches = registry.match_skills(&["review"]);
    assert_eq!(review_matches.len(), 1);
    assert_eq!(review_matches[0].name(), "code_review");

    let no_matches = registry.match_skills(&["unrelated", "nonsense"]);
    assert!(no_matches.is_empty());

    let forecast_matches = registry.match_skills(&["forecast"]);
    assert_eq!(forecast_matches.len(), 1);
    assert_eq!(forecast_matches[0].name(), "weather_lookup");
}

#[test]
fn skill_hash_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let content = r#"
name = "test_skill"
description = "A test"
kind = "Structured"
risk_level = "Safe"

[triggers]
keywords = ["test"]
"#;
    fs::write(dir.path().join("test.toml"), content).unwrap();

    let skills_1 = SkillLoader::load_from_dir(dir.path()).unwrap();
    let skills_2 = SkillLoader::load_from_dir(dir.path()).unwrap();

    assert_eq!(skills_1[0].hash(), skills_2[0].hash());
}

#[test]
fn empty_dir_returns_no_skills() {
    let dir = tempfile::tempdir().unwrap();
    let skills = SkillLoader::load_from_dir(dir.path()).unwrap();
    assert!(skills.is_empty());
}

#[test]
fn nonexistent_dir_returns_no_skills() {
    let skills =
        SkillLoader::load_from_dir(std::path::Path::new("/nonexistent/skills/dir")).unwrap();
    assert!(skills.is_empty());
}
