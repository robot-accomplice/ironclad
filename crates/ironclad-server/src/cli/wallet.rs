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
    eprintln!();
    Ok(())
}
