use super::*;

// ── Models ───────────────────────────────────────────────────

pub async fn cmd_models_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let resp = super::http_client()?
        .get(format!("{base_url}/api/config"))
        .send()
        .await?;
    let config: serde_json::Value = resp.json().await?;

    println!("\n  {BOLD}Configured Models{RESET}\n");

    let primary = config
        .pointer("/models/primary")
        .and_then(|v| v.as_str())
        .unwrap_or("not set");
    println!("  {:<12} {}", format!("{GREEN}primary{RESET}"), primary);

    if let Some(fallbacks) = config
        .pointer("/models/fallbacks")
        .and_then(|v| v.as_array())
    {
        for (i, fb) in fallbacks.iter().enumerate() {
            let name = fb.as_str().unwrap_or("?");
            println!(
                "  {:<12} {}",
                format!("{YELLOW}fallback {}{RESET}", i + 1),
                name
            );
        }
    }

    let mode = config
        .pointer("/models/routing/mode")
        .and_then(|v| v.as_str())
        .unwrap_or("rule");
    let threshold = config
        .pointer("/models/routing/confidence_threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.9);
    let local_first = config
        .pointer("/models/routing/local_first")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    println!();
    println!(
        "  {DIM}Routing: mode={mode}, threshold={threshold}, local_first={local_first}{RESET}"
    );
    println!();
    Ok(())
}

pub async fn cmd_models_scan(
    base_url: &str,
    provider: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Scanning for available models...{RESET}\n");

    let resp = super::http_client()?
        .get(format!("{base_url}/api/config"))
        .send()
        .await?;
    let config: serde_json::Value = resp.json().await?;

    let providers = config
        .get("providers")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    if providers.is_empty() {
        println!("  No providers configured.");
        println!();
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    for (name, prov_config) in &providers {
        if let Some(filter) = provider
            && name != filter
        {
            continue;
        }

        let url = prov_config
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if url.is_empty() {
            println!("  {YELLOW}{name}{RESET}: no URL configured");
            continue;
        }

        let name_l = name.to_lowercase();
        let url_l = url.to_lowercase();
        let ollama_like = name_l.contains("ollama") || url_l.contains("11434");
        let models_url = if ollama_like {
            format!("{url}/api/tags")
        } else {
            format!("{url}/v1/models")
        };

        print!("  {CYAN}{name}{RESET} ({url}): ");

        match client.get(&models_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let models: Vec<String> =
                    if let Some(arr) = body.get("models").and_then(|v| v.as_array()) {
                        arr.iter()
                            .filter_map(|m| {
                                m.get("name")
                                    .or_else(|| m.get("model"))
                                    .and_then(|v| v.as_str())
                            })
                            .map(String::from)
                            .collect()
                    } else if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
                        arr.iter()
                            .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                            .map(String::from)
                            .collect()
                    } else {
                        vec![]
                    };

                if models.is_empty() {
                    println!("no models found");
                } else {
                    println!("{} model(s)", models.len());
                    for model in &models {
                        println!("    - {model}");
                    }
                }
            }
            Ok(resp) => {
                println!("{RED}error: {}{RESET}", resp.status());
            }
            Err(e) => {
                println!("{RED}unreachable: {e}{RESET}");
            }
        }
    }

    println!();
    Ok(())
}
