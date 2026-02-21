use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::session::CdpSession;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum BrowserAction {
    Navigate { url: String },
    Click { selector: String },
    Type { selector: String, text: String },
    Screenshot,
    Pdf,
    Evaluate { expression: String },
    GetCookies,
    ClearCookies,
    ReadPage,
    GoBack,
    GoForward,
    Reload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub action: String,
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}

impl ActionResult {
    pub fn ok(action: &str, data: Value) -> Self {
        Self {
            action: action.to_string(),
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(action: &str, error: String) -> Self {
        Self {
            action: action.to_string(),
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Executes `BrowserAction` variants against a live CDP session.
pub struct ActionExecutor;

impl ActionExecutor {
    pub async fn execute(session: &CdpSession, action: &BrowserAction) -> ActionResult {
        match action {
            BrowserAction::Navigate { url } => Self::navigate(session, url).await,
            BrowserAction::Click { selector } => Self::click(session, selector).await,
            BrowserAction::Type { selector, text } => {
                Self::type_text(session, selector, text).await
            }
            BrowserAction::Screenshot => Self::screenshot(session).await,
            BrowserAction::Pdf => Self::pdf(session).await,
            BrowserAction::Evaluate { expression } => {
                Self::evaluate(session, expression).await
            }
            BrowserAction::GetCookies => Self::get_cookies(session).await,
            BrowserAction::ClearCookies => Self::clear_cookies(session).await,
            BrowserAction::ReadPage => Self::read_page(session).await,
            BrowserAction::GoBack => Self::go_back(session).await,
            BrowserAction::GoForward => Self::go_forward(session).await,
            BrowserAction::Reload => Self::reload(session).await,
        }
    }

    async fn navigate(session: &CdpSession, url: &str) -> ActionResult {
        match session
            .send_command("Page.navigate", json!({ "url": url }))
            .await
        {
            Ok(result) => {
                if let Some(err_text) = result.get("errorText").and_then(|e| e.as_str()) {
                    return ActionResult::err("navigate", format!("navigation error: {err_text}"));
                }
                let frame_id = result
                    .get("frameId")
                    .and_then(|f| f.as_str())
                    .unwrap_or("")
                    .to_string();
                ActionResult::ok("navigate", json!({ "url": url, "frameId": frame_id }))
            }
            Err(e) => ActionResult::err("navigate", e.to_string()),
        }
    }

    async fn click(session: &CdpSession, selector: &str) -> ActionResult {
        let selector_json =
            serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{}\"", selector));

        let js = format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                if (!el) return JSON.stringify({{error: "element not found"}});
                const rect = el.getBoundingClientRect();
                return JSON.stringify({{
                    x: rect.x + rect.width / 2,
                    y: rect.y + rect.height / 2
                }});
            }})()"#,
            sel = selector_json
        );

        let eval_result = match session
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return ActionResult::err("click", e.to_string()),
        };

        let value_str = eval_result
            .pointer("/result/value")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");

        let coords: Value = serde_json::from_str(value_str).unwrap_or(json!({}));

        if coords.get("error").is_some() {
            return ActionResult::err(
                "click",
                format!("selector '{}' not found on page", selector),
            );
        }

        let x = coords["x"].as_f64().unwrap_or(0.0);
        let y = coords["y"].as_f64().unwrap_or(0.0);

        for event_type in ["mousePressed", "mouseReleased"] {
            if let Err(e) = session
                .send_command(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": event_type,
                        "x": x,
                        "y": y,
                        "button": "left",
                        "clickCount": 1,
                    }),
                )
                .await
            {
                return ActionResult::err("click", e.to_string());
            }
        }

        ActionResult::ok("click", json!({ "selector": selector, "x": x, "y": y }))
    }

    async fn type_text(session: &CdpSession, selector: &str, text: &str) -> ActionResult {
        let selector_json =
            serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{}\"", selector));

        let focus_js = format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                if (!el) return "not_found";
                el.focus();
                return "ok";
            }})()"#,
            sel = selector_json
        );

        let focus_result = match session
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": focus_js, "returnByValue": true }),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return ActionResult::err("type", e.to_string()),
        };

        let focus_status = focus_result
            .pointer("/result/value")
            .and_then(|v| v.as_str())
            .unwrap_or("error");

        if focus_status == "not_found" {
            return ActionResult::err(
                "type",
                format!("selector '{}' not found on page", selector),
            );
        }

        match session
            .send_command("Input.insertText", json!({ "text": text }))
            .await
        {
            Ok(_) => ActionResult::ok(
                "type",
                json!({ "selector": selector, "text": text, "length": text.len() }),
            ),
            Err(e) => ActionResult::err("type", e.to_string()),
        }
    }

    async fn screenshot(session: &CdpSession) -> ActionResult {
        match session
            .send_command(
                "Page.captureScreenshot",
                json!({ "format": "png", "quality": 80 }),
            )
            .await
        {
            Ok(result) => {
                let data = result
                    .get("data")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                let byte_len = data.len() * 3 / 4; // approximate decoded size
                ActionResult::ok(
                    "screenshot",
                    json!({
                        "format": "png",
                        "data_base64_length": data.len(),
                        "approximate_bytes": byte_len,
                        "data": data,
                    }),
                )
            }
            Err(e) => ActionResult::err("screenshot", e.to_string()),
        }
    }

    async fn pdf(session: &CdpSession) -> ActionResult {
        match session
            .send_command("Page.printToPDF", json!({ "printBackground": true }))
            .await
        {
            Ok(result) => {
                let data = result
                    .get("data")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                ActionResult::ok(
                    "pdf",
                    json!({
                        "data_base64_length": data.len(),
                        "data": data,
                    }),
                )
            }
            Err(e) => ActionResult::err("pdf", e.to_string()),
        }
    }

    async fn evaluate(session: &CdpSession, expression: &str) -> ActionResult {
        match session
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await
        {
            Ok(result) => {
                let value = result
                    .get("result")
                    .cloned()
                    .unwrap_or(json!(null));

                if let Some(exception) = result.get("exceptionDetails") {
                    let text = exception
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("JavaScript exception");
                    return ActionResult::err("evaluate", text.to_string());
                }

                ActionResult::ok("evaluate", value)
            }
            Err(e) => ActionResult::err("evaluate", e.to_string()),
        }
    }

    async fn read_page(session: &CdpSession) -> ActionResult {
        let js = r#"JSON.stringify({
            url: location.href,
            title: document.title,
            text: document.body ? document.body.innerText.substring(0, 50000) : "",
            html_length: document.documentElement.outerHTML.length
        })"#;

        match session
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
            )
            .await
        {
            Ok(result) => {
                let raw = result
                    .pointer("/result/value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                let page: Value = serde_json::from_str(raw).unwrap_or(json!({}));
                ActionResult::ok("read_page", page)
            }
            Err(e) => ActionResult::err("read_page", e.to_string()),
        }
    }

    async fn get_cookies(session: &CdpSession) -> ActionResult {
        match session
            .send_command("Network.getCookies", json!({}))
            .await
        {
            Ok(result) => {
                let cookies = result.get("cookies").cloned().unwrap_or(json!([]));
                let count = cookies.as_array().map(|a| a.len()).unwrap_or(0);
                ActionResult::ok("get_cookies", json!({ "cookies": cookies, "count": count }))
            }
            Err(e) => ActionResult::err("get_cookies", e.to_string()),
        }
    }

    async fn clear_cookies(session: &CdpSession) -> ActionResult {
        match session
            .send_command("Network.clearBrowserCookies", json!({}))
            .await
        {
            Ok(_) => ActionResult::ok("clear_cookies", json!({ "cleared": true })),
            Err(e) => ActionResult::err("clear_cookies", e.to_string()),
        }
    }

    async fn go_back(session: &CdpSession) -> ActionResult {
        let js = r#"(() => { history.back(); return "ok"; })()"#;
        match session
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
            )
            .await
        {
            Ok(_) => ActionResult::ok("go_back", json!({ "navigated": "back" })),
            Err(e) => ActionResult::err("go_back", e.to_string()),
        }
    }

    async fn go_forward(session: &CdpSession) -> ActionResult {
        let js = r#"(() => { history.forward(); return "ok"; })()"#;
        match session
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
            )
            .await
        {
            Ok(_) => ActionResult::ok("go_forward", json!({ "navigated": "forward" })),
            Err(e) => ActionResult::err("go_forward", e.to_string()),
        }
    }

    async fn reload(session: &CdpSession) -> ActionResult {
        match session
            .send_command("Page.reload", json!({ "ignoreCache": false }))
            .await
        {
            Ok(_) => ActionResult::ok("reload", json!({ "reloaded": true })),
            Err(e) => ActionResult::err("reload", e.to_string()),
        }
    }
}

/// Maps a `BrowserAction` to the name string used in `ActionResult::action`.
pub fn action_name(action: &BrowserAction) -> &'static str {
    match action {
        BrowserAction::Navigate { .. } => "navigate",
        BrowserAction::Click { .. } => "click",
        BrowserAction::Type { .. } => "type",
        BrowserAction::Screenshot => "screenshot",
        BrowserAction::Pdf => "pdf",
        BrowserAction::Evaluate { .. } => "evaluate",
        BrowserAction::GetCookies => "get_cookies",
        BrowserAction::ClearCookies => "clear_cookies",
        BrowserAction::ReadPage => "read_page",
        BrowserAction::GoBack => "go_back",
        BrowserAction::GoForward => "go_forward",
        BrowserAction::Reload => "reload",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_navigate_serde() {
        let action = BrowserAction::Navigate {
            url: "https://example.com".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("Navigate"));
        assert!(json.contains("https://example.com"));
    }

    #[test]
    fn action_click_serde() {
        let action = BrowserAction::Click {
            selector: "#btn".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: BrowserAction = serde_json::from_str(&json).unwrap();
        match back {
            BrowserAction::Click { selector } => assert_eq!(selector, "#btn"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn action_type_serde() {
        let action = BrowserAction::Type {
            selector: "#input".into(),
            text: "hello".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: BrowserAction = serde_json::from_str(&json).unwrap();
        match back {
            BrowserAction::Type { selector, text } => {
                assert_eq!(selector, "#input");
                assert_eq!(text, "hello");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn action_result_ok() {
        let result = ActionResult::ok("navigate", json!({"url": "https://test.com"}));
        assert!(result.success);
        assert!(result.error.is_none());
        assert_eq!(result.action, "navigate");
    }

    #[test]
    fn action_result_err() {
        let result = ActionResult::err("screenshot", "browser not running".into());
        assert!(!result.success);
        assert!(result.data.is_none());
        assert_eq!(result.error.as_deref(), Some("browser not running"));
    }

    #[test]
    fn all_actions_serialize() {
        let actions = vec![
            BrowserAction::Navigate { url: "u".into() },
            BrowserAction::Click {
                selector: "s".into(),
            },
            BrowserAction::Type {
                selector: "s".into(),
                text: "t".into(),
            },
            BrowserAction::Screenshot,
            BrowserAction::Pdf,
            BrowserAction::Evaluate {
                expression: "1+1".into(),
            },
            BrowserAction::GetCookies,
            BrowserAction::ClearCookies,
            BrowserAction::ReadPage,
            BrowserAction::GoBack,
            BrowserAction::GoForward,
            BrowserAction::Reload,
        ];
        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            assert!(!json.is_empty());
        }
    }

    #[test]
    fn action_names_match() {
        assert_eq!(
            action_name(&BrowserAction::Navigate {
                url: "x".into()
            }),
            "navigate"
        );
        assert_eq!(
            action_name(&BrowserAction::Click {
                selector: "x".into()
            }),
            "click"
        );
        assert_eq!(action_name(&BrowserAction::Screenshot), "screenshot");
        assert_eq!(action_name(&BrowserAction::Pdf), "pdf");
        assert_eq!(
            action_name(&BrowserAction::Evaluate {
                expression: "x".into()
            }),
            "evaluate"
        );
        assert_eq!(action_name(&BrowserAction::GetCookies), "get_cookies");
        assert_eq!(action_name(&BrowserAction::ClearCookies), "clear_cookies");
        assert_eq!(action_name(&BrowserAction::ReadPage), "read_page");
        assert_eq!(action_name(&BrowserAction::GoBack), "go_back");
        assert_eq!(action_name(&BrowserAction::GoForward), "go_forward");
        assert_eq!(action_name(&BrowserAction::Reload), "reload");
    }

    #[test]
    fn action_result_json_roundtrip() {
        let result = ActionResult::ok("evaluate", json!({"value": 42}));
        let json_str = serde_json::to_string(&result).unwrap();
        let back: ActionResult = serde_json::from_str(&json_str).unwrap();
        assert!(back.success);
        assert_eq!(back.action, "evaluate");
        assert_eq!(back.data.unwrap()["value"], 42);
    }
}
