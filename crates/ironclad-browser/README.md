# ironclad-browser

> **Version 0.5.0**

Headless browser automation via Chrome DevTools Protocol (CDP) for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime. Supports 12 browser actions: navigate, click, type, screenshot, evaluate, read page content, reload, back, forward, wait, scroll, and PDF export.

## Key Types

| Type | Module | Description |
|------|--------|-------------|
| `Browser` | `lib` | High-level facade combining process management, CDP, and actions |
| `SharedBrowser` | `lib` | `Arc<Browser>` for thread-safe shared ownership |
| `BrowserAction` | `actions` | Enum of 12 browser actions |
| `ActionResult` | `actions` | Action execution result (success, data, error) |
| `ActionExecutor` | `actions` | Dispatches actions to CDP commands |
| `BrowserManager` | `manager` | Chrome/Chromium process lifecycle |
| `CdpSession` | `session` | WebSocket session to CDP endpoint |
| `CdpClient` | `cdp` | CDP HTTP client for target discovery |
| `PageInfo` | `lib` | Page metadata (id, url, title) |
| `ScreenshotResult` | `lib` | Base64-encoded screenshot with dimensions |

## Usage

```toml
[dependencies]
ironclad-browser = "0.5"
```

```rust
use ironclad_browser::{Browser, BrowserConfig};

let browser = Browser::new(BrowserConfig::default());
browser.start().await?;
let result = browser.execute_action(&BrowserAction::Navigate {
    url: "https://example.com".into(),
}).await;
browser.stop().await?;
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-browser).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
