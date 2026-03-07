use super::*;

pub async fn cmd_wallet(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let balance = c.get("/api/wallet/balance").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let address = c.get("/api/wallet/address").await?;
    heading("Wallet");
    let bal = balance["balance"].as_str().unwrap_or("0.00");
    let currency = balance["currency"].as_str().unwrap_or("USDC");
    let addr = address["address"].as_str().unwrap_or("not connected");
    let treasury = &balance["treasury"];
    let swap = &treasury["revenue_swap"];
    let tax = &balance["self_funding"]["tax"];
    let accounting = &balance["revenue_accounting"];
    let swap_queue = &balance["revenue_swap_queue"];
    let tax_queue = &balance["revenue_tax_queue"];
    let strategy_summary = balance["revenue_strategy_summary"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let feedback_summary = balance["revenue_feedback_summary"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let seed_readiness = &balance["seed_exercise_readiness"];
    let seed_progress = &balance["seed_exercise_progress"];
    let seed_plan = balance["seed_exercise_plan"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    kv_accent("Balance", &format!("{bal} {currency}"));
    kv_mono("Address", addr);
    if swap.is_object() {
        let swap_status = if swap["enabled"].as_bool().unwrap_or(false) {
            "enabled"
        } else {
            "disabled"
        };
        let target = swap["target_symbol"].as_str().unwrap_or("PALM_USD");
        let chain = swap["default_chain"].as_str().unwrap_or("ETH");
        kv(
            "Revenue Swap",
            &format!("{swap_status} -> {target} on {chain}"),
        );
        if let Some(chains) = swap["chains"].as_array() {
            let configured: Vec<String> = chains
                .iter()
                .filter_map(|entry| entry["chain"].as_str())
                .map(str::to_string)
                .collect();
            if !configured.is_empty() {
                kv("Swap Chains", &configured.join(", "));
            }
        }
    }
    if tax.is_object() {
        let tax_status = if tax["enabled"].as_bool().unwrap_or(false) {
            "enabled"
        } else {
            "disabled"
        };
        let tax_rate = tax["rate"].as_f64().unwrap_or(0.0) * 100.0;
        kv("Profit Tax", &format!("{tax_status} @ {tax_rate:.2}%"));
        if let Some(dest) = tax["destination_wallet"].as_str().filter(|s| !s.is_empty()) {
            kv("Tax Wallet", dest);
        }
    }
    if accounting.is_object() {
        kv(
            "Revenue Gross",
            &format!(
                "{:.2} USDC",
                accounting["gross_revenue_usdc"].as_f64().unwrap_or(0.0)
            ),
        );
        kv(
            "Revenue Net",
            &format!(
                "{:.2} USDC",
                accounting["net_profit_usdc"].as_f64().unwrap_or(0.0)
            ),
        );
        kv(
            "Retained",
            &format!(
                "{:.2} USDC",
                accounting["retained_earnings_usdc"].as_f64().unwrap_or(0.0)
            ),
        );
    }
    if swap_queue.is_object() {
        kv(
            "Swap Queue",
            &format!(
                "total={} pending={} in_progress={} failed={} stale={}",
                swap_queue["total"].as_i64().unwrap_or(0),
                swap_queue["pending"].as_i64().unwrap_or(0),
                swap_queue["in_progress"].as_i64().unwrap_or(0),
                swap_queue["failed"].as_i64().unwrap_or(0),
                swap_queue["stale_in_progress"].as_i64().unwrap_or(0),
            ),
        );
    }
    if tax_queue.is_object() {
        kv(
            "Tax Queue",
            &format!(
                "total={} pending={} in_progress={} failed={} completed={}",
                tax_queue["total"].as_i64().unwrap_or(0),
                tax_queue["pending"].as_i64().unwrap_or(0),
                tax_queue["in_progress"].as_i64().unwrap_or(0),
                tax_queue["failed"].as_i64().unwrap_or(0),
                tax_queue["completed"].as_i64().unwrap_or(0),
            ),
        );
    }
    if !strategy_summary.is_empty() {
        let top = &strategy_summary[0];
        kv(
            "Top Revenue Strategy",
            &format!(
                "{} (net {:.2} USDC)",
                top["strategy"].as_str().unwrap_or("unknown"),
                top["net_profit_usdc"].as_f64().unwrap_or(0.0)
            ),
        );
    }
    if !feedback_summary.is_empty() {
        let top = &feedback_summary[0];
        kv(
            "Top Feedback Strategy",
            &format!(
                "{} ({:.2}/5 over {} signals)",
                top["strategy"].as_str().unwrap_or("unknown"),
                top["avg_grade"].as_f64().unwrap_or(0.0),
                top["feedback_count"].as_i64().unwrap_or(0)
            ),
        );
    }
    if seed_readiness.is_object() {
        kv(
            "$50 Seed Readiness",
            if seed_readiness["meets_seed_target"]
                .as_bool()
                .unwrap_or(false)
            {
                "ready"
            } else {
                "not ready"
            },
        );
        kv(
            "Stable Balance",
            &format!(
                "{:.2} / {:.2} USDC target",
                seed_readiness["stable_balance_usdc"]
                    .as_f64()
                    .unwrap_or(0.0),
                seed_readiness["seed_target_usdc"].as_f64().unwrap_or(50.0)
            ),
        );
    }
    if seed_progress.is_object() {
        kv(
            "Seed Next Action",
            seed_progress["next_action"]
                .as_str()
                .unwrap_or("no action available"),
        );
    }
    if let Some(phases) = seed_plan.get("phases").and_then(|v| v.as_array()) {
        kv("Seed Exercise Phases", &phases.len().to_string());
        if let Some(first) = phases.first() {
            kv(
                "Seed First Phase",
                first["label"].as_str().unwrap_or("no phase available"),
            );
        }
    }
    if let Some(note) = balance["note"].as_str() {
        eprintln!();
        eprintln!("    {DIM}\u{2139}  {note}{RESET}");
    }
    eprintln!();
    Ok(())
}

pub async fn cmd_wallet_address(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let address = c.get("/api/wallet/address").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let addr = address["address"].as_str().unwrap_or("not connected");
    eprintln!();
    eprintln!("    {MONO}{addr}{RESET}");
    eprintln!();
    Ok(())
}

pub async fn cmd_wallet_balance(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let balance = c.get("/api/wallet/balance").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    let bal = balance["balance"].as_str().unwrap_or("0.00");
    let currency = balance["currency"].as_str().unwrap_or("USDC");
    let swap = &balance["treasury"]["revenue_swap"];
    let tax = &balance["self_funding"]["tax"];
    let accounting = &balance["revenue_accounting"];
    let swap_queue = &balance["revenue_swap_queue"];
    let tax_queue = &balance["revenue_tax_queue"];
    let strategy_summary = balance["revenue_strategy_summary"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let feedback_summary = balance["revenue_feedback_summary"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let seed_readiness = &balance["seed_exercise_readiness"];
    let seed_progress = &balance["seed_exercise_progress"];
    let seed_plan = balance["seed_exercise_plan"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    eprintln!();
    kv_accent("Balance", &format!("{bal} {currency}"));
    if swap.is_object() {
        let swap_status = if swap["enabled"].as_bool().unwrap_or(false) {
            "enabled"
        } else {
            "disabled"
        };
        let target = swap["target_symbol"].as_str().unwrap_or("PALM_USD");
        let chain = swap["default_chain"].as_str().unwrap_or("ETH");
        kv(
            "Revenue Swap",
            &format!("{swap_status} -> {target} on {chain}"),
        );
    }
    if tax.is_object() {
        let tax_status = if tax["enabled"].as_bool().unwrap_or(false) {
            "enabled"
        } else {
            "disabled"
        };
        let tax_rate = tax["rate"].as_f64().unwrap_or(0.0) * 100.0;
        kv("Profit Tax", &format!("{tax_status} @ {tax_rate:.2}%"));
    }
    if accounting.is_object() {
        kv(
            "Revenue Net",
            &format!(
                "{:.2} USDC",
                accounting["net_profit_usdc"].as_f64().unwrap_or(0.0)
            ),
        );
    }
    if swap_queue.is_object() {
        kv(
            "Swap Queue",
            &format!(
                "pending={} in_progress={} failed={}",
                swap_queue["pending"].as_i64().unwrap_or(0),
                swap_queue["in_progress"].as_i64().unwrap_or(0),
                swap_queue["failed"].as_i64().unwrap_or(0),
            ),
        );
    }
    if tax_queue.is_object() {
        kv(
            "Tax Queue",
            &format!(
                "pending={} in_progress={} failed={} completed={}",
                tax_queue["pending"].as_i64().unwrap_or(0),
                tax_queue["in_progress"].as_i64().unwrap_or(0),
                tax_queue["failed"].as_i64().unwrap_or(0),
                tax_queue["completed"].as_i64().unwrap_or(0),
            ),
        );
    }
    if !strategy_summary.is_empty() {
        let top = &strategy_summary[0];
        kv(
            "Top Strategy",
            &format!(
                "{} ({:.2} USDC net)",
                top["strategy"].as_str().unwrap_or("unknown"),
                top["net_profit_usdc"].as_f64().unwrap_or(0.0)
            ),
        );
    }
    if !feedback_summary.is_empty() {
        let top = &feedback_summary[0];
        kv(
            "Top Feedback",
            &format!(
                "{} ({:.2}/5 over {} signals)",
                top["strategy"].as_str().unwrap_or("unknown"),
                top["avg_grade"].as_f64().unwrap_or(0.0),
                top["feedback_count"].as_i64().unwrap_or(0)
            ),
        );
    }
    if seed_readiness.is_object() {
        kv(
            "Seed Readiness",
            if seed_readiness["meets_seed_target"]
                .as_bool()
                .unwrap_or(false)
            {
                "ready"
            } else {
                "not ready"
            },
        );
    }
    if seed_progress.is_object() {
        kv(
            "Seed Next Action",
            seed_progress["next_action"]
                .as_str()
                .unwrap_or("no action available"),
        );
    }
    if let Some(phases) = seed_plan.get("phases").and_then(|v| v.as_array()) {
        kv("Seed Exercise Phases", &phases.len().to_string());
    }
    eprintln!();
    Ok(())
}
