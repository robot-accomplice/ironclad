use super::*;
use crate::config_runtime;
use ironclad_core::IroncladConfig;

// ── Config (show from API) ────────────────────────────────────

pub async fn cmd_config(url: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/config").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    heading("Configuration");

    let sections = [
        "agent",
        "server",
        "database",
        "models",
        "memory",
        "cache",
        "treasury",
        "yield",
        "wallet",
        "a2a",
        "skills",
        "channels",
        "circuit_breaker",
        "providers",
    ];

    for section in sections {
        if let Some(val) = data.get(section) {
            if val.is_null() {
                continue;
            }
            eprintln!();
            eprintln!("    {DETAIL} {section}{RESET}");
            print_json_section(val, 6);
        }
    }

    eprintln!();
    Ok(())
}

// ── Config (get/set/unset from file) ───────────────────────────

fn find_config_file() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let candidates = [
        std::path::PathBuf::from("ironclad.toml"),
        dirs_home().join("ironclad.toml"),
    ];
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    Err("No ironclad.toml found in current directory or ~/.ironclad/".into())
}

fn dirs_home() -> std::path::PathBuf {
    ironclad_core::home_dir().join(".ironclad")
}

fn navigate_toml<'a>(table: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = table;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}

fn format_toml_value(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(a) => {
            let items: Vec<String> = a.iter().map(format_toml_value).collect();
            format!("[{}]", items.join(", "))
        }
        toml::Value::Table(_) => toml::to_string_pretty(v).unwrap_or_else(|_| format!("{v:?}")),
        toml::Value::Datetime(d) => d.to_string(),
    }
}

fn set_toml_value(
    table: &mut toml::Value,
    path: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = table;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let toml::Value::Table(map) = current {
                let parsed_value = if matches!(map.get(*part), Some(toml::Value::Array(_))) {
                    parse_toml_value_for_existing_array(value)
                } else {
                    parse_toml_value(value)
                };
                map.insert(part.to_string(), parsed_value);
            }
        } else {
            if current.get(part).is_none()
                && let toml::Value::Table(map) = current
            {
                map.insert(part.to_string(), toml::Value::Table(toml::map::Map::new()));
            }
            current = current
                .get_mut(part)
                .ok_or_else(|| format!("cannot navigate to {part}"))?;
        }
    }

    Ok(())
}

fn remove_toml_key(table: &mut toml::Value, path: &str) -> bool {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.len() == 1 {
        if let toml::Value::Table(map) = table {
            return map.remove(parts[0]).is_some();
        }
        return false;
    }

    let mut current = table;
    for part in &parts[..parts.len() - 1] {
        current = match current.get_mut(part) {
            Some(v) => v,
            None => return false,
        };
    }

    if let toml::Value::Table(map) = current {
        parts
            .last()
            .map(|p| map.remove(*p).is_some())
            .unwrap_or(false)
    } else {
        false
    }
}

fn parse_toml_value_for_existing_array(s: &str) -> toml::Value {
    let trimmed = s.trim();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return parse_toml_value(trimmed);
    }
    let items: Vec<toml::Value> = trimmed
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(parse_toml_value)
        .collect();
    toml::Value::Array(items)
}

fn parse_toml_value(s: &str) -> toml::Value {
    if let Ok(parsed) = format!("value = {s}").parse::<toml::Table>()
        && let Some(value) = parsed.get("value")
    {
        return value.clone();
    }
    if s == "true" {
        return toml::Value::Boolean(true);
    }
    if s == "false" {
        return toml::Value::Boolean(false);
    }
    if let Ok(i) = s.parse::<i64>() {
        return toml::Value::Integer(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        return toml::Value::Float(f);
    }
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        let items: Vec<toml::Value> = inner
            .split(',')
            .map(|item| parse_toml_value(item.trim().trim_matches('"')))
            .collect();
        return toml::Value::Array(items);
    }
    toml::Value::String(s.trim_matches('"').to_string())
}

pub async fn cmd_config_get(url: &str, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Try live API first — shows actual runtime config
    if let Ok(client) = crate::cli::IroncladClient::new(url)
        && let Ok(live) = client.get("/api/config").await
    {
        let value = navigate_json(&live, path);
        match value {
            Some(v) => {
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            None => {
                eprintln!("  Key not found: {path}");
                std::process::exit(1);
            }
        }
    }

    // Fall back to on-disk TOML when server is not running
    let config_path = find_config_file()?;
    let contents = std::fs::read_to_string(&config_path)?;
    let table: toml::Value = contents.parse()?;

    let value = navigate_toml(&table, path);
    match value {
        Some(v) => {
            println!("{}", format_toml_value(v));
        }
        None => {
            eprintln!("  Key not found: {path}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Navigate a serde_json::Value by dot-separated path
fn navigate_json<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}

pub fn cmd_config_set(
    path: &str,
    value: &str,
    file: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let contents = std::fs::read_to_string(file).unwrap_or_else(|_| String::new());
    let mut table: toml::Value = if contents.is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        contents.parse()?
    };

    set_toml_value(&mut table, path, value)?;

    let output = toml::to_string_pretty(&table)?;
    std::fs::write(file, output)?;
    println!("  {OK} Set {path} = {value} in {file}");
    Ok(())
}

pub fn cmd_config_unset(path: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let contents = std::fs::read_to_string(file)?;
    let mut table: toml::Value = contents.parse()?;

    if remove_toml_key(&mut table, path) {
        let output = toml::to_string_pretty(&table)?;
        std::fs::write(file, output)?;
        println!("  {OK} Removed {path} from {file}");
    } else {
        eprintln!("  Key not found: {path}");
    }
    Ok(())
}

pub fn cmd_config_lint(file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (OK, _, _, _, _) = icons();
    let _cfg = IroncladConfig::from_file(std::path::Path::new(file))?;
    println!("  {OK} Config lint passed: {file}");
    Ok(())
}

pub fn cmd_config_backup(file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (OK, _, _, _, _) = icons();
    let path = std::path::Path::new(file);
    // Try to read backup limits from the config itself; fall back to defaults
    // if the file is missing or unparseable (we still want the backup to succeed).
    let backups = IroncladConfig::from_file(path)
        .map(|c| c.backups)
        .unwrap_or_default();
    match config_runtime::backup_config_file(path, backups.max_count, backups.max_age_days)? {
        Some(backup) => println!("  {OK} Backup created: {}", backup.display()),
        None => println!("  {OK} No backup needed; config file does not exist: {file}"),
    }
    Ok(())
}

pub async fn cmd_config_apply(url: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (OK, _, WARN, _, _) = icons();
    let cfg = IroncladConfig::from_file(std::path::Path::new(file))?;
    let c = IroncladClient::new(url)?;
    let payload = serde_json::to_value(cfg)?;
    match c.put("/api/config", payload).await {
        Ok(_) => {
            println!("  {OK} Runtime apply succeeded via /api/config");
            Ok(())
        }
        Err(e) => {
            eprintln!(
                "  {WARN} Config file updated, but runtime apply failed (server unavailable?): {e}"
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_toml_simple_key() {
        let toml: toml::Value = "[agent]\nname = \"Duncan\"".parse().unwrap();
        let result = navigate_toml(&toml, "agent.name");
        assert_eq!(result.unwrap().as_str().unwrap(), "Duncan");
    }

    #[test]
    fn navigate_toml_missing_key() {
        let toml: toml::Value = "[agent]\nname = \"Duncan\"".parse().unwrap();
        assert!(navigate_toml(&toml, "agent.missing").is_none());
    }

    #[test]
    fn navigate_toml_top_level() {
        let toml: toml::Value = "port = 8080".parse().unwrap();
        let result = navigate_toml(&toml, "port");
        assert_eq!(result.unwrap().as_integer().unwrap(), 8080);
    }

    #[test]
    fn navigate_toml_deeply_nested() {
        let toml: toml::Value = "[a.b.c]\nval = true".parse().unwrap();
        let result = navigate_toml(&toml, "a.b.c.val");
        assert!(result.unwrap().as_bool().unwrap());
    }

    #[test]
    fn format_toml_value_string() {
        let v = toml::Value::String("hello".into());
        assert_eq!(format_toml_value(&v), "hello");
    }

    #[test]
    fn format_toml_value_integer() {
        assert_eq!(format_toml_value(&toml::Value::Integer(42)), "42");
    }

    #[test]
    fn format_toml_value_float() {
        assert_eq!(format_toml_value(&toml::Value::Float(2.72)), "2.72");
    }

    #[test]
    fn format_toml_value_bool() {
        assert_eq!(format_toml_value(&toml::Value::Boolean(true)), "true");
        assert_eq!(format_toml_value(&toml::Value::Boolean(false)), "false");
    }

    #[test]
    fn format_toml_value_array() {
        let v = toml::Value::Array(vec![
            toml::Value::String("a".into()),
            toml::Value::String("b".into()),
        ]);
        assert_eq!(format_toml_value(&v), "[a, b]");
    }

    #[test]
    fn format_toml_value_table() {
        let mut map = toml::map::Map::new();
        map.insert("x".into(), toml::Value::Integer(1));
        let v = toml::Value::Table(map);
        let s = format_toml_value(&v);
        assert!(s.contains("x"));
    }

    #[test]
    fn parse_toml_value_bool_true() {
        assert_eq!(parse_toml_value("true"), toml::Value::Boolean(true));
    }

    #[test]
    fn parse_toml_value_bool_false() {
        assert_eq!(parse_toml_value("false"), toml::Value::Boolean(false));
    }

    #[test]
    fn parse_toml_value_integer() {
        assert_eq!(parse_toml_value("42"), toml::Value::Integer(42));
    }

    #[test]
    fn parse_toml_value_negative_integer() {
        assert_eq!(parse_toml_value("-1"), toml::Value::Integer(-1));
    }

    #[test]
    fn parse_toml_value_float() {
        assert_eq!(parse_toml_value("2.72"), toml::Value::Float(2.72));
    }

    #[test]
    fn parse_toml_value_string() {
        assert_eq!(
            parse_toml_value("hello"),
            toml::Value::String("hello".into())
        );
    }

    #[test]
    fn parse_toml_value_quoted_string() {
        assert_eq!(
            parse_toml_value("\"hello\""),
            toml::Value::String("hello".into())
        );
    }

    #[test]
    fn parse_toml_value_array() {
        let result = parse_toml_value("[a, b, c]");
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn parse_toml_value_inline_table_array() {
        let result = parse_toml_value(
            r#"[{ chain = "ETH", target_contract_address = "0x1", swap_contract_address = "0x2" }]"#,
        );
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let first = arr[0].as_table().unwrap();
        assert!(first.get("chain").is_some());
    }

    #[test]
    fn set_toml_value_existing_key() {
        let mut table: toml::Value = "[server]\nport = 8080".parse().unwrap();
        set_toml_value(&mut table, "server.port", "9090").unwrap();
        assert_eq!(
            navigate_toml(&table, "server.port")
                .unwrap()
                .as_integer()
                .unwrap(),
            9090
        );
    }

    #[test]
    fn set_toml_value_new_section() {
        let mut table = toml::Value::Table(toml::map::Map::new());
        set_toml_value(&mut table, "new_section.key", "value").unwrap();
        assert_eq!(
            navigate_toml(&table, "new_section.key")
                .unwrap()
                .as_str()
                .unwrap(),
            "value"
        );
    }

    #[test]
    fn set_toml_value_top_level() {
        let mut table = toml::Value::Table(toml::map::Map::new());
        set_toml_value(&mut table, "name", "test").unwrap();
        assert_eq!(table.get("name").unwrap().as_str().unwrap(), "test");
    }

    #[test]
    fn set_toml_value_existing_array_accepts_csv() {
        let mut table: toml::Value = "[channels]\nstartup_announcements = [\"telegram\"]"
            .parse()
            .unwrap();
        set_toml_value(
            &mut table,
            "channels.startup_announcements",
            "telegram,signal,email",
        )
        .unwrap();
        let arr = navigate_toml(&table, "channels.startup_announcements")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[1].as_str().unwrap(), "signal");
    }

    #[test]
    fn remove_toml_key_existing() {
        let mut table: toml::Value = "[agent]\nname = \"Duncan\"".parse().unwrap();
        assert!(remove_toml_key(&mut table, "agent.name"));
        assert!(navigate_toml(&table, "agent.name").is_none());
    }

    #[test]
    fn remove_toml_key_missing() {
        let mut table: toml::Value = "[agent]\nname = \"Duncan\"".parse().unwrap();
        assert!(!remove_toml_key(&mut table, "agent.missing"));
    }

    #[test]
    fn remove_toml_key_top_level() {
        let mut table: toml::Value = "port = 8080\nname = \"test\"".parse().unwrap();
        assert!(remove_toml_key(&mut table, "port"));
        assert!(table.get("port").is_none());
        assert!(table.get("name").is_some());
    }

    #[test]
    fn remove_toml_key_from_non_table() {
        let mut table = toml::Value::String("not a table".into());
        assert!(!remove_toml_key(&mut table, "anything"));
    }

    #[test]
    fn dirs_home_contains_ironclad() {
        let p = dirs_home();
        assert!(p.to_string_lossy().contains(".ironclad"));
    }
}
