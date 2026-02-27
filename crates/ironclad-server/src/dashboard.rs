use axum::extract::State;
use axum::response::Html;

use crate::api::AppState;

pub async fn dashboard_handler(State(state): State<AppState>) -> Html<String> {
    let config = state.config.read().await;
    let key = config.server.api_key.as_deref();
    Html(build_dashboard_html(key))
}

pub fn build_dashboard_html(_api_key: Option<&str>) -> String {
    let html = include_str!("dashboard_spa.html");
    let canonical = if let Some(idx) = html.find("</html>") {
        &html[..idx + "</html>".len()]
    } else {
        html
    };
    canonical.replace("var BASE = '';", "var BASE = ''; var API_KEY = null;")
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
    fn dashboard_html_contains_catalog_controls() {
        let html = build_dashboard_html(None);
        assert!(html.contains("/api/skills/catalog"));
        assert!(html.contains("/api/skills/catalog/install"));
        assert!(html.contains("/api/skills/catalog/activate"));
        assert!(html.contains("btn-catalog-install"));
        assert!(html.contains("btn-catalog-install-activate"));
        assert!(html.contains("cat-skill-check"));
    }

    #[test]
    fn dashboard_html_without_key_has_api_health() {
        let html = build_dashboard_html(None);
        assert!(html.contains("<title>Ironclad Dashboard</title>"));
        assert!(html.contains("/api/health"));
    }

    #[test]
    fn dashboard_never_injects_api_key() {
        let html = build_dashboard_html(Some("test-dashboard-key"));
        assert!(
            html.contains("API_KEY = null"),
            "API key must never be embedded"
        );
    }

    #[test]
    fn dashboard_null_api_key_always() {
        let html = build_dashboard_html(None);
        assert!(html.contains("API_KEY = null"));
    }

    #[test]
    fn dashboard_html_contains_single_html_close_tag() {
        let html = build_dashboard_html(None);
        assert_eq!(html.matches("</html>").count(), 1);
    }
}
