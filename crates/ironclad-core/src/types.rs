use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::Result;

/// Trait for handling HTTP 402 Payment Required responses (x402 protocol).
///
/// Implementors receive the JSON response body from a 402 response and must
/// return a payment header string (e.g. `"x402 amount=... recipient=... auth=..."`)
/// that will be attached to the retry request.
///
/// This trait lives in `ironclad-core` so that `ironclad-llm` (the HTTP client)
/// can accept a handler without depending on `ironclad-wallet` directly.
pub trait PaymentHandler: Send + Sync {
    fn handle_payment_required(
        &self,
        response_body: &serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>>;
}

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
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_author")]
    pub author: String,
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
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_author")]
    pub author: String,
}

fn default_priority() -> u32 {
    5
}

fn default_risk_level() -> RiskLevel {
    RiskLevel::Caution
}

fn default_version() -> String {
    "0.0.0".into()
}

fn default_author() -> String {
    "local".into()
}

/// Authority level of the message sender, resolved from authentication layers.
///
/// Variant order matters: `External < Peer < SelfGenerated < Creator` via derived
/// `Ord`, enabling `min()`/`max()` in the claim composition algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InputAuthority {
    External,
    Peer,
    SelfGenerated,
    Creator,
}

// ── Claim-based RBAC types ──────────────────────────────────────────────────

/// Which authentication layer contributed a positive grant to a [`SecurityClaim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClaimSource {
    /// Sender passed the channel's allow-list (Telegram chat IDs, Discord guild IDs, etc.).
    ChannelAllowList,
    /// Sender matched `channels.trusted_sender_ids`.
    TrustedSenderId,
    /// HTTP API key authentication.
    ApiKey,
    /// WebSocket ticket authentication.
    WsTicket,
    /// A2A ECDH session key agreement.
    A2aSession,
    /// No authentication source — anonymous/default.
    Anonymous,
}

/// Immutable security principal resolved from all authentication layers.
///
/// Constructed by security claim resolvers such as
/// [`crate::security::resolve_channel_claim`] — callers receive it, they
/// cannot modify the authority decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityClaim {
    /// The resolved effective authority after claim composition.
    pub authority: InputAuthority,
    /// Which authentication layer(s) contributed positive grants.
    pub sources: Vec<ClaimSource>,
    /// The ceiling applied by negative restrictions (threat scanner, etc.).
    pub ceiling: InputAuthority,
    /// Whether the threat scanner applied a downgrade.
    pub threat_downgraded: bool,
    /// Original sender identifier for audit trail.
    pub sender_id: String,
    /// Channel that produced this claim.
    pub channel: String,
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

    #[test]
    fn input_authority_ordering() {
        assert!(InputAuthority::External < InputAuthority::Peer);
        assert!(InputAuthority::Peer < InputAuthority::SelfGenerated);
        assert!(InputAuthority::SelfGenerated < InputAuthority::Creator);

        // min/max work correctly for claim composition
        let grants = [InputAuthority::Peer, InputAuthority::Creator];
        assert_eq!(
            grants.iter().copied().max().unwrap(),
            InputAuthority::Creator
        );

        let ceilings = [InputAuthority::External, InputAuthority::Peer];
        assert_eq!(
            ceilings.iter().copied().min().unwrap(),
            InputAuthority::External
        );

        // min(grant, ceiling) caps authority
        let grant = InputAuthority::Creator;
        let ceiling = InputAuthority::External;
        assert_eq!(grant.min(ceiling), InputAuthority::External);
    }

    #[test]
    fn claim_source_serde() {
        for src in [
            ClaimSource::ChannelAllowList,
            ClaimSource::TrustedSenderId,
            ClaimSource::ApiKey,
            ClaimSource::WsTicket,
            ClaimSource::A2aSession,
            ClaimSource::Anonymous,
        ] {
            let json = serde_json::to_string(&src).unwrap();
            let back: ClaimSource = serde_json::from_str(&json).unwrap();
            assert_eq!(src, back);
        }
    }

    #[test]
    fn security_claim_serde() {
        let claim = SecurityClaim {
            authority: InputAuthority::Peer,
            sources: vec![ClaimSource::ChannelAllowList],
            ceiling: InputAuthority::Creator,
            threat_downgraded: false,
            sender_id: "12345".into(),
            channel: "telegram".into(),
        };
        let json = serde_json::to_string(&claim).unwrap();
        let back: SecurityClaim = serde_json::from_str(&json).unwrap();
        assert_eq!(back.authority, InputAuthority::Peer);
        assert_eq!(back.sources, vec![ClaimSource::ChannelAllowList]);
        assert!(!back.threat_downgraded);
    }

    #[test]
    fn default_priority_is_five() {
        assert_eq!(default_priority(), 5);
    }

    #[test]
    fn default_risk_level_is_caution() {
        assert_eq!(default_risk_level(), RiskLevel::Caution);
    }

    #[test]
    fn survival_tier_boundary_dead_needs_one_hour() {
        // Exactly at the 0.999 boundary
        assert_eq!(SurvivalTier::from_balance(-0.01, 0.999), SurvivalTier::Dead);
        // Just under the boundary
        assert_eq!(
            SurvivalTier::from_balance(-0.01, 0.998),
            SurvivalTier::Critical
        );
    }

    #[test]
    fn survival_tier_serde() {
        for tier in [
            SurvivalTier::High,
            SurvivalTier::Normal,
            SurvivalTier::LowCompute,
            SurvivalTier::Critical,
            SurvivalTier::Dead,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let back: SurvivalTier = serde_json::from_str(&json).unwrap();
            assert_eq!(tier, back);
        }
    }

    #[test]
    fn skill_manifest_serde() {
        let manifest = SkillManifest {
            name: "test".into(),
            description: "A test skill".into(),
            kind: SkillKind::Structured,
            triggers: SkillTrigger::default(),
            priority: 5,
            tool_chain: None,
            policy_overrides: None,
            script_path: None,
            risk_level: RiskLevel::Safe,
            version: "1.0.0".into(),
            author: "tester".into(),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: SkillManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "test");
        assert_eq!(back.risk_level, RiskLevel::Safe);
    }

    #[test]
    fn instruction_skill_serde() {
        let skill = InstructionSkill {
            name: "help".into(),
            description: "Provides help".into(),
            triggers: SkillTrigger::default(),
            priority: 3,
            body: "Help text".into(),
            version: "0.1.0".into(),
            author: "local".into(),
        };
        let json = serde_json::to_string(&skill).unwrap();
        let back: InstructionSkill = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "help");
        assert_eq!(back.priority, 3);
    }

    #[test]
    fn tool_chain_step_serde() {
        let step = ToolChainStep {
            tool_name: "git_commit".into(),
            params: serde_json::json!({"message": "fix"}),
        };
        let json = serde_json::to_string(&step).unwrap();
        let back: ToolChainStep = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool_name, "git_commit");
    }
}
