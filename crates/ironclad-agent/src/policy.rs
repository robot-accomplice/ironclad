use serde_json::Value;

use ironclad_core::{InputAuthority, PolicyDecision, RiskLevel, SurvivalTier};

pub trait PolicyRule: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> u32;
    fn evaluate(&self, call: &ToolCallRequest, ctx: &PolicyContext) -> PolicyDecision;
}

#[derive(Debug, Clone)]
pub struct PolicyContext {
    pub authority: InputAuthority,
    pub survival_tier: SurvivalTier,
}

#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub tool_name: String,
    pub params: Value,
    pub risk_level: RiskLevel,
}

pub struct PolicyEngine {
    rules: Vec<Box<dyn PolicyRule>>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(&mut self, rule: Box<dyn PolicyRule>) {
        self.rules.push(rule);
        self.rules.sort_by_key(|r| r.priority());
    }

    pub fn evaluate_all(&self, call: &ToolCallRequest, ctx: &PolicyContext) -> PolicyDecision {
        for rule in &self.rules {
            let decision = rule.evaluate(call, ctx);
            if !decision.is_allowed() {
                return decision;
            }
        }
        PolicyDecision::Allow
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Priority 1: restricts tool access based on input authority level.
pub struct AuthorityRule;

impl PolicyRule for AuthorityRule {
    fn name(&self) -> &str {
        "authority"
    }

    fn priority(&self) -> u32 {
        1
    }

    fn evaluate(&self, call: &ToolCallRequest, ctx: &PolicyContext) -> PolicyDecision {
        let allowed = match ctx.authority {
            InputAuthority::Creator => true,
            InputAuthority::SelfGenerated => call.risk_level <= RiskLevel::Dangerous,
            InputAuthority::Peer => call.risk_level <= RiskLevel::Caution,
            InputAuthority::External => call.risk_level <= RiskLevel::Safe,
        };

        if allowed {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny {
                rule: self.name().into(),
                reason: format!(
                    "{:?} authority cannot use {:?}-level tool '{}'",
                    ctx.authority, call.risk_level, call.tool_name
                ),
            }
        }
    }
}

/// Priority 2: blocks Forbidden-risk tools unconditionally.
pub struct CommandSafetyRule;

impl PolicyRule for CommandSafetyRule {
    fn name(&self) -> &str {
        "command_safety"
    }

    fn priority(&self) -> u32 {
        2
    }

    fn evaluate(&self, call: &ToolCallRequest, _ctx: &PolicyContext) -> PolicyDecision {
        if call.risk_level == RiskLevel::Forbidden {
            PolicyDecision::Deny {
                rule: self.name().into(),
                reason: format!("tool '{}' is forbidden", call.tool_name),
            }
        } else {
            PolicyDecision::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(tool: &str, risk: RiskLevel) -> ToolCallRequest {
        ToolCallRequest {
            tool_name: tool.into(),
            params: serde_json::json!({}),
            risk_level: risk,
        }
    }

    #[test]
    fn authority_based_blocking() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Box::new(AuthorityRule));

        let ctx_external = PolicyContext {
            authority: InputAuthority::External,
            survival_tier: SurvivalTier::Normal,
        };

        assert!(
            engine
                .evaluate_all(&make_request("echo", RiskLevel::Safe), &ctx_external)
                .is_allowed()
        );

        assert!(
            !engine
                .evaluate_all(&make_request("rm_file", RiskLevel::Caution), &ctx_external)
                .is_allowed()
        );

        let ctx_creator = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::Normal,
        };
        assert!(
            engine
                .evaluate_all(&make_request("nuke", RiskLevel::Dangerous), &ctx_creator)
                .is_allowed()
        );

        let ctx_self = PolicyContext {
            authority: InputAuthority::SelfGenerated,
            survival_tier: SurvivalTier::Normal,
        };
        assert!(
            engine
                .evaluate_all(&make_request("cmd", RiskLevel::Dangerous), &ctx_self)
                .is_allowed()
        );
        assert!(
            !engine
                .evaluate_all(&make_request("cmd", RiskLevel::Forbidden), &ctx_self)
                .is_allowed()
        );
    }

    #[test]
    fn command_safety_blocks_forbidden() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Box::new(CommandSafetyRule));

        let ctx = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::Normal,
        };

        assert!(
            !engine
                .evaluate_all(&make_request("evil", RiskLevel::Forbidden), &ctx)
                .is_allowed()
        );
        assert!(
            engine
                .evaluate_all(&make_request("good", RiskLevel::Dangerous), &ctx)
                .is_allowed()
        );
    }

    #[test]
    fn allow_pass_through() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Box::new(AuthorityRule));
        engine.add_rule(Box::new(CommandSafetyRule));

        let ctx = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::High,
        };

        let decision = engine.evaluate_all(&make_request("read_file", RiskLevel::Safe), &ctx);
        assert!(decision.is_allowed());
    }
}
