use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputCapabilityScan {
    pub requires_filesystem: bool,
    pub requires_network: bool,
    pub requires_environment: bool,
}

#[derive(Debug, Clone)]
struct StringToken {
    key: Option<String>,
    in_path_context: bool,
    in_model_context: bool,
    value: String,
}

pub fn scan_input_capabilities(input: &Value) -> InputCapabilityScan {
    let mut scan = InputCapabilityScan::default();
    let mut tokens = Vec::new();
    collect_strings(input, None, false, false, &mut tokens, &mut scan);

    for token in tokens {
        let value = token.value.trim();
        if value.is_empty() {
            continue;
        }
        if is_url(value) {
            scan.requires_network = true;
        }
        if looks_like_filesystem_path(
            value,
            token.key.as_deref(),
            token.in_path_context,
            token.in_model_context,
        ) {
            scan.requires_filesystem = true;
        }
    }

    scan
}

fn is_path_key(key: &str) -> bool {
    ["path", "file", "filepath", "directory", "dir", "filename"].contains(&key)
}

fn is_model_key(key: &str) -> bool {
    [
        "model",
        "model_id",
        "provider",
        "provider_model",
        "engine",
        "primary",
        "fallback",
        "model_name",
    ]
    .contains(&key)
}

fn is_network_key(key: &str) -> bool {
    ["url", "endpoint", "host", "api"].contains(&key)
}

fn is_environment_key(key: &str) -> bool {
    ["env", "environment", "env_var", "env_key"].contains(&key)
}

fn is_url(v: &str) -> bool {
    let lower = v.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ws://")
        || lower.starts_with("wss://")
}

fn collect_strings(
    value: &Value,
    key_ctx: Option<&str>,
    in_path_context: bool,
    in_model_context: bool,
    out: &mut Vec<StringToken>,
    scan: &mut InputCapabilityScan,
) {
    match value {
        Value::String(s) => out.push(StringToken {
            key: key_ctx.map(|k| k.to_string()),
            in_path_context,
            in_model_context,
            value: s.clone(),
        }),
        Value::Array(arr) => {
            for item in arr {
                collect_strings(item, key_ctx, in_path_context, in_model_context, out, scan);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let lower_key = key.to_lowercase();
                if is_network_key(&lower_key) {
                    scan.requires_network = true;
                }
                if is_environment_key(&lower_key) {
                    scan.requires_environment = true;
                }

                let next_path_context = in_path_context || is_path_key(&lower_key);
                let next_model_context = in_model_context || is_model_key(&lower_key);
                collect_strings(
                    item,
                    Some(&lower_key),
                    next_path_context,
                    next_model_context,
                    out,
                    scan,
                );
            }
        }
        _ => {}
    }
}

fn looks_like_filesystem_path(
    v: &str,
    key: Option<&str>,
    in_path_context: bool,
    in_model_context: bool,
) -> bool {
    if in_path_context || key.is_some_and(is_path_key) {
        return true;
    }
    if is_url(v) {
        return false;
    }
    if v.starts_with('/')
        || v.starts_with("./")
        || v.starts_with("../")
        || v.starts_with("~/")
        || v.starts_with(".\\")
        || v.starts_with("..\\")
        || v.starts_with("~\\")
        || v.starts_with("\\\\")
    {
        return true;
    }
    if v.len() > 2 && v.as_bytes().get(1) == Some(&b':') && matches!(v.as_bytes().get(2), Some(b'\\' | b'/')) {
        return v.as_bytes()[0].is_ascii_alphabetic();
    }
    if in_model_context {
        return false;
    }
    // Treat slash-separated values as path-like after excluding URLs/model context.
    if v.contains('/') && !key.is_some_and(is_model_key) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_and_websocket_values_require_network_only() {
        let scan = scan_input_capabilities(&json!({
            "endpoint": "https://example.com/v1",
            "socket": "wss://stream.example.com",
        }));
        assert!(scan.requires_network);
        assert!(!scan.requires_filesystem);
    }

    #[test]
    fn model_identifier_is_not_filesystem_but_explicit_path_is() {
        let model_scan = scan_input_capabilities(&json!({"model": "openai/gpt-4o"}));
        assert!(!model_scan.requires_filesystem);

        let model_path_scan = scan_input_capabilities(&json!({"model": "/etc/passwd"}));
        assert!(model_path_scan.requires_filesystem);
    }

    #[test]
    fn regex_like_string_is_not_filesystem() {
        let scan = scan_input_capabilities(&json!({"pattern": "\\d+\\w+\\s*"}));
        assert!(!scan.requires_filesystem);
    }

    #[test]
    fn environment_keys_require_environment_capability() {
        let scan = scan_input_capabilities(&json!({"env_var": "SECRET_TOKEN"}));
        assert!(scan.requires_environment);
    }
}
