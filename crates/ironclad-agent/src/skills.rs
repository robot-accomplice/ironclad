use std::path::Path;

use sha2::{Digest, Sha256};

use ironclad_core::{InstructionSkill, IroncladError, Result, SkillManifest, SkillTrigger};

#[derive(Debug, Clone)]
pub enum LoadedSkill {
    Structured(SkillManifest, String),
    Instruction(InstructionSkill, String),
}

impl LoadedSkill {
    pub fn name(&self) -> &str {
        match self {
            LoadedSkill::Structured(m, _) => &m.name,
            LoadedSkill::Instruction(i, _) => &i.name,
        }
    }

    pub fn triggers(&self) -> &SkillTrigger {
        match self {
            LoadedSkill::Structured(m, _) => &m.triggers,
            LoadedSkill::Instruction(i, _) => &i.triggers,
        }
    }

    pub fn hash(&self) -> &str {
        match self {
            LoadedSkill::Structured(_, h) | LoadedSkill::Instruction(_, h) => h,
        }
    }
}

fn content_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub struct SkillLoader;

impl SkillLoader {
    pub fn load_from_dir(dir: &Path) -> Result<Vec<LoadedSkill>> {
        let mut skills = Vec::new();

        if !dir.exists() {
            return Ok(skills);
        }

        let entries = std::fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                match path.extension().and_then(|e| e.to_str()) {
                    Some("toml") => {
                        let raw = std::fs::read_to_string(&path)?;
                        let hash = content_hash(raw.as_bytes());
                        let manifest: SkillManifest = toml::from_str(&raw).map_err(|e| {
                            IroncladError::Skill(format!("failed to parse {}: {e}", path.display()))
                        })?;
                        skills.push(LoadedSkill::Structured(manifest, hash));
                    }
                    Some("md") => {
                        let raw = std::fs::read_to_string(&path)?;
                        let hash = content_hash(raw.as_bytes());
                        let skill = parse_instruction_md(&raw, &path)?;
                        skills.push(LoadedSkill::Instruction(skill, hash));
                    }
                    _ => {}
                }
            }
        }

        Ok(skills)
    }
}

fn parse_instruction_md(content: &str, path: &Path) -> Result<InstructionSkill> {
    let trimmed = content.trim();

    if !trimmed.starts_with("---") {
        return Err(IroncladError::Skill(format!(
            "no YAML frontmatter in {}",
            path.display()
        )));
    }

    let rest = &trimmed[3..];
    let end = rest.find("---").ok_or_else(|| {
        IroncladError::Skill(format!("unclosed YAML frontmatter in {}", path.display()))
    })?;

    let yaml_str = &rest[..end];
    let body = rest[end + 3..].trim().to_string();

    #[derive(serde::Deserialize)]
    struct FrontMatter {
        name: String,
        description: String,
        #[serde(default)]
        triggers: SkillTrigger,
        #[serde(default = "default_priority")]
        priority: u32,
    }

    fn default_priority() -> u32 {
        5
    }

    let fm: FrontMatter = serde_yaml::from_str(yaml_str).map_err(|e| {
        IroncladError::Skill(format!(
            "invalid YAML frontmatter in {}: {e}",
            path.display()
        ))
    })?;

    Ok(InstructionSkill {
        name: fm.name,
        description: fm.description,
        triggers: fm.triggers,
        priority: fm.priority,
        body,
    })
}

pub struct SkillRegistry {
    skills: Vec<LoadedSkill>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self { skills: Vec::new() }
    }

    pub fn register(&mut self, skill: LoadedSkill) {
        self.skills.push(skill);
    }

    pub fn match_skills(&self, keywords: &[&str]) -> Vec<&LoadedSkill> {
        self.skills
            .iter()
            .filter(|skill| {
                let triggers = skill.triggers();
                keywords.iter().any(|kw| {
                    let kw_lower = kw.to_lowercase();
                    triggers
                        .keywords
                        .iter()
                        .any(|t| t.to_lowercase().contains(&kw_lower))
                })
            })
            .collect()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_toml_skill_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let toml_content = r#"
name = "code_review"
description = "Reviews code for quality"
kind = "Structured"
priority = 3
risk_level = "Safe"

[triggers]
keywords = ["review", "code"]
tool_names = []
regex_patterns = []
"#;
        fs::write(dir.path().join("code_review.toml"), toml_content).unwrap();

        let skills = SkillLoader::load_from_dir(dir.path()).unwrap();
        assert_eq!(skills.len(), 1);

        match &skills[0] {
            LoadedSkill::Structured(manifest, hash) => {
                assert_eq!(manifest.name, "code_review");
                assert_eq!(manifest.priority, 3);
                assert!(!hash.is_empty());
            }
            _ => panic!("expected Structured skill"),
        }
    }

    #[test]
    fn parse_md_instruction_skill() {
        let dir = tempfile::tempdir().unwrap();
        let md_content = r#"---
name: greeting
description: Greets the user warmly
triggers:
  keywords:
    - hello
    - greet
priority: 2
---
Always greet the user with enthusiasm and warmth.
"#;
        fs::write(dir.path().join("greeting.md"), md_content).unwrap();

        let skills = SkillLoader::load_from_dir(dir.path()).unwrap();
        assert_eq!(skills.len(), 1);

        match &skills[0] {
            LoadedSkill::Instruction(skill, hash) => {
                assert_eq!(skill.name, "greeting");
                assert_eq!(skill.priority, 2);
                assert!(skill.body.contains("enthusiasm"));
                assert!(!hash.is_empty());
            }
            _ => panic!("expected Instruction skill"),
        }
    }

    #[test]
    fn trigger_matching() {
        let mut registry = SkillRegistry::new();

        let skill_a = LoadedSkill::Instruction(
            InstructionSkill {
                name: "code_review".into(),
                description: "Reviews code".into(),
                triggers: SkillTrigger {
                    keywords: vec!["review".into(), "code".into()],
                    tool_names: vec![],
                    regex_patterns: vec![],
                },
                priority: 5,
                body: "Review the code.".into(),
            },
            "hash_a".into(),
        );

        let skill_b = LoadedSkill::Instruction(
            InstructionSkill {
                name: "deploy".into(),
                description: "Deploys services".into(),
                triggers: SkillTrigger {
                    keywords: vec!["deploy".into(), "release".into()],
                    tool_names: vec![],
                    regex_patterns: vec![],
                },
                priority: 5,
                body: "Deploy the service.".into(),
            },
            "hash_b".into(),
        );

        registry.register(skill_a);
        registry.register(skill_b);

        let matches = registry.match_skills(&["review"]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name(), "code_review");

        let matches = registry.match_skills(&["deploy"]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name(), "deploy");

        let matches = registry.match_skills(&["unrelated"]);
        assert!(matches.is_empty());
    }
}
