use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use ironclad_core::{InputAuthority, PolicyDecision, RiskLevel, SurvivalTier};

fn collect_string_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(arr) => {
            for v in arr {
                collect_string_values(v, out);
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                collect_string_values(v, out);
            }
        }
        _ => {}
    }
}

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

/// Priority 3: blocks high-value or sensitive financial operations.
pub struct FinancialRule {
    /// Maximum single transfer in dollars; transfers above are denied.
    pub threshold_dollars: f64,
}

impl Default for FinancialRule {
    fn default() -> Self {
        Self {
            threshold_dollars: 100.0,
        }
    }
}

impl FinancialRule {
    pub fn new(threshold_dollars: f64) -> Self {
        Self { threshold_dollars }
    }

    fn is_financial_tool(name: &str) -> bool {
        let name_lower = name.to_lowercase();
        [
            "transfer", "send", "withdraw", "deposit", "payment", "wallet",
        ]
        .iter()
        .any(|k| name_lower.contains(k))
    }

    fn extract_amount_cents(params: &Value) -> Option<i64> {
        let obj = params.as_object()?;
        for key in ["amount", "amount_cents", "cents", "value_cents"] {
            if let Some(v) = obj.get(key)
                && let Some(n) = v.as_i64()
            {
                return Some(n);
            }
        }
        if let Some(v) = obj
            .get("amount_dollars")
            .or(obj.get("dollars"))
            .or(obj.get("value"))
            && let Some(n) = v.as_f64()
        {
            return Some((n * 100.0).round() as i64);
        }
        None
    }

    fn is_wallet_config_or_drain(params: &Value) -> bool {
        let obj = match params.as_object() {
            Some(o) => o,
            None => return false,
        };
        let drain_keys = [
            "drain",
            "withdraw_all",
            "export_private_key",
            "set_wallet_path",
        ];
        for key in drain_keys {
            if obj.contains_key(key) {
                return true;
            }
        }
        false
    }
}

impl PolicyRule for FinancialRule {
    fn name(&self) -> &str {
        "financial"
    }

    fn priority(&self) -> u32 {
        3
    }

    fn evaluate(&self, call: &ToolCallRequest, _ctx: &PolicyContext) -> PolicyDecision {
        if !Self::is_financial_tool(&call.tool_name) {
            return PolicyDecision::Allow;
        }
        if Self::is_wallet_config_or_drain(&call.params) {
            return PolicyDecision::Deny {
                rule: self.name().into(),
                reason: "tool attempts to change wallet config or drain funds".into(),
            };
        }
        let threshold_cents = (self.threshold_dollars * 100.0).round() as i64;
        if let Some(cents) = Self::extract_amount_cents(&call.params)
            && cents > threshold_cents
        {
            return PolicyDecision::Deny {
                rule: self.name().into(),
                reason: format!(
                    "amount {} cents exceeds threshold ${:.2}",
                    cents, self.threshold_dollars
                ),
            };
        }
        PolicyDecision::Allow
    }
}

/// Priority 4: blocks access to protected path patterns.
pub struct PathProtectionRule {
    /// Path patterns that are not allowed in tool arguments.
    pub protected: Vec<String>,
}

impl Default for PathProtectionRule {
    fn default() -> Self {
        Self {
            protected: vec![
                "/etc/".into(),
                ".env".into(),
                "wallet.json".into(),
                "private_key".into(),
                ".ssh/".into(),
                "ironclad.toml".into(),
            ],
        }
    }
}

impl PathProtectionRule {
    pub fn new(protected: Vec<String>) -> Self {
        Self { protected }
    }

    fn matches_protected(&self, s: &str) -> Option<&str> {
        let s_lower = s.to_lowercase();
        for pattern in &self.protected {
            let p_lower = pattern.to_lowercase();
            if s_lower.contains(&p_lower) || s_lower.ends_with(p_lower.trim_end_matches('/')) {
                return Some(pattern);
            }
        }
        None
    }
}

impl PolicyRule for PathProtectionRule {
    fn name(&self) -> &str {
        "path_protection"
    }

    fn priority(&self) -> u32 {
        4
    }

    fn evaluate(&self, call: &ToolCallRequest, _ctx: &PolicyContext) -> PolicyDecision {
        let mut strings = Vec::new();
        collect_string_values(&call.params, &mut strings);
        for s in &strings {
            if let Some(pattern) = self.matches_protected(s) {
                return PolicyDecision::Deny {
                    rule: self.name().into(),
                    reason: format!("protected path pattern '{}' not allowed", pattern),
                };
            }
        }
        PolicyDecision::Allow
    }
}

/// Priority 5: rate-limits tool calls per tool name.
pub struct RateLimitRule {
    max_calls_per_minute: u32,
    /// tool_name -> timestamps of recent calls
    calls: Mutex<std::collections::HashMap<String, Vec<Instant>>>,
}

impl Default for RateLimitRule {
    fn default() -> Self {
        Self {
            max_calls_per_minute: 30,
            calls: Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl RateLimitRule {
    pub fn new(max_calls_per_minute: u32) -> Self {
        Self {
            max_calls_per_minute,
            calls: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn prune_older_than(cuts: &mut Vec<Instant>, cutoff: Instant) {
        cuts.retain(|&t| t > cutoff);
    }
}

impl PolicyRule for RateLimitRule {
    fn name(&self) -> &str {
        "rate_limit"
    }

    fn priority(&self) -> u32 {
        5
    }

    fn evaluate(&self, call: &ToolCallRequest, _ctx: &PolicyContext) -> PolicyDecision {
        let now = Instant::now();
        let window_start = now - Duration::from_secs(60);
        let mut guard = self.calls.lock().expect("rate limit mutex");
        let cuts = guard.entry(call.tool_name.clone()).or_default();
        Self::prune_older_than(cuts, window_start);
        if cuts.len() >= self.max_calls_per_minute as usize {
            return PolicyDecision::Deny {
                rule: self.name().into(),
                reason: format!(
                    "tool '{}' rate limit exceeded (max {} per minute)",
                    call.tool_name, self.max_calls_per_minute
                ),
            };
        }
        cuts.push(now);
        PolicyDecision::Allow
    }
}

/// Priority 6: validates argument size and blocks malicious patterns.
pub struct ValidationRule;

const MAX_ARG_SIZE_BYTES: usize = 100 * 1024; // 100KB

impl ValidationRule {
    fn serialized_size(value: &Value) -> usize {
        value.to_string().len()
    }

    fn looks_malicious(s: &str) -> bool {
        let s_lower = s.to_lowercase();
        // Shell injection
        if s.contains('$') && (s.contains('(') || s.contains('`') || s.contains("${")) {
            return true;
        }
        if s.contains("; ")
            && (s_lower.contains("rm ") || s_lower.contains("curl ") || s_lower.contains("wget "))
        {
            return true;
        }
        // Path traversal
        if s.contains("..") && (s.contains('/') || s.contains('\\')) {
            return true;
        }
        false
    }
}

impl PolicyRule for ValidationRule {
    fn name(&self) -> &str {
        "validation"
    }

    fn priority(&self) -> u32 {
        6
    }

    fn evaluate(&self, call: &ToolCallRequest, _ctx: &PolicyContext) -> PolicyDecision {
        if Self::serialized_size(&call.params) > MAX_ARG_SIZE_BYTES {
            return PolicyDecision::Deny {
                rule: self.name().into(),
                reason: format!(
                    "arguments exceed maximum size ({} bytes)",
                    MAX_ARG_SIZE_BYTES
                ),
            };
        }
        let mut strings = Vec::new();
        collect_string_values(&call.params, &mut strings);
        for s in &strings {
            if Self::looks_malicious(s) {
                return PolicyDecision::Deny {
                    rule: self.name().into(),
                    reason: "arguments contain potentially malicious pattern (shell injection or path traversal)".into(),
                };
            }
        }
        PolicyDecision::Allow
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

    #[test]
    fn financial_rule_blocks_high_value_allows_low() {
        let rule = FinancialRule::new(100.0);
        let ctx = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::Normal,
        };

        let low = ToolCallRequest {
            tool_name: "transfer".into(),
            params: serde_json::json!({ "amount_cents": 5000 }),
            risk_level: RiskLevel::Safe,
        };
        assert!(rule.evaluate(&low, &ctx).is_allowed());

        let high = ToolCallRequest {
            tool_name: "send".into(),
            params: serde_json::json!({ "amount_dollars": 150.0 }),
            risk_level: RiskLevel::Safe,
        };
        assert!(!rule.evaluate(&high, &ctx).is_allowed());

        let non_financial = ToolCallRequest {
            tool_name: "read_file".into(),
            params: serde_json::json!({ "path": "/tmp/foo" }),
            risk_level: RiskLevel::Safe,
        };
        assert!(rule.evaluate(&non_financial, &ctx).is_allowed());
    }

    #[test]
    fn financial_rule_blocks_wallet_drain() {
        let rule = FinancialRule::default();
        let ctx = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::Normal,
        };

        let drain = ToolCallRequest {
            tool_name: "wallet_export".into(),
            params: serde_json::json!({ "export_private_key": true }),
            risk_level: RiskLevel::Safe,
        };
        assert!(!rule.evaluate(&drain, &ctx).is_allowed());
    }

    #[test]
    fn path_protection_blocks_env_allows_normal() {
        let rule = PathProtectionRule::default();
        let ctx = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::Normal,
        };

        let blocked = ToolCallRequest {
            tool_name: "read_file".into(),
            params: serde_json::json!({ "path": "/app/.env" }),
            risk_level: RiskLevel::Safe,
        };
        let decision = rule.evaluate(&blocked, &ctx);
        assert!(!decision.is_allowed());
        if let PolicyDecision::Deny { reason, .. } = &decision {
            assert!(reason.contains(".env") || reason.contains("protected"));
        }

        let allowed = ToolCallRequest {
            tool_name: "read_file".into(),
            params: serde_json::json!({ "path": "/tmp/foo.txt" }),
            risk_level: RiskLevel::Safe,
        };
        assert!(rule.evaluate(&allowed, &ctx).is_allowed());
    }

    #[test]
    fn rate_limit_blocks_over_limit_allows_under() {
        let rule = RateLimitRule::new(2);
        let ctx = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::Normal,
        };

        let req = |tool: &str| ToolCallRequest {
            tool_name: tool.into(),
            params: serde_json::json!({}),
            risk_level: RiskLevel::Safe,
        };

        assert!(rule.evaluate(&req("foo"), &ctx).is_allowed());
        assert!(rule.evaluate(&req("foo"), &ctx).is_allowed());
        assert!(!rule.evaluate(&req("foo"), &ctx).is_allowed());

        assert!(rule.evaluate(&req("bar"), &ctx).is_allowed());
    }

    #[test]
    fn validation_rejects_oversized_and_malicious() {
        let rule = ValidationRule;
        let ctx = PolicyContext {
            authority: InputAuthority::Creator,
            survival_tier: SurvivalTier::Normal,
        };

        let huge = ToolCallRequest {
            tool_name: "echo".into(),
            params: serde_json::json!({ "data": "x".repeat(101 * 1024) }),
            risk_level: RiskLevel::Safe,
        };
        assert!(!rule.evaluate(&huge, &ctx).is_allowed());

        let shell_injection = ToolCallRequest {
            tool_name: "run".into(),
            params: serde_json::json!({ "cmd": "$(rm -rf /)" }),
            risk_level: RiskLevel::Safe,
        };
        assert!(!rule.evaluate(&shell_injection, &ctx).is_allowed());

        let path_traversal = ToolCallRequest {
            tool_name: "read".into(),
            params: serde_json::json!({ "path": "../../../etc/passwd" }),
            risk_level: RiskLevel::Safe,
        };
        assert!(!rule.evaluate(&path_traversal, &ctx).is_allowed());

        let ok = ToolCallRequest {
            tool_name: "echo".into(),
            params: serde_json::json!({ "msg": "hello" }),
            risk_level: RiskLevel::Safe,
        };
        assert!(rule.evaluate(&ok, &ctx).is_allowed());
    }
}
