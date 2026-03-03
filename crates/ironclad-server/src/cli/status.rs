use super::*;

pub(crate) fn is_connection_error(msg: &str) -> bool {
    msg.contains("Connection refused")
        || msg.contains("ConnectionRefused")
        || msg.contains("ConnectError")
        || msg.contains("connect error")
        || msg.contains("kind: Decode")
        || msg.contains("hyper::Error")
}

pub async fn cmd_status(url: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let c = IroncladClient::new(url)?;
    let health = match c.get("/api/health").await {
        Ok(h) => h,
        Err(e) => {
            let msg = format!("{:?}", e);
            if is_connection_error(&msg) {
                if json {
                    println!("{}", serde_json::json!({"status": "offline", "url": url}));
                } else {
                    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
                    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
                    eprintln!();
                    eprintln!("  {WARN} Server is not running at {BOLD}{url}{RESET}");
                    eprintln!("  {DIM}Start with: {BOLD}ironclad serve{RESET}");
                    eprintln!();
                }
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

    if json {
        let out = serde_json::json!({
            "status": "online",
            "version": health["version"],
            "agent": {
                "name": config["agent"]["name"],
                "id": config["agent"]["id"],
                "state": agent["state"],
            },
            "sessions": sessions["sessions"].as_array().map(|a| a.len()).unwrap_or(0),
            "skills": skills["skills"].as_array().map(|a| a.len()).unwrap_or(0),
            "cron_jobs": jobs["jobs"].as_array().map(|a| a.len()).unwrap_or(0),
            "cache": cache,
            "wallet": wallet,
            "primary_model": config["models"]["primary"],
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    heading("Agent Status");
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
        "0%".into()
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
    use axum::{Json, Router, routing::get};
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

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

    #[test]
    fn detects_connect_error_lowercase() {
        assert!(is_connection_error("connect error: tcp handshake failed"));
    }

    #[test]
    fn detects_hyper_error() {
        assert!(is_connection_error("hyper::Error somewhere in the chain"));
    }

    #[test]
    fn empty_string_is_not_connection_error() {
        assert!(!is_connection_error(""));
    }

    #[test]
    fn detects_connection_refused_variant() {
        assert!(is_connection_error("ConnectionRefused: host unreachable"));
    }

    #[tokio::test]
    async fn cmd_status_succeeds_against_local_mock_server() {
        async fn health() -> Json<serde_json::Value> {
            Json(serde_json::json!({"version":"0.8.0"}))
        }
        async fn agent_status() -> Json<serde_json::Value> {
            Json(serde_json::json!({"state":"running"}))
        }
        async fn config() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "agent": {"name":"TestBot","id":"test-bot"},
                "models": {"primary":"ollama/qwen3:8b"}
            }))
        }
        async fn sessions() -> Json<serde_json::Value> {
            Json(serde_json::json!({"sessions":[{"id":"s1"}]}))
        }
        async fn skills() -> Json<serde_json::Value> {
            Json(serde_json::json!({"skills":[{"id":"k1"},{"id":"k2"}]}))
        }
        async fn cron_jobs() -> Json<serde_json::Value> {
            Json(serde_json::json!({"jobs":[{"id":"j1"}]}))
        }
        async fn cache() -> Json<serde_json::Value> {
            Json(serde_json::json!({"hits":3,"misses":1}))
        }
        async fn wallet() -> Json<serde_json::Value> {
            Json(serde_json::json!({"balance":"12.34","currency":"USDC"}))
        }

        let app = Router::new()
            .route("/api/health", get(health))
            .route("/api/agent/status", get(agent_status))
            .route("/api/config", get(config))
            .route("/api/sessions", get(sessions))
            .route("/api/skills", get(skills))
            .route("/api/cron/jobs", get(cron_jobs))
            .route("/api/stats/cache", get(cache))
            .route("/api/wallet/balance", get(wallet));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}:{}", addr.ip(), addr.port());
        let result = cmd_status(&url, false).await;
        server.abort();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cmd_status_returns_ok_for_unreachable_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = format!("http://127.0.0.1:{port}");
        let result = cmd_status(&url, false).await;
        assert!(result.is_ok());
    }
}
