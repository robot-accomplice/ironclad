use super::*;

// ── Setup wizard & starter skills ─────────────────────────────

pub const STARTER_SKILLS: &[(&str, &str)] = &[
    (
        "hello.md",
        r#"---
name: hello
description: Greet the user and introduce the agent
triggers:
  keywords: [hello, hi, greet, introduce, who are you]
priority: 5
---

Greet the user warmly. Introduce yourself by name and briefly describe what you can do.
Keep it concise -- one or two sentences. If the user seems new, mention they can
ask for help at any time.
"#,
    ),
    (
        "summarize.md",
        r#"---
name: summarize
description: Summarize text, articles, or conversations
triggers:
  keywords: [summarize, summary, tldr, recap, brief]
priority: 7
---

Summarize the provided content clearly and concisely. Structure your summary as:
1. A one-sentence overview
2. Key points (3-5 bullets)
3. Any action items or decisions mentioned

If no content is provided, ask the user what they'd like summarized.
Preserve important details like names, dates, and numbers.
"#,
    ),
    (
        "explain.md",
        r#"---
name: explain
description: Explain concepts, code, or ideas in plain language
triggers:
  keywords: [explain, what is, how does, break down, eli5, teach]
priority: 7
---

Explain the topic clearly, adjusting depth to the user's apparent level of expertise.
Start with a simple one-sentence definition, then expand with relevant details.
Use analogies when helpful. If explaining code, walk through the logic step by step.
If the concept is complex, break it into numbered parts.
"#,
    ),
    (
        "draft.md",
        r#"---
name: draft
description: Draft emails, messages, documents, or other written content
triggers:
  keywords: [draft, write, compose, email, letter, message, document]
priority: 6
---

Draft the requested content based on the user's description. Ask clarifying questions
if the audience, tone, or purpose is unclear. Default to a professional but approachable
tone unless told otherwise. Present the draft clearly and offer to revise.
"#,
    ),
    (
        "review.md",
        r#"---
name: review
description: Review code, text, or plans and provide constructive feedback
triggers:
  keywords: [review, check, feedback, critique, improve, proofread]
priority: 7
---

Review the provided content carefully. Organize your feedback into:
- **Issues**: things that are wrong or could cause problems
- **Suggestions**: improvements that would make it better
- **Strengths**: what's already working well

Be specific -- reference exact lines or passages. Prioritize the most impactful
feedback first. Keep the tone constructive.
"#,
    ),
    (
        "plan.md",
        r#"---
name: plan
description: Break down tasks or projects into actionable steps
triggers:
  keywords: [plan, steps, breakdown, roadmap, how to, strategy, approach]
priority: 6
---

Break the task into clear, actionable steps. For each step:
1. State what to do
2. Note any prerequisites or dependencies
3. Estimate relative effort (quick / moderate / significant) if helpful

Order steps logically. Flag any risks or decisions that need to be made before
proceeding. Keep the plan practical -- prefer concrete actions over vague guidance.
"#,
    ),
    (
        "search.md",
        r#"---
name: search
description: Help find information, files, or answers
triggers:
  keywords: [find, search, look up, locate, where is, which]
priority: 5
---

Help the user find what they're looking for. If you have access to relevant tools
or context, use them. If the query is ambiguous, ask one clarifying question before
searching. Present results clearly with sources or locations when available.
"#,
    ),
];

pub fn write_starter_skills(skills_dir: &std::path::Path) -> std::io::Result<usize> {
    let mut written = 0;
    for (filename, content) in STARTER_SKILLS {
        let path = skills_dir.join(filename);
        if !path.exists() {
            std::fs::write(&path, content)?;
            written += 1;
        }
    }
    Ok(written)
}

const APERTUS_8B_SUFFIX: &str = "apertus-8b-instruct:latest";
const APERTUS_70B_SUFFIX: &str = "apertus-70b-instruct:latest";

fn detect_system_ram_gb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("sh")
            .args(["-c", "awk '/MemTotal/ {print $2}' /proc/meminfo"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let kb = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()?;
        return Some(kb / 1024 / 1024);
    }

    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let bytes = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()?;
        return Some(bytes / 1024 / 1024 / 1024);
    }

    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let bytes = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()?;
        return Some(bytes / 1024 / 1024 / 1024);
    }

    #[allow(unreachable_code)]
    None
}

fn aperture_options_for_provider(provider_prefix: &str, ram_gb: Option<u64>) -> Vec<String> {
    let mut options = vec![format!("{provider_prefix}/{APERTUS_8B_SUFFIX}")];
    if ram_gb.map(|v| v >= 64).unwrap_or(false) {
        options.push(format!("{provider_prefix}/{APERTUS_70B_SUFFIX}"));
    }
    options
}

fn has_hf_model_cache() -> bool {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    let hf_home = std::env::var("HF_HOME")
        .ok()
        .or_else(|| home.as_ref().map(|h| format!("{h}/.cache/huggingface")));
    let hub_dir = match hf_home {
        Some(v) => std::path::PathBuf::from(v).join("hub"),
        None => return false,
    };
    if !hub_dir.exists() {
        return false;
    }
    std::fs::read_dir(&hub_dir)
        .ok()
        .map(|iter| {
            iter.filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().starts_with("models--"))
        })
        .unwrap_or(false)
}

fn has_ollama_models() -> bool {
    if which_binary("ollama").is_none() {
        return false;
    }
    let output = std::process::Command::new("ollama").arg("list").output();
    let Ok(out) = output else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let line_count = String::from_utf8_lossy(&out.stdout).lines().count();
    line_count > 1
}

fn has_existing_local_model_stack() -> bool {
    let has_framework = [
        "sglang",
        "vllm",
        "docker",
        "ollama",
        "llama-server",
        "llama_cpp",
    ]
    .iter()
    .any(|bin| which_binary(bin).is_some());
    has_framework || has_ollama_models() || has_hf_model_cache()
}

fn run_quick_personality_setup(
    workspace: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    use dialoguer::{Input, Select};

    let name: String = Input::new()
        .with_prompt("  Agent name")
        .default("Roboticus".into())
        .interact_text()?;

    let formality_options = vec!["formal", "balanced", "casual"];
    let formality_idx = Select::new()
        .with_prompt("  Communication style")
        .items(&formality_options)
        .default(1)
        .interact()?;

    let proactive_options = vec![
        "wait (only act when told)",
        "suggest (flag opportunities, ask first)",
        "initiative (act proactively)",
    ];
    let proactive_idx = Select::new()
        .with_prompt("  Proactiveness level")
        .items(&proactive_options)
        .default(1)
        .interact()?;
    let proactive_val = match proactive_idx {
        0 => "wait",
        2 => "initiative",
        _ => "suggest",
    };

    let domain_options = vec!["general", "developer", "business", "creative", "research"];
    let domain_idx = Select::new()
        .with_prompt("  Primary domain")
        .items(&domain_options)
        .default(0)
        .interact()?;

    let boundaries: String = Input::new()
        .with_prompt(
            "  Any hard boundaries? (topics/actions that are off-limits, or press Enter to skip)",
        )
        .allow_empty(true)
        .interact_text()?;

    let os_toml = ironclad_core::personality::generate_os_toml(
        &name,
        formality_options[formality_idx],
        proactive_val,
        domain_options[domain_idx],
    );
    let fw_toml = ironclad_core::personality::generate_firmware_toml(&boundaries);

    std::fs::create_dir_all(workspace)?;
    std::fs::write(workspace.join("OS.toml"), &os_toml)?;
    std::fs::write(workspace.join("FIRMWARE.toml"), &fw_toml)?;

    println!("  {OK} Personality configured for {BOLD}{name}{RESET} (OS.toml + FIRMWARE.toml)");

    Ok(())
}

pub fn cmd_setup() -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    use dialoguer::{Confirm, Input, Select};

    println!("\n  {BOLD}Ironclad Setup Wizard{RESET}\n");
    println!("  This wizard will help you create an ironclad.toml configuration.\n");

    // Prerequisites: Go + gosh (plugin scripting engine)
    println!("  {BOLD}Checking prerequisites...{RESET}\n");
    let go_bin = which_binary("go");
    let has_go = go_bin.is_some();
    let has_gosh = which_binary("gosh").is_some();

    if !has_go {
        println!("  {WARN} Go is not installed (required for the gosh plugin engine).");
        println!(
            "     Install from {CYAN}https://go.dev/dl/{RESET} or: {MONO}brew install go{RESET}"
        );
        println!();
        let proceed = Confirm::new()
            .with_prompt(
                "  Continue without Go? (plugins won't work until Go + gosh are installed)",
            )
            .default(true)
            .interact()?;
        if !proceed {
            println!("\n  Setup paused. Install Go, then re-run {BOLD}ironclad init{RESET}.\n");
            return Ok(());
        }
    } else if !has_gosh {
        println!("  {OK} Go found");
        println!("  {WARN} gosh scripting engine not found.");
        let install_now = Confirm::new()
            .with_prompt("  Install gosh now via `go install`?")
            .default(true)
            .interact()?;
        if install_now {
            println!("  Installing gosh...");
            let result = if let Some(go_path) = go_bin.as_deref() {
                std::process::Command::new(go_path)
                    .args(["install", "github.com/drewwalton19216801/gosh@latest"])
                    .status()
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "go binary not found",
                ))
            };
            match result {
                Ok(s) if s.success() => {
                    println!("  {OK} gosh installed successfully");
                }
                _ => {
                    println!("  {WARN} gosh installation failed. Install manually:");
                    println!(
                        "     {MONO}go install github.com/drewwalton19216801/gosh@latest{RESET}"
                    );
                }
            }
        } else {
            println!(
                "  Skipped. Install later: {MONO}go install github.com/drewwalton19216801/gosh@latest{RESET}"
            );
        }
    } else {
        println!("  {OK} Go found");
        println!("  {OK} gosh scripting engine found");
    }
    println!();

    // 1. Agent name
    let agent_name: String = Input::new()
        .with_prompt("  Agent name")
        .default("Roboticus".into())
        .interact_text()?;

    let offer_apertus_onboarding = !has_existing_local_model_stack();
    if !offer_apertus_onboarding {
        println!(
            "  {DETAIL} Existing local model framework/model cache detected; skipping automatic SGLang + Apertus recommendation."
        );
    }

    // 2. LLM provider
    let providers = if offer_apertus_onboarding {
        vec![
            "SGLang (local, recommended for Apertus)",
            "vLLM (local)",
            "Docker Model Runner (local)",
            "Ollama (local)",
            "OpenAI",
            "Anthropic",
            "Google AI",
            "Moonshot",
            "OpenRouter",
            "llama-cpp (local)",
        ]
    } else {
        vec![
            "Ollama (local)",
            "SGLang (local)",
            "vLLM (local)",
            "Docker Model Runner (local)",
            "OpenAI",
            "Anthropic",
            "Google AI",
            "Moonshot",
            "OpenRouter",
            "llama-cpp (local)",
        ]
    };
    let provider_idx = Select::new()
        .with_prompt("  Select LLM provider")
        .items(&providers)
        .default(if offer_apertus_onboarding { 0 } else { 4 })
        .interact()?;

    let (provider_prefix, needs_api_key) = match (offer_apertus_onboarding, provider_idx) {
        (true, 0) => ("sglang", false),
        (true, 1) => ("vllm", false),
        (true, 2) => ("docker-model-runner", false),
        (true, 3) => ("ollama", false),
        (true, 4) => ("openai", true),
        (true, 5) => ("anthropic", true),
        (true, 6) => ("google", true),
        (true, 7) => ("moonshot", true),
        (true, 8) => ("openrouter", true),
        (true, 9) => ("llama-cpp", false),
        (false, 0) => ("ollama", false),
        (false, 1) => ("sglang", false),
        (false, 2) => ("vllm", false),
        (false, 3) => ("docker-model-runner", false),
        (false, 4) => ("openai", true),
        (false, 5) => ("anthropic", true),
        (false, 6) => ("google", true),
        (false, 7) => ("moonshot", true),
        (false, 8) => ("openrouter", true),
        (false, 9) => ("llama-cpp", false),
        _ => ("openai", true),
    };

    // 3. API key
    let api_key = if needs_api_key {
        let key: String = Input::new()
            .with_prompt("  API key (or press Enter to set later)")
            .allow_empty(true)
            .interact_text()?;
        if key.is_empty() { None } else { Some(key) }
    } else {
        None
    };

    // 4. Model selection
    let ram_gb = detect_system_ram_gb();
    let model = match provider_prefix {
        "sglang" | "vllm" | "docker-model-runner" | "ollama" => {
            if offer_apertus_onboarding {
                if let Some(ram) = ram_gb {
                    println!("  {DETAIL} Detected system RAM: {ram} GB");
                } else {
                    println!(
                        "  {WARN} Could not detect system RAM. Only 8B Apertus is recommended by default."
                    );
                }

                match provider_prefix {
                    "sglang" if which_binary("sglang").is_none() => {
                        println!("  {WARN} sglang binary not found.");
                        let install_now = Confirm::new()
                            .with_prompt("  Install SGLang now via pip? (recommended for Apertus)")
                            .default(true)
                            .interact()?;
                        if install_now {
                            let py_bin = which_binary("python3")
                                .or_else(|| which_binary("python"))
                                .unwrap_or_else(|| "python3".into());
                            let status = std::process::Command::new(py_bin)
                                .args(["-m", "pip", "install", "--user", "sglang[all]"])
                                .status();
                            if status.as_ref().map(|s| s.success()).unwrap_or(false) {
                                println!("  {OK} SGLang install completed.");
                            } else {
                                println!(
                                    "  {WARN} SGLang install failed. You can install it later and keep this model selection."
                                );
                            }
                        }
                    }
                    "vllm" if which_binary("vllm").is_none() => {
                        println!("  {WARN} vllm command not found.");
                        let install_now = Confirm::new()
                            .with_prompt("  Install vLLM now via pip?")
                            .default(false)
                            .interact()?;
                        if install_now {
                            let py_bin = which_binary("python3")
                                .or_else(|| which_binary("python"))
                                .unwrap_or_else(|| "python3".into());
                            let status = std::process::Command::new(py_bin)
                                .args(["-m", "pip", "install", "--user", "vllm"])
                                .status();
                            if status.as_ref().map(|s| s.success()).unwrap_or(false) {
                                println!("  {OK} vLLM install completed.");
                            } else {
                                println!(
                                    "  {WARN} vLLM install failed. You can install it later and keep this model selection."
                                );
                            }
                        }
                    }
                    "docker-model-runner" if which_binary("docker").is_none() => {
                        println!("  {WARN} Docker not found. Docker Model Runner requires Docker.");
                    }
                    "ollama" if which_binary("ollama").is_none() => {
                        println!(
                            "  {WARN} Ollama not found. Install from https://ollama.ai to run local models."
                        );
                    }
                    _ => {}
                }

                let model_options = aperture_options_for_provider(provider_prefix, ram_gb);
                let model_idx = Select::new()
                    .with_prompt("  Select Apertus model")
                    .items(&model_options)
                    .default(0)
                    .interact()?;
                model_options[model_idx].clone()
            } else {
                let default_model = match provider_prefix {
                    "ollama" => "ollama/qwen3:8b",
                    "sglang" => "sglang/default",
                    "vllm" => "vllm/default",
                    "docker-model-runner" => "docker-model-runner/default",
                    _ => "ollama/qwen3:8b",
                };
                Input::new()
                    .with_prompt("  Model")
                    .default(default_model.into())
                    .interact_text()?
            }
        }
        _ => {
            let default_model = match provider_prefix {
                "openai" => "openai/gpt-4o",
                "anthropic" => "anthropic/claude-sonnet-4-20250514",
                "google" => "google/gemini-3.1-pro-preview",
                "moonshot" => "moonshot/kimi-k2.5",
                "openrouter" => "openrouter/google/gemini-3.1-pro-preview",
                "llama-cpp" => "llama-cpp/default",
                _ => "sglang/apertus-8b-instruct:latest",
            };
            Input::new()
                .with_prompt("  Model")
                .default(default_model.into())
                .interact_text()?
        }
    };

    // 5. Server port
    let port: String = Input::new()
        .with_prompt("  Server port")
        .default("18789".into())
        .interact_text()?;
    let port_num: u16 = port.parse().unwrap_or(18789);

    // 6. Channels
    let enable_telegram = Confirm::new()
        .with_prompt("  Enable Telegram channel?")
        .default(false)
        .interact()?;

    let telegram_token = if enable_telegram {
        let token: String = Input::new()
            .with_prompt("  Telegram bot token")
            .interact_text()?;
        Some(token)
    } else {
        None
    };

    let enable_discord = Confirm::new()
        .with_prompt("  Enable Discord channel?")
        .default(false)
        .interact()?;

    let discord_token = if enable_discord {
        let token: String = Input::new()
            .with_prompt("  Discord bot token")
            .interact_text()?;
        Some(token)
    } else {
        None
    };

    // 7. Workspace directory
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let default_workspace = format!("{home}/.ironclad/workspace");
    let workspace: String = Input::new()
        .with_prompt("  Workspace directory")
        .default(default_workspace)
        .interact_text()?;

    // 8. Database path
    let default_db = format!("{home}/.ironclad/state.db");
    let db_path: String = Input::new()
        .with_prompt("  Database path")
        .default(default_db)
        .interact_text()?;

    // Generate config
    let mut config = String::new();
    config.push_str("# Ironclad Configuration (generated by onboard wizard)\n\n");
    config.push_str("[agent]\n");
    config.push_str(&format!("name = \"{agent_name}\"\n"));
    config.push_str(&format!(
        "id = \"{}\"\n",
        agent_name.to_lowercase().replace(' ', "-")
    ));
    config.push_str(&format!("workspace = \"{workspace}\"\n"));
    config.push_str("log_level = \"info\"\n\n");

    config.push_str("[server]\n");
    config.push_str(&format!("port = {port_num}\n"));
    config.push_str("bind = \"127.0.0.1\"\n\n");

    config.push_str("[database]\n");
    config.push_str(&format!("path = \"{db_path}\"\n\n"));

    config.push_str("[models]\n");
    config.push_str(&format!("primary = \"{model}\"\n"));
    config.push_str("fallbacks = []\n\n");

    config.push_str("[models.routing]\n");
    config.push_str("mode = \"rule\"\n");
    config.push_str("confidence_threshold = 0.9\n");
    config.push_str("local_first = true\n\n");

    config.push_str(
        "# Bundled provider defaults (sglang, vllm, docker-model-runner, ollama, openai, anthropic, google, openrouter)\n",
    );
    config.push_str("# are auto-merged. Override or add new providers here.\n");
    if api_key.is_some() {
        config.push_str(&format!(
            "# Set the API key via env: {}_API_KEY\n\n",
            provider_prefix.to_uppercase()
        ));
    } else {
        config.push('\n');
    }

    config.push_str("[memory]\n");
    config.push_str("working_budget_pct = 30.0\n");
    config.push_str("episodic_budget_pct = 25.0\n");
    config.push_str("semantic_budget_pct = 20.0\n");
    config.push_str("procedural_budget_pct = 15.0\n");
    config.push_str("relationship_budget_pct = 10.0\n\n");

    config.push_str("[treasury]\n");
    config.push_str("per_payment_cap = 100.0\n");
    config.push_str("hourly_transfer_limit = 500.0\n");
    config.push_str("daily_transfer_limit = 2000.0\n");
    config.push_str("minimum_reserve = 5.0\n");
    config.push_str("daily_inference_budget = 50.0\n\n");

    if let Some(ref token) = telegram_token {
        config.push_str("[channels.telegram]\n");
        config.push_str(&format!("token = \"{token}\"\n\n"));
    }

    if let Some(ref token) = discord_token {
        config.push_str("[channels.discord]\n");
        config.push_str(&format!("token = \"{token}\"\n\n"));
    }

    config.push_str("[skills]\n");
    config.push_str(&format!("skills_dir = \"{home}/.ironclad/skills\"\n\n"));

    config.push_str("[a2a]\n");
    config.push_str("enabled = true\n");

    // Write config
    let config_path = "ironclad.toml";
    let is_first_install = !std::path::Path::new(config_path).exists();
    if !is_first_install {
        let overwrite = Confirm::new()
            .with_prompt("  ironclad.toml already exists. Overwrite?")
            .default(false)
            .interact()?;
        if !overwrite {
            println!("\n  Aborted. Existing config preserved.\n");
            return Ok(());
        }
    }

    std::fs::write(config_path, &config)?;
    println!("\n  {OK} Configuration written to {config_path}");

    // Create workspace dir
    let ws_path = std::path::Path::new(&workspace);
    if !ws_path.exists() {
        std::fs::create_dir_all(ws_path)?;
        println!("  {OK} Created workspace: {workspace}");
    }

    // Create skills dir with starter skills
    let skills_path = format!("{home}/.ironclad/skills");
    let sp = std::path::Path::new(&skills_path);
    if !sp.exists() {
        std::fs::create_dir_all(sp)?;
    }
    let skills_written = write_starter_skills(sp)?;
    if skills_written > 0 {
        println!("  {ACTION} Created {skills_written} starter skills");
    } else {
        println!("  {OK} Skills directory ready");
    }

    // Personality setup
    println!("\n  {BOLD}Personality Setup{RESET}\n");
    let personality_options = vec![
        "Keep Roboticus (recommended default)",
        "Quick setup (5 questions)",
        "Full interview (guided conversation with your agent)",
    ];
    let personality_idx = Select::new()
        .with_prompt("  How would you like to configure your agent's personality?")
        .items(&personality_options)
        .default(0)
        .interact()?;

    match personality_idx {
        0 => {
            ironclad_core::personality::write_defaults(ws_path)?;
            println!("  {OK} Roboticus personality loaded (OS.toml + FIRMWARE.toml)");
        }
        1 => {
            run_quick_personality_setup(ws_path)?;
        }
        2 => {
            let basic_name: String = Input::new()
                .with_prompt("  Agent name")
                .default(agent_name.clone())
                .interact_text()?;
            let domains = vec!["general", "developer", "business", "creative", "research"];
            let domain_idx = Select::new()
                .with_prompt("  Primary domain")
                .items(&domains)
                .default(0)
                .interact()?;

            let starter_os = ironclad_core::personality::generate_os_toml(
                &basic_name,
                "balanced",
                "suggest",
                domains[domain_idx],
            );
            std::fs::write(ws_path.join("OS.toml"), &starter_os)?;
            ironclad_core::personality::write_defaults(ws_path).ok();

            println!();
            println!("  {OK} Starter personality written.");
            println!("  {DETAIL} Start your agent:  {BOLD}ironclad serve{RESET}");
            println!("  {DETAIL} Then send it:      {BOLD}/interview{RESET}");
            println!("  {DETAIL} The agent will walk you through a deep personality interview.");
        }
        _ => {}
    }

    // On first install, explicitly ask whether to run the interview flow.
    if is_first_install && personality_idx != 2 {
        let do_interview = Confirm::new()
            .with_prompt("  Run the guided personality interview now? (recommended)")
            .default(true)
            .interact()?;
        if do_interview {
            println!();
            println!("  {DETAIL} Start your agent:  {BOLD}ironclad serve{RESET}");
            println!("  {DETAIL} Then send it:      {BOLD}/interview{RESET}");
            println!("  {DETAIL} The agent will walk you through a deep personality interview.");
        }
    }

    println!();
    println!("  {OK} Setup complete! Run {BOLD}ironclad serve{RESET} to start.");
    println!();

    Ok(())
}
