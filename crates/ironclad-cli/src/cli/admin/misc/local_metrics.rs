pub async fn cmd_metrics(
    url: &str,
    kind: &str,
    hours: Option<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;

    match kind {
        "costs" => {
            let data = c.get("/api/stats/costs").await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading("Inference Costs");
            let costs = data["costs"].as_array();
            match costs {
                Some(arr) if !arr.is_empty() => {
                    let mut suppressed_zero_rows = 0usize;
                    let filtered: Vec<&serde_json::Value> = arr
                        .iter()
                        .filter(|c| {
                            let tin = c["tokens_in"].as_i64().unwrap_or(0);
                            let tout = c["tokens_out"].as_i64().unwrap_or(0);
                            let cost = c["cost"].as_f64().unwrap_or(0.0);
                            let cached = c["cached"].as_bool().unwrap_or(false);
                            let keep = cached || tin != 0 || tout != 0 || cost > 0.0;
                            if !keep {
                                suppressed_zero_rows += 1;
                            }
                            keep
                        })
                        .collect();
                    if filtered.is_empty() {
                        empty_state(
                            "No billable/non-empty inference costs recorded (all recent rows were zero-token/no-cost events)",
                        );
                        if suppressed_zero_rows > 0 {
                            kv("Suppressed Zero Rows", &suppressed_zero_rows.to_string());
                        }
                        return Ok(());
                    }

                    let widths = [20, 16, 10, 10, 10, 8];
                    table_header(
                        &[
                            "Model",
                            "Provider",
                            "Tokens In",
                            "Tokens Out",
                            "Cost",
                            "Cached",
                        ],
                        &widths,
                    );

                    let mut total_cost = 0.0f64;
                    let mut total_in = 0i64;
                    let mut total_out = 0i64;

                    for c in &filtered {
                        let model = truncate_id(c["model"].as_str().unwrap_or(""), 17);
                        let provider = c["provider"].as_str().unwrap_or("").to_string();
                        let tin = c["tokens_in"].as_i64().unwrap_or(0);
                        let tout = c["tokens_out"].as_i64().unwrap_or(0);
                        let cost = c["cost"].as_f64().unwrap_or(0.0);
                        let cached = c["cached"].as_bool().unwrap_or(false);

                        total_cost += cost;
                        total_in += tin;
                        total_out += tout;

                        table_row(
                            &[
                                format!("{ACCENT}{model}{RESET}"),
                                provider,
                                tin.to_string(),
                                tout.to_string(),
                                format!("${cost:.4}"),
                                if cached {
                                    OK.to_string()
                                } else {
                                    format!("{DIM}-{RESET}")
                                },
                            ],
                            &widths,
                        );
                    }
                    table_separator(&widths);
                    eprintln!();
                    kv_accent("Total Cost", &format!("${total_cost:.4}"));
                    kv("Total Tokens", &format!("{total_in} in / {total_out} out"));
                    kv("Requests", &filtered.len().to_string());
                    if suppressed_zero_rows > 0 {
                        kv("Suppressed Zero Rows", &suppressed_zero_rows.to_string());
                    }
                    if !filtered.is_empty() {
                        kv(
                            "Avg Cost/Request",
                            &format!("${:.4}", total_cost / filtered.len() as f64),
                        );
                    }
                }
                _ => empty_state("No inference costs recorded"),
            }
        }
        "transactions" => {
            let h = hours.unwrap_or(24);
            let data = c
                .get(&format!("/api/stats/transactions?hours={h}"))
                .await
                .map_err(|e| {
                    IroncladClient::check_connectivity_hint(&*e);
                    e
                })?;
            heading(&format!("Transactions (last {h}h)"));
            let txs = data["transactions"].as_array();
            match txs {
                Some(arr) if !arr.is_empty() => {
                    let widths = [14, 12, 12, 20, 22];
                    table_header(&["ID", "Type", "Amount", "Counterparty", "Time"], &widths);

                    let mut total = 0.0f64;
                    for t in arr {
                        let id = truncate_id(t["id"].as_str().unwrap_or(""), 11);
                        let tx_type = t["tx_type"].as_str().unwrap_or("").to_string();
                        let amount = t["amount"].as_f64().unwrap_or(0.0);
                        let currency = t["currency"].as_str().unwrap_or("USD");
                        let counter = t["counterparty"].as_str().unwrap_or("-").to_string();
                        let time = t["created_at"]
                            .as_str()
                            .map(|t| if t.len() > 19 { &t[..19] } else { t })
                            .unwrap_or("")
                            .to_string();

                        total += amount;

                        table_row(
                            &[
                                format!("{MONO}{id}{RESET}"),
                                tx_type,
                                format!("{amount:.2} {currency}"),
                                counter,
                                format!("{DIM}{time}{RESET}"),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    kv_accent("Total", &format!("{total:.2}"));
                    kv("Count", &arr.len().to_string());
                }
                _ => empty_state("No transactions in this time window"),
            }
        }
        "cache" => {
            let data = c.get("/api/stats/cache").await.map_err(|e| {
                IroncladClient::check_connectivity_hint(&*e);
                e
            })?;
            heading("Cache Statistics");
            let hits = data["hits"].as_u64().unwrap_or(0);
            let misses = data["misses"].as_u64().unwrap_or(0);
            let entries = data["entries"].as_u64().unwrap_or(0);
            let hit_rate = data["hit_rate"].as_f64().unwrap_or(0.0);

            kv_accent("Entries", &entries.to_string());
            kv("Hits", &hits.to_string());
            kv("Misses", &misses.to_string());

            let bar_width = 30;
            let filled = (hit_rate * bar_width as f64 / 100.0) as usize;
            let empty_part = bar_width - filled;
            let bar = format!(
                "{GREEN}{}{DIM}{}{RESET} {:.1}%",
                "\u{2588}".repeat(filled),
                "\u{2591}".repeat(empty_part),
                hit_rate
            );
            kv("Hit Rate", &bar);
        }
        _ => {
            return Err(
                format!("unknown metric kind: {kind}. Use: costs, transactions, cache").into(),
            );
        }
    }

    eprintln!();
    Ok(())
}

