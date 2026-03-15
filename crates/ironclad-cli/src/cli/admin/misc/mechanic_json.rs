include!("mechanic_json_flow.rs");

async fn cmd_mechanic_json(
    base_url: &str,
    repair: bool,
    allow_jobs: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");
    let mut findings: Vec<MechanicFinding> = vec![];
    let mut actions = RepairActionSummary::default();

    collect_mechanic_json_local_findings(&ironclad_dir, repair, &mut findings, &mut actions)?;
    collect_mechanic_json_gateway_findings(
        base_url,
        &ironclad_dir,
        repair,
        allow_jobs,
        &mut findings,
        &mut actions,
    )
    .await?;
    collect_mechanic_json_security_and_plugin_findings(
        &ironclad_dir,
        repair,
        &mut findings,
        &mut actions,
    )?;

    let report = MechanicJsonReport {
        ok: findings
            .iter()
            .all(|f| f.severity == "info" || f.auto_repaired),
        repair_mode: repair,
        findings,
        actions,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
