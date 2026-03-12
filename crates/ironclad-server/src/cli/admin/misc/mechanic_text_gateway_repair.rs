async fn run_gateway_integrated_repair_sweep(
    base_url: &str,
    ironclad_dir: &Path,
    gateway_up: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (OK, _, WARN, DETAIL, _) = icons();

    match super::http_client() {
        Ok(client) => match crate::cli::update::check_binary_version(&client).await {
            Ok(Some(latest)) if crate::cli::update::is_newer(&latest, env!("CARGO_PKG_VERSION")) => {
                println!(
                    "  {WARN} Update available: v{latest} (current v{})",
                    env!("CARGO_PKG_VERSION")
                );
            }
            Ok(Some(latest)) => println!("  {OK} Binary version current (latest v{latest})"),
            Ok(None) => println!("  {WARN} Update check unavailable (could not query release source)"),
            Err(e) => println!("  {WARN} Update check failed: {e}"),
        },
        Err(e) => println!("  {WARN} Update check setup failed: {e}"),
    }

    let workspace = ironclad_dir.join("workspace");
    if workspace.exists() {
        let passes = [
            crate::cli::defrag::pass_refs(&workspace),
            crate::cli::defrag::pass_drift(&workspace),
            crate::cli::defrag::pass_artifacts(&workspace),
            crate::cli::defrag::pass_stale(&workspace),
            crate::cli::defrag::pass_identity(&workspace),
            crate::cli::defrag::pass_scripts(&workspace),
        ];
        let total_findings: usize = passes.iter().map(std::vec::Vec::len).sum();
        let fixable_findings: usize = passes.iter().flatten().filter(|f| f.fixable).count();
        if total_findings == 0 {
            println!("  {OK} Defrag sweep clean (0 findings)");
        } else {
            println!("  {WARN} Defrag sweep found {total_findings} finding(s), {fixable_findings} fixable");
            println!("    {DETAIL} Run `ironclad defrag --fix --yes` to apply fixable defrag repairs.");
        }
    } else {
        println!(
            "  {WARN} Defrag sweep skipped: workspace directory missing ({})",
            workspace.display()
        );
    }

    if gateway_up {
        match super::http_client() {
            Ok(client) => match client
                .get(format!("{base_url}/api/breaker/status"))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();
                    let providers = body
                        .get("providers")
                        .and_then(|v| v.as_object())
                        .cloned()
                        .unwrap_or_default();
                    if providers.is_empty() {
                        println!("  {OK} Circuit status: no providers registered");
                    } else {
                        let mut open_or_half = 0usize;
                        for status in providers.values() {
                            let state = status.get("state").and_then(|v| v.as_str()).unwrap_or("unknown");
                            if state.eq_ignore_ascii_case("open")
                                || state.eq_ignore_ascii_case("half_open")
                                || state.eq_ignore_ascii_case("half-open")
                            {
                                open_or_half += 1;
                            }
                        }
                        if open_or_half == 0 {
                            println!(
                                "  {OK} Circuit status healthy ({} provider{} closed)",
                                providers.len(),
                                if providers.len() == 1 { "" } else { "s" }
                            );
                        } else {
                            println!(
                                "  {WARN} Circuit status degraded ({open_or_half}/{} provider{} open or half-open)",
                                providers.len(),
                                if providers.len() == 1 { "" } else { "s" }
                            );
                            println!("    {DETAIL} Run `ironclad circuit status` for per-provider state.");
                        }
                    }
                }
                Ok(resp) => println!("  {WARN} Circuit status check failed (HTTP {})", resp.status()),
                Err(e) => println!("  {WARN} Circuit status check failed: {e}"),
            },
            Err(e) => println!("  {WARN} Circuit status check setup failed: {e}"),
        }
    } else {
        println!("  {WARN} Circuit status check skipped: gateway unavailable");
    }
    Ok(())
}
