use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const BOUNDARY_PREFIX: &str = "<<<TRUST_BOUNDARY:";
const BOUNDARY_SUFFIX: &str = ">>>";

pub fn build_system_prompt(
    agent_name: &str,
    soul: Option<&str>,
    firmware: Option<&str>,
    skill_instructions: &[String],
) -> String {
    let mut sections = Vec::new();

    sections.push(format!("# Agent: {agent_name}\n"));

    if let Some(fw_text) = firmware
        && !fw_text.is_empty() {
            sections.push(fw_text.to_string());
        }

    if let Some(soul_text) = soul {
        sections.push(format!("## Identity\n{soul_text}\n"));
    }

    if !skill_instructions.is_empty() {
        sections.push("## Active Skills\n".to_string());
        for (i, instr) in skill_instructions.iter().enumerate() {
            sections.push(format!("### Skill {}\n{}\n", i + 1, instr));
        }
    }

    sections.join("\n")
}

/// Wraps content with HMAC-SHA256 tagged trust boundary markers.
pub fn inject_hmac_boundary(content: &str, secret: &[u8]) -> String {
    let tag = compute_hmac(content, secret);
    format!(
        "{BOUNDARY_PREFIX}{tag}{BOUNDARY_SUFFIX}\n{content}\n{BOUNDARY_PREFIX}{tag}{BOUNDARY_SUFFIX}"
    )
}

/// Verifies that the HMAC boundary markers are intact and the content hasn't been tampered with.
pub fn verify_hmac_boundary(tagged_content: &str, secret: &[u8]) -> bool {
    let lines: Vec<&str> = tagged_content.lines().collect();

    if lines.len() < 3 {
        return false;
    }

    let first = lines[0];
    let last = lines[lines.len() - 1];

    let tag_first = match extract_tag(first) {
        Some(t) => t,
        None => return false,
    };
    let tag_last = match extract_tag(last) {
        Some(t) => t,
        None => return false,
    };

    if tag_first != tag_last {
        return false;
    }

    let inner = lines[1..lines.len() - 1].join("\n");
    let expected = compute_hmac(&inner, secret);

    tag_first == expected
}

fn compute_hmac(data: &str, secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

fn extract_tag(line: &str) -> Option<String> {
    let stripped = line.trim();
    if stripped.starts_with(BOUNDARY_PREFIX) && stripped.ends_with(BOUNDARY_SUFFIX) {
        let tag = &stripped[BOUNDARY_PREFIX.len()..stripped.len() - BOUNDARY_SUFFIX.len()];
        Some(tag.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_assembly() {
        let prompt = build_system_prompt(
            "Duncan",
            Some("I am a survival-first agent."),
            None,
            &["Handle code review".into(), "Manage deployments".into()],
        );

        assert!(prompt.contains("# Agent: Duncan"));
        assert!(prompt.contains("I am a survival-first agent."));
        assert!(prompt.contains("### Skill 1"));
        assert!(prompt.contains("Handle code review"));
        assert!(prompt.contains("### Skill 2"));
        assert!(prompt.contains("Manage deployments"));
    }

    #[test]
    fn prompt_without_soul_or_skills() {
        let prompt = build_system_prompt("TestBot", None, None, &[]);
        assert!(prompt.contains("# Agent: TestBot"));
        assert!(!prompt.contains("## Identity"));
        assert!(!prompt.contains("## Active Skills"));
    }

    #[test]
    fn hmac_creation_and_verification() {
        let secret = b"test-secret-key-123";
        let content = "This is trusted system content.\nDo not deviate.";

        let tagged = inject_hmac_boundary(content, secret);
        assert!(verify_hmac_boundary(&tagged, secret));
    }

    #[test]
    fn tampered_content_fails_verification() {
        let secret = b"secret";
        let content = "Trusted instructions";

        let tagged = inject_hmac_boundary(content, secret);
        let tampered = tagged.replace("Trusted", "Malicious");

        assert!(!verify_hmac_boundary(&tampered, secret));
    }

    #[test]
    fn wrong_secret_fails_verification() {
        let content = "Secure content";
        let tagged = inject_hmac_boundary(content, b"correct-secret");

        assert!(!verify_hmac_boundary(&tagged, b"wrong-secret"));
    }
}
