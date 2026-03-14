use super::*;

pub async fn cmd_memory(
    url: &str,
    tier: &str,
    session_id: Option<&str>,
    query: Option<&str>,
    limit: Option<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    match tier {
        "working" => {
            let sid = session_id.ok_or("--session required for working memory. Use 'ironclad sessions list' to find session IDs.")?;
            let data = c
                .get(&format!("/api/memory/working/{sid}"))
                .await
                .map_err(|e| {
                    IroncladClient::check_connectivity_hint(&*e);
                    e
                })?;
            heading("Working Memory");
            let entries = data["entries"].as_array();
            match entries {
                Some(arr) if !arr.is_empty() => {
                    let widths = [12, 14, 36, 10];
                    table_header(&["ID", "Type", "Content", "Importance"], &widths);
                    for e in arr {
                        table_row(
                            &[
                                format!(
                                    "{MONO}{}{RESET}",
                                    truncate_id(e["id"].as_str().unwrap_or(""), 9)
                                ),
                                e["entry_type"].as_str().unwrap_or("").to_string(),
                                truncate_id(e["content"].as_str().unwrap_or(""), 33),
                                e["importance"].to_string(),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} entries{RESET}", arr.len());
                }
                _ => empty_state("No working memory entries"),
            }
        }
        "episodic" => {
            let lim = limit.unwrap_or(20);
            let data = c
                .get(&format!("/api/memory/episodic?limit={lim}"))
                .await
                .map_err(|e| {
                    IroncladClient::check_connectivity_hint(&*e);
                    e
                })?;
            heading("Episodic Memory");
            let entries = data["entries"].as_array();
            match entries {
                Some(arr) if !arr.is_empty() => {
                    let widths = [12, 16, 36, 10];
                    table_header(&["ID", "Classification", "Content", "Importance"], &widths);
                    for e in arr {
                        table_row(
                            &[
                                format!(
                                    "{MONO}{}{RESET}",
                                    truncate_id(e["id"].as_str().unwrap_or(""), 9)
                                ),
                                e["classification"].as_str().unwrap_or("").to_string(),
                                truncate_id(e["content"].as_str().unwrap_or(""), 33),
                                e["importance"].to_string(),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} entries (limit: {lim}){RESET}", arr.len());
                }
                _ => empty_state("No episodic memory entries"),
            }
        }
        "semantic" => {
            let category = session_id.unwrap_or("general");
            let data = c
                .get(&format!("/api/memory/semantic/{category}"))
                .await
                .map_err(|e| {
                    IroncladClient::check_connectivity_hint(&*e);
                    e
                })?;
            heading(&format!("Semantic Memory [{category}]"));
            let entries = data["entries"].as_array();
            match entries {
                Some(arr) if !arr.is_empty() => {
                    let widths = [20, 34, 12];
                    table_header(&["Key", "Value", "Confidence"], &widths);
                    for e in arr {
                        table_row(
                            &[
                                format!("{ACCENT}{}{RESET}", e["key"].as_str().unwrap_or("")),
                                truncate_id(e["value"].as_str().unwrap_or(""), 31),
                                format!("{:.2}", e["confidence"].as_f64().unwrap_or(0.0)),
                            ],
                            &widths,
                        );
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} entries{RESET}", arr.len());
                }
                _ => empty_state("No semantic memory entries in this category"),
            }
        }
        "search" => {
            let q = query.ok_or("--query/-q required for memory search")?;
            let data = c
                .get(&format!("/api/memory/search?q={}", urlencoding(q)))
                .await
                .map_err(|e| {
                    IroncladClient::check_connectivity_hint(&*e);
                    e
                })?;
            heading(&format!("Memory Search: \"{q}\""));
            let results = data["results"].as_array();
            match results {
                Some(arr) if !arr.is_empty() => {
                    for (i, r) in arr.iter().enumerate() {
                        let fallback = r.to_string();
                        let text = r.as_str().unwrap_or(&fallback);
                        eprintln!("    {DIM}{:>3}.{RESET} {text}", i + 1);
                    }
                    eprintln!();
                    eprintln!("    {DIM}{} results{RESET}", arr.len());
                }
                _ => empty_state("No results found"),
            }
        }
        _ => {
            return Err(format!(
                "unknown memory tier: {tier}. Use: working, episodic, semantic, search"
            )
            .into());
        }
    }
    eprintln!();
    Ok(())
}
