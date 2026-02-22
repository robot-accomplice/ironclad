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
    kv_accent("Balance", &format!("{bal} {currency}"));
    kv_mono("Address", addr);
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
    eprintln!();
    kv_accent("Balance", &format!("{bal} {currency}"));
    eprintln!();
    Ok(())
}
