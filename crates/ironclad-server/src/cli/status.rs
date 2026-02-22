use super::*;

pub(crate) fn is_connection_error(msg: &str) -> bool {
    msg.contains("Connection refused")
        || msg.contains("ConnectionRefused")
        || msg.contains("ConnectError")
        || msg.contains("connect error")
        || msg.contains("kind: Decode")
        || msg.contains("hyper::Error")
}

pub async fn cmd_status(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url)?;
    heading("Agent Status");
    let health = match c.get("/api/health").await {
        Ok(h) => h,
        Err(e) => {
            let msg = format!("{:?}", e);
            if is_connection_error(&msg) {
                let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
                let (OK, ACTION, WARN, DETAIL, ERR) = icons();
                eprintln!();
                eprintln!("  {WARN} Server is not running at {BOLD}{url}{RESET}");
                eprintln!("  {DIM}Start with: {BOLD}ironclad serve{RESET}");
                eprintln!();
                return Ok(());
            }
            eprintln!();
            eprintln!("  Could not connect to agent at {url}: {e}");
            eprintln!();
            IroncladClient::check_connectivity_hint(&*e);
            return Err(e);
        }
    };
    let agent = c.get("/api/agent/status").await?;
    let config = c.get("/api/config").await?;
    let sessions = c.get("/api/sessions").await?;
    let skills = c.get("/api/skills").await?;
    let jobs = c.get("/api/cron/jobs").await?;
    let cache = c.get("/api/stats/cache").await?;
    let wallet = c.get("/api/wallet/balance").await?;
    let agent_name = config["agent"]["name"].as_str().unwrap_or("unknown");
    let agent_id = config["agent"]["id"].as_str().unwrap_or("unknown");
    let agent_state = agent["state"].as_str().unwrap_or("unknown");
    let version = health["version"].as_str().unwrap_or("?");
    let session_count = sessions["sessions"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let skill_count = skills["skills"].as_array().map(|a| a.len()).unwrap_or(0);
    let job_count = jobs["jobs"].as_array().map(|a| a.len()).unwrap_or(0);
    let hits = cache["hits"].as_u64().unwrap_or(0);
    let misses = cache["misses"].as_u64().unwrap_or(0);
    let hit_rate = if hits + misses > 0 {
        format!("{:.1}%", hits as f64 / (hits + misses) as f64 * 100.0)
    } else {
        "n/a".into()
    };
    let balance = wallet["balance"].as_str().unwrap_or("0.00");
    let currency = wallet["currency"].as_str().unwrap_or("USDC");
    kv_accent("Agent", &format!("{agent_name} ({agent_id})"));
    kv("State", &status_badge(agent_state).to_string());
    kv_accent("Version", version);
    kv("Sessions", &session_count.to_string());
    kv("Skills", &skill_count.to_string());
    kv("Cron Jobs", &job_count.to_string());
    kv("Cache Hit Rate", &hit_rate);
    kv_accent("Balance", &format!("{balance} {currency}"));
    let primary = config["models"]["primary"].as_str().unwrap_or("unknown");
    kv("Primary Model", primary);
    eprintln!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_connection_refused() {
        assert!(is_connection_error("Connection refused (os error 61)"));
    }

    #[test]
    fn detects_connect_error_variant() {
        assert!(is_connection_error("hyper::Error(Connect, ConnectError)"));
    }

    #[test]
    fn detects_decode_error() {
        assert!(is_connection_error("kind: Decode"));
    }

    #[test]
    fn ignores_unrelated_errors() {
        assert!(!is_connection_error("404 Not Found"));
        assert!(!is_connection_error("timeout after 30s"));
    }
}
