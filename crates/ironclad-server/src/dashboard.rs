use axum::extract::State;
use axum::response::Html;

use crate::api::AppState;

pub async fn dashboard_handler(State(state): State<AppState>) -> Html<String> {
    let config = state.config.read().await;
    let key = config.server.api_key.as_deref();
    Html(build_dashboard_html(key))
}

pub fn build_dashboard_html(api_key: Option<&str>) -> String {
    let html = include_str!("dashboard_spa.html");
    match api_key {
        Some(key) => {
            let escaped = key.replace('\\', "\\\\").replace('\'', "\\'");
            html.replace(
                "var BASE = '';",
                &format!("var BASE = ''; var API_KEY = '{}';", escaped),
            )
        }
        None => html.replace(
            "var BASE = '';",
            "var BASE = ''; var API_KEY = null;",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_html_contains_title() {
        let html = build_dashboard_html(None);
        assert!(html.contains("<title>Ironclad Dashboard</title>"));
        assert!(html.contains("/api/health"));
    }

    #[test]
    fn build_dashboard_html_contains_all_sections() {
        let html = build_dashboard_html(None);
        assert!(html.contains("Ironclad"));
        assert!(html.contains("Autonomous Agent Runtime"));
        assert!(html.contains("/api/sessions"));
        assert!(html.contains("/api/memory/episodic"));
        assert!(html.contains("/api/cron/jobs"));
        assert!(html.contains("/api/stats/costs"));
        assert!(html.contains("/api/skills"));
        assert!(html.contains("/api/wallet/balance"));
        assert!(html.contains("/api/breaker/status"));
    }

    #[test]
    fn dashboard_html_without_key_has_api_health() {
        let html = build_dashboard_html(None);
        assert!(html.contains("<title>Ironclad Dashboard</title>"));
        assert!(html.contains("/api/health"));
    }

    #[test]
    fn dashboard_injects_api_key() {
        let html = build_dashboard_html(Some("my-secret-key"));
        assert!(html.contains("API_KEY = 'my-secret-key'"));
    }

    #[test]
    fn dashboard_null_api_key_when_none() {
        let html = build_dashboard_html(None);
        assert!(html.contains("API_KEY = null"));
    }
}
