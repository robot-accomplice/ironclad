use super::*;

// ── Config (show from API) ────────────────────────────────────

pub async fn cmd_config(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let c = IroncladClient::new(url)?;
    let data = c.get("/api/config").await.map_err(|e| {
        IroncladClient::check_connectivity_hint(&*e);
        e
    })?;

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
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".ironclad")
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
                let parsed_value = parse_toml_value(value);
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

fn parse_toml_value(s: &str) -> toml::Value {
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

pub fn cmd_config_get(path: &str) -> Result<(), Box<dyn std::error::Error>> {
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
