#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConfigMigrationReport {
    pub renamed_server_host_to_bind: bool,
    pub routing_mode_heuristic_rewritten: bool,
    pub deny_on_empty_allowlist_hardened: bool,
    pub removed_credit_cooldown_seconds: bool,
}

impl ConfigMigrationReport {
    pub fn changed(&self) -> bool {
        self.renamed_server_host_to_bind
            || self.routing_mode_heuristic_rewritten
            || self.deny_on_empty_allowlist_hardened
            || self.removed_credit_cooldown_seconds
    }
}

pub fn migrate_removed_legacy_config(raw: &str) -> Result<Option<(String, ConfigMigrationReport)>> {
    let mut doc: toml::Value = toml::from_str(raw)?;
    let mut report = ConfigMigrationReport::default();
    let root = match doc.as_table_mut() {
        Some(root) => root,
        None => return Ok(None),
    };

    if let Some(server) = root.get_mut("server").and_then(|v| v.as_table_mut())
        && let Some(host) = server.remove("host")
    {
        if !server.contains_key("bind") {
            server.insert("bind".to_string(), host);
        }
        report.renamed_server_host_to_bind = true;
    }

    if let Some(models) = root.get_mut("models").and_then(|v| v.as_table_mut())
        && let Some(routing) = models.get_mut("routing").and_then(|v| v.as_table_mut())
        && let Some(mode) = routing.get_mut("mode")
        && let Some(mode_str) = mode.as_str()
        && mode_str == "heuristic"
    {
        *mode = toml::Value::String("metascore".into());
        report.routing_mode_heuristic_rewritten = true;
    }

    if let Some(security) = root.get_mut("security").and_then(|v| v.as_table_mut())
        && let Some(deny) = security.get_mut("deny_on_empty_allowlist")
        && deny.as_bool() == Some(false)
    {
        *deny = toml::Value::Boolean(true);
        report.deny_on_empty_allowlist_hardened = true;
    }

    if let Some(circuit_breaker) = root.get_mut("circuit_breaker").and_then(|v| v.as_table_mut())
        && circuit_breaker.remove("credit_cooldown_seconds").is_some()
    {
        report.removed_credit_cooldown_seconds = true;
    }

    if !report.changed() {
        return Ok(None);
    }

    Ok(Some((toml::to_string_pretty(&doc)?, report)))
}
