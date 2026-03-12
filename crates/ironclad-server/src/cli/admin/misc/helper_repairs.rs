#[derive(Debug, Clone, Serialize)]
struct MechanicRepairPlan {
    description: String,
    commands: Vec<String>,
    safe_auto_repair: bool,
    requires_human_approval: bool,
}

#[derive(Debug, Clone, Serialize)]
struct MechanicFinding {
    id: String,
    severity: String,
    confidence: f64,
    summary: String,
    details: String,
    repair_plan: MechanicRepairPlan,
    auto_repaired: bool,
}

#[derive(Debug, Default, Clone, Serialize)]
struct RepairActionSummary {
    directories_created: Vec<String>,
    config_created: bool,
    permissions_hardened: Vec<String>,
    schema_normalized: bool,
    internalized_skills_cleaned: Vec<String>,
    paused_jobs_reenabled: Vec<String>,
    security_configured: bool,
}

#[derive(Debug, Serialize)]
struct MechanicJsonReport {
    ok: bool,
    repair_mode: bool,
    findings: Vec<MechanicFinding>,
    actions: RepairActionSummary,
}

#[allow(clippy::too_many_arguments)]
fn finding(
    id: &str,
    severity: &str,
    confidence: f64,
    summary: impl Into<String>,
    details: impl Into<String>,
    plan_desc: impl Into<String>,
    commands: Vec<String>,
    safe_auto_repair: bool,
    requires_human_approval: bool,
) -> MechanicFinding {
    MechanicFinding {
        id: id.to_string(),
        severity: severity.to_string(),
        confidence,
        summary: summary.into(),
        details: details.into(),
        repair_plan: MechanicRepairPlan {
            description: plan_desc.into(),
            commands,
            safe_auto_repair,
            requires_human_approval,
        },
        auto_repaired: false,
    }
}

fn normalize_schema_safe(state_db_path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(crate::state_hygiene::run_state_hygiene(state_db_path)?.changed)
}

