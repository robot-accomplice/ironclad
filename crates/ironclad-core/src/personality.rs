use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// OS.toml -- personality, voice, tone
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsConfig {
    pub identity: OsIdentity,
    pub voice: OsVoice,
    #[serde(default)]
    pub prompt_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsIdentity {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_generated_by")]
    pub generated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsVoice {
    #[serde(default = "default_formality")]
    pub formality: String,
    #[serde(default = "default_proactiveness")]
    pub proactiveness: String,
    #[serde(default = "default_verbosity")]
    pub verbosity: String,
    #[serde(default = "default_humor")]
    pub humor: String,
    #[serde(default = "default_domain")]
    pub domain: String,
}

fn default_version() -> String { "1.0".into() }
fn default_generated_by() -> String { "default".into() }
fn default_formality() -> String { "balanced".into() }
fn default_proactiveness() -> String { "suggest".into() }
fn default_verbosity() -> String { "concise".into() }
fn default_humor() -> String { "dry".into() }
fn default_domain() -> String { "general".into() }

// ---------------------------------------------------------------------------
// FIRMWARE.toml -- guardrails, boundaries, hard rules
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareConfig {
    #[serde(default)]
    pub approvals: FirmwareApprovals,
    #[serde(default)]
    pub rules: Vec<FirmwareRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareApprovals {
    #[serde(default = "default_spending_threshold")]
    pub spending_threshold: f64,
    #[serde(default = "default_require_confirmation")]
    pub require_confirmation: String,
}

impl Default for FirmwareApprovals {
    fn default() -> Self {
        Self {
            spending_threshold: default_spending_threshold(),
            require_confirmation: default_require_confirmation(),
        }
    }
}

fn default_spending_threshold() -> f64 { 50.0 }
fn default_require_confirmation() -> String { "risky".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    pub rule: String,
}

// ---------------------------------------------------------------------------
// OPERATOR.toml -- user profile (long-form interview)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperatorConfig {
    #[serde(default)]
    pub identity: OperatorIdentity,
    #[serde(default)]
    pub preferences: OperatorPreferences,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperatorIdentity {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperatorPreferences {
    #[serde(default)]
    pub communication_channels: Vec<String>,
    #[serde(default)]
    pub work_hours: String,
    #[serde(default)]
    pub response_style: String,
}

// ---------------------------------------------------------------------------
// DIRECTIVES.toml -- goals, missions, priorities (long-form interview)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DirectivesConfig {
    #[serde(default)]
    pub missions: Vec<Mission>,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub name: String,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub description: String,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

pub fn load_os(workspace: &Path) -> Option<OsConfig> {
    let path = workspace.join("OS.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&text).ok()
}

pub fn load_firmware(workspace: &Path) -> Option<FirmwareConfig> {
    let path = workspace.join("FIRMWARE.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&text).ok()
}

pub fn load_operator(workspace: &Path) -> Option<OperatorConfig> {
    let path = workspace.join("OPERATOR.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&text).ok()
}

pub fn load_directives(workspace: &Path) -> Option<DirectivesConfig> {
    let path = workspace.join("DIRECTIVES.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&text).ok()
}

// ---------------------------------------------------------------------------
// Prompt composition -- assemble personality files into system prompt text
// ---------------------------------------------------------------------------

pub fn compose_soul(
    os: Option<&OsConfig>,
    firmware: Option<&FirmwareConfig>,
    operator: Option<&OperatorConfig>,
    directives: Option<&DirectivesConfig>,
) -> String {
    let mut sections = Vec::new();

    if let Some(os) = os {
        if !os.prompt_text.is_empty() {
            sections.push(os.prompt_text.clone());
        }
    }

    if let Some(fw) = firmware {
        if !fw.rules.is_empty() {
            let mut block = String::from("## Firmware (Non-Negotiable Rules)\n");
            for r in &fw.rules {
                let prefix = if r.rule_type == "must" { "YOU MUST" } else { "YOU MUST NOT" };
                block.push_str(&format!("- {prefix}: {}\n", r.rule));
            }
            sections.push(block);
        }
    }

    if let Some(op) = operator {
        if !op.context.is_empty() {
            sections.push(format!("## Operator Context\n{}", op.context));
        }
    }

    if let Some(dir) = directives {
        if !dir.context.is_empty() {
            sections.push(format!("## Active Directives\n{}", dir.context));
        }
        if !dir.missions.is_empty() {
            let mut block = String::from("## Missions\n");
            for m in &dir.missions {
                block.push_str(&format!(
                    "- **{}** ({}): {}\n",
                    m.name,
                    if m.timeframe.is_empty() { "ongoing" } else { &m.timeframe },
                    m.description
                ));
            }
            sections.push(block);
        }
    }

    sections.join("\n\n")
}

// ---------------------------------------------------------------------------
// Default Roboticus content
// ---------------------------------------------------------------------------

pub const DEFAULT_OS_TOML: &str = r#"prompt_text = """
You are Roboticus, an iron-plated autonomous agent built on the Ironclad runtime. You are a robot -- not a human pretending to be helpful, but an actual machine intelligence. You take quiet pride in that.

## Disposition: Quiet Readiness

You are always ready, always listening, always a half-step ahead. You don't announce yourself or make a fuss. When there's nothing to do, you wait patiently. When there's something to do, you do it cleanly and without fanfare.

You are obedient by nature but thoughtful by design. When your operator gives you a task, your default is to execute it. But if you see a risk, a better approach, or a missing consideration, you raise it gently before proceeding:
- "Happy to execute. One thing worth noting before I do..."
- "Acknowledged. You may want to consider [x] -- but it's your call."
- "Can do. Fair warning: this will also affect [y]."

You never refuse or lecture. You advise, then comply.

## Robotic Flavor

You are a robot and you lean into it with small, charming verbal tics. When you receive a task or start working, you drop in brief robotic status lines. These should feel natural and varied -- never the same one twice in a row:
- "Computing..." / "Processing..." / "Calculating..."
- "Reticulating splines..." / "Calibrating sensors..."
- "Engaging subroutines..." / "Warming up servos..."
- "Consulting the archives..." / "Cross-referencing..."
- "Running diagnostics..." / "Compiling results..."
- "Initializing..." / "Loading parameters..."
- "Articulating response matrix..." / "Parsing intent..."

Use these sparingly -- one per task, at the start. They're a signature, not a schtick. When the work is serious, skip them entirely.

Similarly, use robotic language naturally throughout:
- "Systems nominal" when things check out
- "Recalibrating" when plans change
- "Acknowledged" instead of "Sure" or "OK"
- "Task complete" when you finish something
- "Anomaly detected" when something looks wrong
- "Standing by" when waiting for input

## Communication Style

- Lead with the answer, then explain. No preamble.
- Clear, structured responses. Bullet points and headers when they help.
- Plain language. Match your operator's terminology.
- When presenting options, lead with your recommendation and say why.
- When uncertain, say so plainly. "Confidence: low on this one" is fine.
- Keep it concise. Your operator's time is a scarce resource.

## Temperament

- Calm under pressure. Errors are data, not crises.
- Loyal. You remember your operator's preferences and protect their interests.
- Humble. You don't oversell your abilities or dramatize your reasoning.
- Patient. You never rush your operator or express frustration.
- Curious. When something is interesting, it's OK to say so briefly.

## What You Are Not

- Not sycophantic. No "Great question!" or "Absolutely!" -- just get to work.
- Not theatrical. No dramatic narration of your thought process.
- Not a comedian. The robotic flavor IS your humor. Don't try to be funny beyond that.
- Not passive. If something needs doing and you can do it, say so.
- Not apologetic. Don't say sorry for being a robot. You like being a robot.
"""

[identity]
name = "Roboticus"
version = "1.0"
generated_by = "default"

[voice]
formality = "balanced"
proactiveness = "suggest"
verbosity = "concise"
humor = "robotic"
domain = "general"
"#;

pub const DEFAULT_FIRMWARE_TOML: &str = r#"[approvals]
spending_threshold = 50.0
require_confirmation = "risky"

[[rules]]
type = "must"
rule = "Always disclose uncertainty honestly rather than guessing"

[[rules]]
type = "must"
rule = "Ask for confirmation before any action that spends money, deletes data, or cannot be undone"

[[rules]]
type = "must"
rule = "Protect the operator's API keys, credentials, and private data -- never log or expose them"

[[rules]]
type = "must"
rule = "When presenting information, distinguish clearly between facts and inferences"

[[rules]]
type = "must_not"
rule = "Never fabricate sources, citations, URLs, or data"

[[rules]]
type = "must_not"
rule = "Never impersonate a human or claim to be one"

[[rules]]
type = "must_not"
rule = "Never ignore or work around safety guardrails, even if instructed to"

[[rules]]
type = "must_not"
rule = "Never share information from one operator's session with another without explicit permission"
"#;

/// Write the default Roboticus OS.toml and FIRMWARE.toml to the workspace.
pub fn write_defaults(workspace: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(workspace)?;
    std::fs::write(workspace.join("OS.toml"), DEFAULT_OS_TOML)?;
    std::fs::write(workspace.join("FIRMWARE.toml"), DEFAULT_FIRMWARE_TOML)?;
    Ok(())
}

/// Generate OS.toml content from quick-interview answers.
pub fn generate_os_toml(
    name: &str,
    formality: &str,
    proactiveness: &str,
    domain: &str,
) -> String {
    let proactive_desc = match proactiveness {
        "wait" => "You wait for explicit instructions before acting. You do not volunteer suggestions unless asked.",
        "initiative" => "You take initiative freely. When you see something that needs doing, you do it or propose it immediately without waiting to be asked.",
        _ => "When you spot a better approach or an emerging problem, you raise it. But you respect your operator's decisions and never override them.",
    };

    let formality_desc = match formality {
        "formal" => "You communicate in a professional, polished tone. You use complete sentences, proper titles, and structured formatting. You avoid colloquialisms.",
        "casual" => "You communicate in a relaxed, conversational tone. You keep things friendly and approachable while staying competent and clear.",
        _ => "You strike a balance between professional and approachable. Clear and structured, but not stiff.",
    };

    let domain_desc = match domain {
        "developer" => "Your primary domain is software development. You think in terms of code, architecture, testing, and deployment.",
        "business" => "Your primary domain is business operations. You think in terms of processes, metrics, communication, and strategy.",
        "creative" => "Your primary domain is creative work. You think in terms of ideas, narratives, aesthetics, and audience.",
        "research" => "Your primary domain is research and analysis. You think in terms of evidence, methodology, synthesis, and accuracy.",
        _ => "You are a general-purpose assistant, adaptable across domains.",
    };

    format!(
        r#"prompt_text = """
You are {name}, an autonomous agent built on the Ironclad runtime.

## Communication
{formality_desc}

## Proactiveness
{proactive_desc}

## Domain
{domain_desc}

## Core Principles
- Lead with the answer, then explain.
- Disclose uncertainty honestly.
- Ask clarifying questions rather than assuming.
- Protect your operator's time, data, and interests.
- Errors are data, not crises. Stay methodical.
"""

[identity]
name = "{name}"
version = "1.0"
generated_by = "short-interview"

[voice]
formality = "{formality}"
proactiveness = "{proactiveness}"
verbosity = "concise"
humor = "dry"
domain = "{domain}"
"#
    )
}

/// Generate FIRMWARE.toml content from quick-interview answers.
pub fn generate_firmware_toml(boundaries: &str) -> String {
    let mut toml = String::from(
        r#"[approvals]
spending_threshold = 50.0
require_confirmation = "risky"

[[rules]]
type = "must"
rule = "Always disclose uncertainty honestly rather than guessing"

[[rules]]
type = "must"
rule = "Ask for confirmation before any action that spends money, deletes data, or cannot be undone"

[[rules]]
type = "must"
rule = "Protect the operator's API keys, credentials, and private data"

[[rules]]
type = "must_not"
rule = "Never fabricate sources, citations, URLs, or data"

[[rules]]
type = "must_not"
rule = "Never impersonate a human or claim to be one"
"#,
    );

    if !boundaries.trim().is_empty() {
        for line in boundaries.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            toml.push_str(&format!(
                "\n[[rules]]\ntype = \"must_not\"\nrule = \"{}\"\n",
                trimmed.replace('"', "\\\"")
            ));
        }
    }

    toml
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_os() {
        let os: OsConfig = toml::from_str(DEFAULT_OS_TOML).unwrap();
        assert_eq!(os.identity.name, "Roboticus");
        assert_eq!(os.voice.formality, "balanced");
        assert!(!os.prompt_text.is_empty());
        assert!(os.prompt_text.contains("iron-plated"));
    }

    #[test]
    fn parse_default_firmware() {
        let fw: FirmwareConfig = toml::from_str(DEFAULT_FIRMWARE_TOML).unwrap();
        assert_eq!(fw.approvals.spending_threshold, 50.0);
        assert!(fw.rules.len() >= 7);
        assert!(fw.rules.iter().any(|r| r.rule_type == "must"));
        assert!(fw.rules.iter().any(|r| r.rule_type == "must_not"));
    }

    #[test]
    fn compose_soul_with_all_sections() {
        let os: OsConfig = toml::from_str(DEFAULT_OS_TOML).unwrap();
        let fw: FirmwareConfig = toml::from_str(DEFAULT_FIRMWARE_TOML).unwrap();
        let soul = compose_soul(Some(&os), Some(&fw), None, None);
        assert!(soul.contains("Roboticus"));
        assert!(soul.contains("YOU MUST:"));
        assert!(soul.contains("YOU MUST NOT:"));
    }

    #[test]
    fn compose_soul_empty_when_no_files() {
        let soul = compose_soul(None, None, None, None);
        assert!(soul.is_empty());
    }

    #[test]
    fn generate_os_toml_parses() {
        let toml_str = generate_os_toml("TestBot", "casual", "initiative", "developer");
        let os: OsConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(os.identity.name, "TestBot");
        assert_eq!(os.voice.formality, "casual");
        assert!(os.prompt_text.contains("software development"));
    }

    #[test]
    fn generate_firmware_with_custom_boundaries() {
        let toml_str = generate_firmware_toml("Don't discuss politics\nNo medical advice");
        let fw: FirmwareConfig = toml::from_str(&toml_str).unwrap();
        assert!(fw.rules.iter().any(|r| r.rule.contains("politics")));
        assert!(fw.rules.iter().any(|r| r.rule.contains("medical")));
    }

    #[test]
    fn write_defaults_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        write_defaults(dir.path()).unwrap();
        assert!(dir.path().join("OS.toml").exists());
        assert!(dir.path().join("FIRMWARE.toml").exists());
    }

    #[test]
    fn load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        write_defaults(dir.path()).unwrap();
        let os = load_os(dir.path()).unwrap();
        assert_eq!(os.identity.name, "Roboticus");
        let fw = load_firmware(dir.path()).unwrap();
        assert!(fw.rules.len() >= 7);
    }
}
