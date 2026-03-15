pub async fn cmd_circuit_status(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/breaker/status").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

    heading("Circuit Breaker Status");

    if let Some(providers) = data["providers"].as_object() {
        if providers.is_empty() {
            empty_state("No providers registered yet");
        } else {
            for (name, status) in providers {
                let state = status["state"].as_str().unwrap_or("unknown");
                kv_accent(name, &status_badge(state));
            }
        }
    } else {
        empty_state("No providers registered yet");
    }

    if let Some(note) = data["note"].as_str() {
        eprintln!();
        eprintln!("    {DIM}\u{2139}  {note}{RESET}");
    }

    eprintln!();
    Ok(())
}

pub async fn cmd_circuit_reset(
    url: &str,
    provider: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let client = super::http_client()?;
    heading("Circuit Breaker Reset");

    let providers: Vec<String> = if let Some(single) = provider {
        vec![single.to_string()]
    } else {
        let status = client
            .get(format!("{url}/api/breaker/status"))
            .send()
            .await
            .inspect_err(|_| {
                eprintln!("  {ERR} Cannot reach gateway at {url}");
            })?;

        if !status.status().is_success() {
            eprintln!("    {WARN} Status returned HTTP {}", status.status());
            eprintln!();
            return Ok(());
        }

        let body: serde_json::Value = status.json().await.unwrap_or_else(|e| {
            tracing::warn!("failed to parse breaker status response: {e}");
            serde_json::Value::default()
        });
        body.get("providers")
            .and_then(|v| v.as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };

    if providers.is_empty() {
        eprintln!("    {WARN} No providers reported by gateway");
        eprintln!();
        return Ok(());
    }

    let mut reset_ok = 0usize;
    for provider in &providers {
        let resp = client
            .post(format!("{url}/api/breaker/reset/{provider}"))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                reset_ok += 1;
            }
            Ok(r) => {
                eprintln!("    {WARN} reset {} returned HTTP {}", provider, r.status());
            }
            Err(e) => {
                eprintln!("    {WARN} reset {} failed: {}", provider, e);
            }
        }
    }

    if reset_ok == providers.len() {
        eprintln!(
            "    {OK} Reset {} providers to closed state",
            providers.len()
        );
    } else {
        eprintln!(
            "    {WARN} Reset {}/{} providers",
            reset_ok,
            providers.len()
        );
    }

    eprintln!();
    Ok(())
}

// ── Agents, channels ──────────────────────────────────────────

pub async fn cmd_agent_start(base_url: &str, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = super::http_client()?;
    let resp = client
        .post(format!("{base_url}/api/agents/{id}/start"))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_else(|e| {
            tracing::warn!("failed to read agent start error body: {e}");
            String::new()
        });
        return Err(format!("HTTP {status}: {body}").into());
    }
    eprintln!("  Agent {id} started");
    Ok(())
}

pub async fn cmd_agent_stop(base_url: &str, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = super::http_client()?;
    let resp = client
        .post(format!("{base_url}/api/agents/{id}/stop"))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_else(|e| {
            tracing::warn!("failed to read agent stop error body: {e}");
            String::new()
        });
        return Err(format!("HTTP {status}: {body}").into());
    }
    eprintln!("  Agent {id} stopped");
    Ok(())
}

pub async fn cmd_agents_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = super::http_client()?;
    let resp = client.get(format!("{base_url}/api/agents")).send().await?;
    let body: serde_json::Value = resp.json().await?;

    let agents = body
        .get("agents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if agents.is_empty() {
        println!("\n  No agents registered.\n");
        return Ok(());
    }

    println!(
        "\n  {:<15} {:<20} {:<10} {:<15}",
        "ID", "Name", "State", "Model"
    );
    println!("  {}", "─".repeat(65));
    for a in &agents {
        let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let state = a.get("state").and_then(|v| v.as_str()).unwrap_or("?");
        let model = a.get("model").and_then(|v| v.as_str()).unwrap_or("?");
        println!("  {:<15} {:<20} {:<10} {:<15}", id, name, state, model);
    }
    println!();
    Ok(())
}

pub async fn cmd_channels_status(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = super::http_client()?
        .get(format!("{base_url}/api/channels/status"))
        .send()
        .await?;
    let channels: Vec<serde_json::Value> = resp.json().await?;

    if channels.is_empty() {
        println!("  No channels configured.");
        return Ok(());
    }

    println!(
        "\n  {:<15} {:<10} {:<10} {:<10}",
        "Channel", "Status", "Recv", "Sent"
    );
    println!("  {}", "─".repeat(50));
    for ch in &channels {
        let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let connected = ch
            .get("connected")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let status_str = if connected { "✓ up" } else { "✗ down" };
        let recv = ch
            .get("messages_received")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let sent = ch
            .get("messages_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        println!(
            "  {:<15} {:<10} {:<10} {:<10}",
            name, status_str, recv, sent
        );
    }
    println!();
    Ok(())
}

pub async fn cmd_channels_dead_letter(
    base_url: &str,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let resp = super::http_client()?
        .get(format!("{base_url}/api/channels/dead-letter?limit={limit}"))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if items.is_empty() {
        println!("  No dead-letter deliveries.");
        return Ok(());
    }

    println!(
        "\n  {:<38} {:<12} {:<10} {:<40}",
        "ID", "Channel", "Attempts", "Last error"
    );
    println!("  {}", "─".repeat(108));
    for item in items {
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("?");
        let attempts = item
            .get("attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or_default();
        let max_attempts = item
            .get("max_attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or_default();
        let last_error = item
            .get("last_error")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        println!(
            "  {:<38} {:<12} {:<10} {:<40}",
            truncate_id(id, 35),
            channel,
            format!("{attempts}/{max_attempts}"),
            truncate_id(last_error, 37),
        );
    }
    println!();
    Ok(())
}

pub async fn cmd_channels_replay(
    base_url: &str,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = super::http_client()?;
    let resp = client
        .post(format!("{base_url}/api/channels/dead-letter/{id}/replay"))
        .send()
        .await?;
    if resp.status().is_success() {
        println!("  Replayed dead-letter item: {id}");
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
        println!("  Dead-letter item not found: {id}");
    } else {
        println!("  Replay failed for {id}: HTTP {}", resp.status());
    }
    Ok(())
}

// ── Mechanic ──────────────────────────────────────────────────

