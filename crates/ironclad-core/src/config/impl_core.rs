impl IroncladConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_str(&contents)
    }

    /// Parse configuration from a TOML string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironclad_core::config::IroncladConfig;
    ///
    /// let toml = r#"
    /// [agent]
    /// name = "Test"
    /// id = "test-1"
    /// workspace = "/tmp"
    /// log_level = "info"
    ///
    /// [server]
    /// bind = "127.0.0.1"
    /// port = 3001
    ///
    /// [database]
    /// path = "/tmp/test.db"
    ///
    /// [models]
    /// primary = "ollama/qwen3:8b"
    /// "#;
    /// let config = IroncladConfig::from_str(toml).unwrap();
    /// assert_eq!(config.server.port, 3001);
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(toml_str: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(toml_str)?;
        config.normalize_paths();
        config.merge_bundled_providers();
        config.validate()?;
        Ok(config)
    }

    /// Expand home-relative (`~`) paths across all configured path fields.
    pub fn normalize_paths(&mut self) {
        self.database.path = expand_tilde(&self.database.path);
        self.agent.workspace = expand_tilde(&self.agent.workspace);
        self.server.log_dir = expand_tilde(&self.server.log_dir);
        self.skills.skills_dir = expand_tilde(&self.skills.skills_dir);
        self.wallet.path = expand_tilde(&self.wallet.path);
        self.plugins.dir = expand_tilde(&self.plugins.dir);
        self.browser.profile_dir = expand_tilde(&self.browser.profile_dir);
        self.daemon.pid_file = expand_tilde(&self.daemon.pid_file);
        self.multimodal.media_dir = self.multimodal.media_dir.as_ref().map(|p| expand_tilde(p));
        self.devices.identity_path = self.devices.identity_path.as_ref().map(|p| expand_tilde(p));

        if let Some(ref vp) = self.obsidian.vault_path {
            self.obsidian.vault_path = Some(expand_tilde(vp));
        }
        self.obsidian.auto_detect_paths = self
            .obsidian
            .auto_detect_paths
            .iter()
            .map(|p| expand_tilde(p))
            .collect();

        for source in &mut self.knowledge.sources {
            if let Some(ref p) = source.path {
                source.path = Some(expand_tilde(p));
            }
        }

        // Auto-populate tool_allowed_paths from feature configs so that
        // workspace_only mode doesn't block configured external paths.
        if self.obsidian.enabled
            && let Some(ref vp) = self.obsidian.vault_path
        {
            let canonical = vp.clone();
            if !self.security.filesystem.tool_allowed_paths.contains(&canonical) {
                self.security.filesystem.tool_allowed_paths.push(canonical);
            }
        }
    }

    fn merge_bundled_providers(&mut self) {
        let bundled: BundledProviders = toml::from_str(BUNDLED_PROVIDERS_TOML)
            .expect("bundled providers TOML must parse — this is a build-time error");
        for (name, bundled_cfg) in bundled.providers {
            self.providers.entry(name).or_insert(bundled_cfg);
        }
    }

    pub fn bundled_providers_toml() -> &'static str {
        BUNDLED_PROVIDERS_TOML
    }

    pub fn validate(&self) -> Result<()> {
        if self.models.primary.is_empty() {
            return Err(IroncladError::Config(
                "models.primary must be non-empty".into(),
            ));
        }

        if self.agent.id.is_empty() {
            return Err(IroncladError::Config("agent.id must be non-empty".into()));
        }

        if self.agent.name.is_empty() {
            return Err(IroncladError::Config("agent.name must be non-empty".into()));
        }
        if self.agent.autonomy_max_react_turns == 0 {
            return Err(IroncladError::Config(
                "agent.autonomy_max_react_turns must be >= 1".into(),
            ));
        }
        if self.agent.autonomy_max_turn_duration_seconds == 0 {
            return Err(IroncladError::Config(
                "agent.autonomy_max_turn_duration_seconds must be >= 1".into(),
            ));
        }

        if !matches!(self.session.scope_mode.as_str(), "agent" | "peer" | "group") {
            return Err(IroncladError::Config(format!(
                "session.scope_mode must be one of \"agent\", \"peer\", \"group\", got \"{}\"",
                self.session.scope_mode
            )));
        }

        let sum = self.memory.working_budget_pct
            + self.memory.episodic_budget_pct
            + self.memory.semantic_budget_pct
            + self.memory.procedural_budget_pct
            + self.memory.relationship_budget_pct;

        if (sum - 100.0).abs() > 0.01 {
            return Err(IroncladError::Config(format!(
                "memory budget percentages must sum to 100, got {sum}"
            )));
        }

        if self.treasury.per_payment_cap <= 0.0 {
            return Err(IroncladError::Config(
                "treasury.per_payment_cap must be positive".into(),
            ));
        }

        if self.treasury.minimum_reserve < 0.0 {
            return Err(IroncladError::Config(
                "treasury.minimum_reserve must be non-negative".into(),
            ));
        }
        if !self.security.deny_on_empty_allowlist {
            return Err(IroncladError::Config(
                "security.deny_on_empty_allowlist=false is no longer supported; run update or mechanic repair to migrate the config".into(),
            ));
        }
        if self.treasury.revenue_swap.target_symbol.trim().is_empty() {
            return Err(IroncladError::Config(
                "treasury.revenue_swap.target_symbol must be non-empty".into(),
            ));
        }
        if self.treasury.revenue_swap.default_chain.trim().is_empty() {
            return Err(IroncladError::Config(
                "treasury.revenue_swap.default_chain must be non-empty".into(),
            ));
        }
        let mut seen_revenue_swap_chains = std::collections::HashSet::new();
        for chain in &self.treasury.revenue_swap.chains {
            let normalized = chain.chain.trim().to_ascii_uppercase();
            if normalized.is_empty() {
                return Err(IroncladError::Config(
                    "treasury.revenue_swap.chains[].chain must be non-empty".into(),
                ));
            }
            if chain.target_contract_address.trim().is_empty() {
                return Err(IroncladError::Config(format!(
                    "treasury.revenue_swap.chains[{normalized}].target_contract_address must be non-empty"
                )));
            }
            if !seen_revenue_swap_chains.insert(normalized.clone()) {
                return Err(IroncladError::Config(format!(
                    "treasury.revenue_swap.chains contains duplicate chain '{normalized}'"
                )));
            }
        }
        if self.treasury.revenue_swap.enabled
            && !seen_revenue_swap_chains.contains(
                &self
                    .treasury
                    .revenue_swap
                    .default_chain
                    .trim()
                    .to_ascii_uppercase(),
            )
        {
            return Err(IroncladError::Config(
                "treasury.revenue_swap.default_chain must exist in treasury.revenue_swap.chains when enabled".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.self_funding.tax.rate) {
            return Err(IroncladError::Config(
                "self_funding.tax.rate must be between 0.0 and 1.0".into(),
            ));
        }
        if self.self_funding.tax.enabled
            && self
                .self_funding
                .tax
                .destination_wallet
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_none()
        {
            return Err(IroncladError::Config(
                "self_funding.tax.destination_wallet must be set when profit tax is enabled"
                    .into(),
            ));
        }

        if self.server.bind.parse::<std::net::IpAddr>().is_err() && self.server.bind != "localhost"
        {
            return Err(IroncladError::Config(format!(
                "server.bind '{}' is not a valid IP address",
                self.server.bind
            )));
        }

        // ── Security validation ─────────────────────────────────
        // Allow-list authority must not exceed trusted authority (the allow-list
        // is a weaker authentication signal than trusted_sender_ids).
        if self.security.allowlist_authority > self.security.trusted_authority {
            return Err(IroncladError::Config(
                "security.allowlist_authority must be ≤ security.trusted_authority \
                 (allow-list is a weaker signal than trusted_sender_ids)"
                    .into(),
            ));
        }

        // Threat scanner ceiling must be below Creator. If the ceiling is Creator,
        // the threat scanner can never actually restrict anything — it's a no-op.
        if self.security.threat_caution_ceiling >= crate::types::InputAuthority::Creator {
            return Err(IroncladError::Config(
                "security.threat_caution_ceiling must be below Creator \
                 (otherwise the threat scanner has no effect)"
                    .into(),
            ));
        }

        // ── Filesystem security validation ─────────────────────────
        for p in &self.security.filesystem.script_allowed_paths {
            if !p.is_absolute() {
                return Err(IroncladError::Config(format!(
                    "security.filesystem.script_allowed_paths: '{}' must be an absolute path",
                    p.display()
                )));
            }
        }

        // ── Routing config validation ──────────────────────────────
        if !matches!(self.models.routing.mode.as_str(), "primary" | "metascore") {
            return Err(IroncladError::Config(format!(
                "models.routing.mode must be one of \"primary\" or \"metascore\", got \"{}\"",
                self.models.routing.mode
            )));
        }
        if !(0.0..=1.0).contains(&self.models.routing.confidence_threshold) {
            return Err(IroncladError::Config(format!(
                "models.routing.confidence_threshold must be in [0.0, 1.0], got {}",
                self.models.routing.confidence_threshold
            )));
        }
        if self.models.routing.estimated_output_tokens == 0 {
            return Err(IroncladError::Config(
                "models.routing.estimated_output_tokens must be >= 1".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.models.routing.accuracy_floor) {
            return Err(IroncladError::Config(format!(
                "models.routing.accuracy_floor must be in [0.0, 1.0], got {}",
                self.models.routing.accuracy_floor
            )));
        }
        if self.models.routing.accuracy_min_obs == 0 {
            return Err(IroncladError::Config(
                "models.routing.accuracy_min_obs must be >= 1".into(),
            ));
        }
        if let Some(cost_weight) = self.models.routing.cost_weight
            && !(0.0..=1.0).contains(&cost_weight)
        {
            return Err(IroncladError::Config(format!(
                "models.routing.cost_weight must be in [0.0, 1.0], got {cost_weight}"
            )));
        }
        if !(0.0..=1.0).contains(&self.models.routing.canary_fraction) {
            return Err(IroncladError::Config(format!(
                "models.routing.canary_fraction must be in [0.0, 1.0], got {}",
                self.models.routing.canary_fraction
            )));
        }

        let canary_model = self
            .models
            .routing
            .canary_model
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        if self.models.routing.canary_fraction > 0.0 && canary_model.is_none() {
            return Err(IroncladError::Config(
                "models.routing.canary_fraction > 0 requires models.routing.canary_model".into(),
            ));
        }
        if canary_model.is_some() && self.models.routing.canary_fraction <= 0.0 {
            return Err(IroncladError::Config(
                "models.routing.canary_model requires models.routing.canary_fraction > 0".into(),
            ));
        }
        if let Some(canary) = canary_model
            && self
                .models
                .routing
                .blocked_models
                .iter()
                .any(|m| m.trim() == canary)
        {
            return Err(IroncladError::Config(
                "models.routing.canary_model must not also appear in models.routing.blocked_models"
                    .into(),
            ));
        }
        for blocked in &self.models.routing.blocked_models {
            if blocked.trim().is_empty() {
                return Err(IroncladError::Config(
                    "models.routing.blocked_models entries must be non-empty".into(),
                ));
            }
        }

        Ok(())
    }
}
