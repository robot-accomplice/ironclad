use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::debug;

use crate::session::CdpSession;

/// Maximum allowed length (in characters) for `BrowserAction::Evaluate` expressions.
const MAX_EXPRESSION_LENGTH: usize = 100_000;

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
            BrowserAction::Evaluate { expression } => Self::evaluate(session, expression).await,
            BrowserAction::GetCookies => Self::get_cookies(session).await,
            BrowserAction::ClearCookies => Self::clear_cookies(session).await,
            BrowserAction::ReadPage => Self::read_page(session).await,
            BrowserAction::GoBack => Self::go_back(session).await,
            BrowserAction::GoForward => Self::go_forward(session).await,
            BrowserAction::Reload => Self::reload(session).await,
        }
    }

    const BLOCKED_URL_SCHEMES: &[&str] = &[
        "file://",
        "javascript:",
        "data:",
        "chrome://",
        "chrome-extension://",
        "about:",
        "blob:",
    ];

    pub fn is_url_scheme_blocked(url: &str) -> bool {
        let lower = url.trim().to_lowercase();
        Self::BLOCKED_URL_SCHEMES
            .iter()
            .any(|scheme| lower.starts_with(scheme))
    }

    async fn navigate(session: &CdpSession, url: &str) -> ActionResult {
        if Self::is_url_scheme_blocked(url) {
            return ActionResult::err(
                "navigate",
                format!("URL scheme is blocked for security: {url}"),
            );
        }

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
            return ActionResult::err("type", format!("selector '{}' not found on page", selector));
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

    // SECURITY: expressions are controlled by the agent, not end users.
    // The length limit guards against accidental megabyte-sized payloads
    // from prompt injection or runaway template expansion.
    async fn evaluate(session: &CdpSession, expression: &str) -> ActionResult {
        if expression.len() > MAX_EXPRESSION_LENGTH {
            return ActionResult::err(
                "evaluate",
                format!(
                    "expression too large ({} chars, max {})",
                    expression.len(),
                    MAX_EXPRESSION_LENGTH
                ),
            );
        }

        debug!(
            expression_len = expression.len(),
            "evaluating JS expression"
        );

        match session
            .send_command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await
        {
            Ok(result) => {
                let value = result.get("result").cloned().unwrap_or(json!(null));

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
        match session.send_command("Network.getCookies", json!({})).await {
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
            action_name(&BrowserAction::Navigate { url: "x".into() }),
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

    #[test]
    fn blocked_url_schemes() {
        assert!(ActionExecutor::is_url_scheme_blocked("file:///etc/passwd"));
        assert!(ActionExecutor::is_url_scheme_blocked("javascript:alert(1)"));
        assert!(ActionExecutor::is_url_scheme_blocked(
            "data:text/html,<h1>hi</h1>"
        ));
        assert!(ActionExecutor::is_url_scheme_blocked("chrome://settings"));
        assert!(ActionExecutor::is_url_scheme_blocked(
            "chrome-extension://abc/popup.html"
        ));
        assert!(ActionExecutor::is_url_scheme_blocked("about:blank"));
        assert!(ActionExecutor::is_url_scheme_blocked(
            "blob:http://example.com/uuid"
        ));
        assert!(ActionExecutor::is_url_scheme_blocked(
            "  FILE:///etc/passwd"
        ));
    }

    #[test]
    fn allowed_url_schemes() {
        assert!(!ActionExecutor::is_url_scheme_blocked(
            "https://example.com"
        ));
        assert!(!ActionExecutor::is_url_scheme_blocked(
            "http://localhost:3000"
        ));
        assert!(!ActionExecutor::is_url_scheme_blocked(
            "https://google.com/search?q=test"
        ));
    }

    #[test]
    fn action_result_serde_roundtrip_ok() {
        let result = ActionResult::ok("test", json!({"key": "value"}));
        let json_str = serde_json::to_string(&result).unwrap();
        let back: ActionResult = serde_json::from_str(&json_str).unwrap();
        assert!(back.success);
        assert_eq!(back.action, "test");
        assert_eq!(back.data.unwrap()["key"], "value");
        assert!(back.error.is_none());
    }

    #[test]
    fn action_result_serde_roundtrip_err() {
        let result = ActionResult::err("fail_action", "something broke".into());
        let json_str = serde_json::to_string(&result).unwrap();
        let back: ActionResult = serde_json::from_str(&json_str).unwrap();
        assert!(!back.success);
        assert_eq!(back.action, "fail_action");
        assert!(back.data.is_none());
        assert_eq!(back.error.as_deref(), Some("something broke"));
    }

    #[test]
    fn all_action_names_exhaustive() {
        // Verify action_name covers every variant
        let variants: Vec<(BrowserAction, &str)> = vec![
            (BrowserAction::Navigate { url: "x".into() }, "navigate"),
            (
                BrowserAction::Click {
                    selector: "x".into(),
                },
                "click",
            ),
            (
                BrowserAction::Type {
                    selector: "x".into(),
                    text: "y".into(),
                },
                "type",
            ),
            (BrowserAction::Screenshot, "screenshot"),
            (BrowserAction::Pdf, "pdf"),
            (
                BrowserAction::Evaluate {
                    expression: "x".into(),
                },
                "evaluate",
            ),
            (BrowserAction::GetCookies, "get_cookies"),
            (BrowserAction::ClearCookies, "clear_cookies"),
            (BrowserAction::ReadPage, "read_page"),
            (BrowserAction::GoBack, "go_back"),
            (BrowserAction::GoForward, "go_forward"),
            (BrowserAction::Reload, "reload"),
        ];
        for (action, expected_name) in &variants {
            assert_eq!(action_name(action), *expected_name);
        }
    }

    #[test]
    fn action_deserialize_all_variants() {
        let cases = vec![
            r##"{"action":"Navigate","url":"https://example.com"}"##,
            r##"{"action":"Click","selector":"#btn"}"##,
            r##"{"action":"Type","selector":"input","text":"hi"}"##,
            r##"{"action":"Screenshot"}"##,
            r##"{"action":"Pdf"}"##,
            r##"{"action":"Evaluate","expression":"1+1"}"##,
            r##"{"action":"GetCookies"}"##,
            r##"{"action":"ClearCookies"}"##,
            r##"{"action":"ReadPage"}"##,
            r##"{"action":"GoBack"}"##,
            r##"{"action":"GoForward"}"##,
            r##"{"action":"Reload"}"##,
        ];
        for json_str in &cases {
            let action: BrowserAction = serde_json::from_str(json_str).unwrap();
            let reserialized = serde_json::to_string(&action).unwrap();
            assert!(!reserialized.is_empty());
        }
    }

    // ─── Mock CDP session tests ─────────────────────────────────────────
    // These tests create a real WebSocket server that echoes appropriate
    // CDP responses, then connect a CdpSession to it and run
    // ActionExecutor methods.

    use futures_util::{SinkExt, StreamExt};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    /// Spin up a mock CDP server that responds to commands with the given handler.
    async fn mock_cdp_session<F>(handler: F) -> CdpSession
    where
        F: Fn(Value) -> Value + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("ws://127.0.0.1:{port}");

        tokio::spawn(async move {
            if let Ok((stream, _addr)) = listener.accept().await {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut sink, mut source) = ws.split();
                while let Some(Ok(msg)) = source.next().await {
                    if let Message::Text(ref t) = msg
                        && let Ok(req) = serde_json::from_str::<Value>(t)
                    {
                        let resp = handler(req);
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&resp).unwrap()))
                            .await;
                    }
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        CdpSession::connect(&url).await.unwrap()
    }

    #[tokio::test]
    async fn execute_navigate_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"frameId": "frame1"}})
        })
        .await;

        let action = BrowserAction::Navigate {
            url: "https://example.com".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(
            result.success,
            "navigate should succeed: {:?}",
            result.error
        );
        assert_eq!(result.action, "navigate");
        let data = result.data.unwrap();
        assert_eq!(data["url"], "https://example.com");
        assert_eq!(data["frameId"], "frame1");
    }

    #[tokio::test]
    async fn execute_navigate_blocked_scheme() {
        // Don't even need a real session for this, but let's test via execute()
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {}})
        })
        .await;

        let action = BrowserAction::Navigate {
            url: "file:///etc/passwd".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn execute_navigate_with_error_text() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"errorText": "net::ERR_NAME_NOT_RESOLVED"}})
        })
        .await;

        let action = BrowserAction::Navigate {
            url: "https://nonexistent.invalid".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap()
                .contains("ERR_NAME_NOT_RESOLVED")
        );
    }

    #[tokio::test]
    async fn execute_navigate_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Navigation failed"}})
        })
        .await;

        let action = BrowserAction::Navigate {
            url: "https://example.com".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_click_element_found() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            let method = req["method"].as_str().unwrap_or("");
            match method {
                "Runtime.evaluate" => {
                    // Return coordinates
                    json!({"id": id, "result": {"result": {"value": r#"{"x":100,"y":200}"#}}})
                }
                "Input.dispatchMouseEvent" => {
                    json!({"id": id, "result": {}})
                }
                _ => json!({"id": id, "result": {}}),
            }
        })
        .await;

        let action = BrowserAction::Click {
            selector: "#btn".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(result.success, "click should succeed: {:?}", result.error);
        let data = result.data.unwrap();
        assert_eq!(data["x"], 100.0);
        assert_eq!(data["y"], 200.0);
    }

    #[tokio::test]
    async fn execute_click_element_not_found() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"result": {"value": r#"{"error":"element not found"}"#}}})
        })
        .await;

        let action = BrowserAction::Click {
            selector: "#nonexistent".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn execute_type_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            let method = req["method"].as_str().unwrap_or("");
            match method {
                "Runtime.evaluate" => {
                    json!({"id": id, "result": {"result": {"value": "ok"}}})
                }
                "Input.insertText" => {
                    json!({"id": id, "result": {}})
                }
                _ => json!({"id": id, "result": {}}),
            }
        })
        .await;

        let action = BrowserAction::Type {
            selector: "input".into(),
            text: "hello world".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(result.success, "type should succeed: {:?}", result.error);
        let data = result.data.unwrap();
        assert_eq!(data["text"], "hello world");
        assert_eq!(data["length"], 11);
    }

    #[tokio::test]
    async fn execute_type_element_not_found() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"result": {"value": "not_found"}}})
        })
        .await;

        let action = BrowserAction::Type {
            selector: "#missing".into(),
            text: "test".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn execute_screenshot_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"data": "iVBORw0KGgo="}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::Screenshot).await;
        assert!(
            result.success,
            "screenshot should succeed: {:?}",
            result.error
        );
        let data = result.data.unwrap();
        assert_eq!(data["format"], "png");
        assert!(data["data_base64_length"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn execute_screenshot_no_data() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::Screenshot).await;
        assert!(result.success);
        let data = result.data.unwrap();
        assert_eq!(data["data"], "");
    }

    #[tokio::test]
    async fn execute_pdf_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"data": "JVBERi0xLjQ="}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::Pdf).await;
        assert!(result.success, "pdf should succeed: {:?}", result.error);
        let data = result.data.unwrap();
        assert!(data["data_base64_length"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn execute_evaluate_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"result": {"type": "number", "value": 42}}})
        })
        .await;

        let action = BrowserAction::Evaluate {
            expression: "21 * 2".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(
            result.success,
            "evaluate should succeed: {:?}",
            result.error
        );
        let data = result.data.unwrap();
        assert_eq!(data["value"], 42);
    }

    #[tokio::test]
    async fn execute_evaluate_expression_too_large() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {}})
        })
        .await;

        let big_expr = "x".repeat(MAX_EXPRESSION_LENGTH + 1);
        let action = BrowserAction::Evaluate {
            expression: big_expr,
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("too large"));
    }

    #[tokio::test]
    async fn execute_evaluate_js_exception() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({
                "id": id,
                "result": {
                    "result": {"type": "object"},
                    "exceptionDetails": {
                        "text": "ReferenceError: foo is not defined"
                    }
                }
            })
        })
        .await;

        let action = BrowserAction::Evaluate {
            expression: "foo()".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("ReferenceError"));
    }

    #[tokio::test]
    async fn execute_evaluate_exception_no_text() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({
                "id": id,
                "result": {
                    "result": {"type": "object"},
                    "exceptionDetails": {}
                }
            })
        })
        .await;

        let action = BrowserAction::Evaluate {
            expression: "bad()".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap()
                .contains("JavaScript exception")
        );
    }

    #[tokio::test]
    async fn execute_read_page_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            let page_json = serde_json::to_string(&json!({
                "url": "https://example.com",
                "title": "Example",
                "text": "Hello World",
                "html_length": 1234
            }))
            .unwrap();
            json!({"id": id, "result": {"result": {"value": page_json}}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::ReadPage).await;
        assert!(
            result.success,
            "read_page should succeed: {:?}",
            result.error
        );
        let data = result.data.unwrap();
        assert_eq!(data["url"], "https://example.com");
        assert_eq!(data["title"], "Example");
    }

    #[tokio::test]
    async fn execute_get_cookies_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"cookies": [{"name": "sid", "value": "abc"}]}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::GetCookies).await;
        assert!(
            result.success,
            "get_cookies should succeed: {:?}",
            result.error
        );
        let data = result.data.unwrap();
        assert_eq!(data["count"], 1);
    }

    #[tokio::test]
    async fn execute_clear_cookies_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::ClearCookies).await;
        assert!(
            result.success,
            "clear_cookies should succeed: {:?}",
            result.error
        );
        let data = result.data.unwrap();
        assert_eq!(data["cleared"], true);
    }

    #[tokio::test]
    async fn execute_go_back_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"result": {"value": "ok"}}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::GoBack).await;
        assert!(result.success, "go_back should succeed: {:?}", result.error);
        assert_eq!(result.data.unwrap()["navigated"], "back");
    }

    #[tokio::test]
    async fn execute_go_forward_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"result": {"value": "ok"}}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::GoForward).await;
        assert!(
            result.success,
            "go_forward should succeed: {:?}",
            result.error
        );
        assert_eq!(result.data.unwrap()["navigated"], "forward");
    }

    #[tokio::test]
    async fn execute_reload_success() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::Reload).await;
        assert!(result.success, "reload should succeed: {:?}", result.error);
        assert_eq!(result.data.unwrap()["reloaded"], true);
    }

    #[tokio::test]
    async fn execute_navigate_cdp_send_error() {
        // Test when the CDP session returns an error (not a CDP protocol error)
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32601, "message": "Method not found"}})
        })
        .await;

        let action = BrowserAction::Navigate {
            url: "https://example.com".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_click_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Target closed"}})
        })
        .await;

        let action = BrowserAction::Click {
            selector: "#btn".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_type_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Target closed"}})
        })
        .await;

        let action = BrowserAction::Type {
            selector: "input".into(),
            text: "hello".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_screenshot_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Target closed"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::Screenshot).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_pdf_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Printing failed"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::Pdf).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_evaluate_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Runtime error"}})
        })
        .await;

        let action = BrowserAction::Evaluate {
            expression: "1+1".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_read_page_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Eval failed"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::ReadPage).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_get_cookies_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Network error"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::GetCookies).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_clear_cookies_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Clear failed"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::ClearCookies).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_go_back_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Navigation failed"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::GoBack).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_go_forward_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Navigation failed"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::GoForward).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_reload_cdp_error() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "error": {"code": -32000, "message": "Reload failed"}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::Reload).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_click_mouse_event_error() {
        // First eval succeeds with coords, but mouse dispatch fails
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering as AtomOrd};

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("ws://127.0.0.1:{port}");

        tokio::spawn(async move {
            if let Ok((stream, _addr)) = listener.accept().await {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut sink, mut source) = ws.split();
                while let Some(Ok(msg)) = source.next().await {
                    if let Message::Text(ref t) = msg
                        && let Ok(req) = serde_json::from_str::<Value>(t)
                    {
                        let id = req["id"].as_u64().unwrap();
                        let method = req["method"].as_str().unwrap_or("");
                        let _n = call_count_clone.fetch_add(1, AtomOrd::SeqCst);

                        let resp = match method {
                            "Runtime.evaluate" => {
                                json!({"id": id, "result": {"result": {"value": r#"{"x":50,"y":50}"#}}})
                            }
                            "Input.dispatchMouseEvent" => {
                                json!({"id": id, "error": {"code": -32000, "message": "Input error"}})
                            }
                            _ => json!({"id": id, "result": {}}),
                        };
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&resp).unwrap()))
                            .await;
                    }
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let session = CdpSession::connect(&url).await.unwrap();

        let action = BrowserAction::Click {
            selector: "#btn".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_type_insert_text_error() {
        // Focus succeeds but insert text fails
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("ws://127.0.0.1:{port}");

        tokio::spawn(async move {
            if let Ok((stream, _addr)) = listener.accept().await {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut sink, mut source) = ws.split();
                while let Some(Ok(msg)) = source.next().await {
                    if let Message::Text(ref t) = msg
                        && let Ok(req) = serde_json::from_str::<Value>(t)
                    {
                        let id = req["id"].as_u64().unwrap();
                        let method = req["method"].as_str().unwrap_or("");
                        let resp = match method {
                            "Runtime.evaluate" => {
                                json!({"id": id, "result": {"result": {"value": "ok"}}})
                            }
                            "Input.insertText" => {
                                json!({"id": id, "error": {"code": -32000, "message": "Insert failed"}})
                            }
                            _ => json!({"id": id, "result": {}}),
                        };
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&resp).unwrap()))
                            .await;
                    }
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let session = CdpSession::connect(&url).await.unwrap();

        let action = BrowserAction::Type {
            selector: "input".into(),
            text: "hello".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_read_page_invalid_json() {
        // Server returns something that is not valid JSON in the value
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {"result": {"value": "not valid json"}}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::ReadPage).await;
        // Should still succeed with fallback to empty object
        assert!(result.success);
    }

    #[tokio::test]
    async fn execute_get_cookies_empty() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {}})
        })
        .await;

        let result = ActionExecutor::execute(&session, &BrowserAction::GetCookies).await;
        assert!(result.success);
        let data = result.data.unwrap();
        assert_eq!(data["count"], 0);
    }

    #[tokio::test]
    async fn execute_navigate_no_frame_id() {
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            json!({"id": id, "result": {}})
        })
        .await;

        let action = BrowserAction::Navigate {
            url: "https://example.com".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        assert!(result.success);
        let data = result.data.unwrap();
        assert_eq!(data["frameId"], "");
    }

    #[tokio::test]
    async fn execute_click_no_coords_in_response() {
        // Element found but coords are missing from response
        let session = mock_cdp_session(|req| {
            let id = req["id"].as_u64().unwrap();
            let method = req["method"].as_str().unwrap_or("");
            match method {
                "Runtime.evaluate" => {
                    json!({"id": id, "result": {"result": {"value": "{}"}}})
                }
                "Input.dispatchMouseEvent" => {
                    json!({"id": id, "result": {}})
                }
                _ => json!({"id": id, "result": {}}),
            }
        })
        .await;

        let action = BrowserAction::Click {
            selector: "#btn".into(),
        };
        let result = ActionExecutor::execute(&session, &action).await;
        // Should succeed (defaults to 0,0 coords)
        assert!(result.success);
    }
}
