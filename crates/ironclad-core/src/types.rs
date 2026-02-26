use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurvivalTier {
    High,
    Normal,
    LowCompute,
    Critical,
    Dead,
}

impl SurvivalTier {
    pub fn from_balance(usd: f64, hours_below_zero: f64) -> Self {
        if usd < 0.0 && hours_below_zero >= 0.999 {
            Self::Dead
        } else if usd < 0.10 {
            Self::Critical
        } else if usd < 0.50 {
            Self::LowCompute
        } else if usd < 5.00 {
            Self::Normal
        } else {
            Self::High
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Setup,
    Waking,
    Running,
    Sleeping,
    Dead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiFormat {
    AnthropicMessages,
    OpenAiCompletions,
    OpenAiResponses,
    GoogleGenerativeAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModelTier {
    T1,
    T2,
    T3,
    T4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny { rule: String, reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Safe,
    Caution,
    Dangerous,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillKind {
    Structured,
    Instruction,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillTrigger {
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub tool_names: Vec<String>,
    #[serde(default)]
    pub regex_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub kind: SkillKind,
    pub triggers: SkillTrigger,
    #[serde(default = "default_priority")]
    pub priority: u32,
    pub tool_chain: Option<Vec<ToolChainStep>>,
    pub policy_overrides: Option<serde_json::Value>,
    pub script_path: Option<PathBuf>,
    #[serde(default = "default_risk_level")]
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChainStep {
    pub tool_name: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionSkill {
    pub name: String,
    pub description: String,
    pub triggers: SkillTrigger,
    #[serde(default = "default_priority")]
    pub priority: u32,
    pub body: String,
}

fn default_priority() -> u32 {
    5
}

fn default_risk_level() -> RiskLevel {
    RiskLevel::Caution
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputAuthority {
    Creator,
    SelfGenerated,
    Peer,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleKind {
    Cron,
    Every,
    At,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survival_tier_thresholds() {
        assert_eq!(SurvivalTier::from_balance(10.0, 0.0), SurvivalTier::High);
        assert_eq!(SurvivalTier::from_balance(5.0, 0.0), SurvivalTier::High);
        assert_eq!(SurvivalTier::from_balance(4.99, 0.0), SurvivalTier::Normal);
        assert_eq!(SurvivalTier::from_balance(0.50, 0.0), SurvivalTier::Normal);
        assert_eq!(
            SurvivalTier::from_balance(0.49, 0.0),
            SurvivalTier::LowCompute
        );
        assert_eq!(
            SurvivalTier::from_balance(0.10, 0.0),
            SurvivalTier::LowCompute
        );
        assert_eq!(
            SurvivalTier::from_balance(0.09, 0.0),
            SurvivalTier::Critical
        );
        assert_eq!(SurvivalTier::from_balance(0.0, 0.0), SurvivalTier::Critical);
        assert_eq!(
            SurvivalTier::from_balance(-1.0, 0.5),
            SurvivalTier::Critical
        );
        assert_eq!(SurvivalTier::from_balance(-1.0, 1.0), SurvivalTier::Dead);
    }

    #[test]
    fn policy_decision_helpers() {
        assert!(PolicyDecision::Allow.is_allowed());
        let deny = PolicyDecision::Deny {
            rule: "test".into(),
            reason: "no".into(),
        };
        assert!(!deny.is_allowed());
    }

    #[test]
    fn risk_level_ordering() {
        assert!(RiskLevel::Safe < RiskLevel::Caution);
        assert!(RiskLevel::Caution < RiskLevel::Dangerous);
        assert!(RiskLevel::Dangerous < RiskLevel::Forbidden);
    }

    #[test]
    fn skill_trigger_default() {
        let trigger = SkillTrigger::default();
        assert!(trigger.keywords.is_empty());
        assert!(trigger.tool_names.is_empty());
        assert!(trigger.regex_patterns.is_empty());
    }

    #[test]
    fn api_format_roundtrip() {
        let formats = [
            ApiFormat::AnthropicMessages,
            ApiFormat::OpenAiCompletions,
            ApiFormat::OpenAiResponses,
            ApiFormat::GoogleGenerativeAi,
        ];
        for fmt in formats {
            let json = serde_json::to_string(&fmt).unwrap();
            let back: ApiFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(fmt, back);
        }
    }

    #[test]
    fn model_tier_ordering() {
        assert!(ModelTier::T1 < ModelTier::T2);
        assert!(ModelTier::T2 < ModelTier::T3);
        assert!(ModelTier::T3 < ModelTier::T4);
    }

    #[test]
    fn agent_state_serde() {
        for state in [
            AgentState::Setup,
            AgentState::Waking,
            AgentState::Running,
            AgentState::Sleeping,
            AgentState::Dead,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let back: AgentState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, back);
        }
    }

    #[test]
    fn skill_kind_serde() {
        let structured = serde_json::to_string(&SkillKind::Structured).unwrap();
        assert_eq!(structured, "\"Structured\"");
        let instruction = serde_json::to_string(&SkillKind::Instruction).unwrap();
        assert_eq!(instruction, "\"Instruction\"");
    }

    #[test]
    fn schedule_kind_serde() {
        for kind in [ScheduleKind::Cron, ScheduleKind::Every, ScheduleKind::At] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: ScheduleKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn input_authority_serde() {
        for auth in [
            InputAuthority::Creator,
            InputAuthority::SelfGenerated,
            InputAuthority::Peer,
            InputAuthority::External,
        ] {
            let json = serde_json::to_string(&auth).unwrap();
            let back: InputAuthority = serde_json::from_str(&json).unwrap();
            assert_eq!(auth, back);
        }
    }
}
