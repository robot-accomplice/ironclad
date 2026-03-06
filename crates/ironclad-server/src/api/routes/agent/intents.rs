pub(super) fn requests_execution(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        " run ",
        " execute ",
        " use a tool",
        "use the tool",
        "tools you can use",
        "pick one at random",
        "introspection tool",
        "introspection skill",
        "introspect",
        "list entries",
        "list files",
        "file distribution",
        "schedule a cron",
        "schedule cron",
        "create cron",
        "order a subagent",
        "delegate",
        "orchestrate",
        "ls ",
        "/status",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

pub(super) fn requests_delegation(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("delegate") || lower.contains("orchestrate") {
        return true;
    }
    if lower.contains("assign") && (lower.contains("subagent") || lower.contains("sub agent")) {
        return true;
    }
    (lower.contains("subagent") || lower.contains("sub agent"))
        && (lower.contains("order")
            || lower.contains("ask")
            || lower.contains("task")
            || lower.contains("run ")
            || lower.contains("to a subagent")
            || lower.contains("to a sub agent")
            || lower.contains("to the subagent")
            || lower.contains("to the sub agent"))
}

pub(super) fn requests_cron(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("cron") || (lower.contains("schedule") && lower.contains("minute"))
}

pub(super) fn requests_file_distribution(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("file distribution")
}

pub(super) fn requests_folder_scan(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let asks_scan = lower.contains("look in")
        || lower.contains("check my")
        || lower.contains("search")
        || lower.contains("scan")
        || lower.contains("inspect");
    let mentions_folder = lower.contains("folder")
        || lower.contains("directory")
        || lower.contains("~/downloads")
        || lower.contains("~/documents")
        || lower.contains("~/pictures")
        || lower.contains("~/photos")
        || lower.contains("~/desktop")
        || lower.contains("~/")
        || lower.contains("/users/");
    asks_scan && mentions_folder
}

pub(super) fn requests_random_tool_use(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("tools you can use")
        || (lower.contains("pick one at random") && lower.contains("tool"))
}

pub(super) fn requests_model_identity(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("current model")
        || lower.contains("what model")
        || lower.contains("which model")
        || lower.contains("still on")
        || lower.contains("still using")
        || lower.contains("using moonshot")
        || lower.contains("confirm for me")
        || lower.contains("/status")
}

pub(super) fn requests_current_events(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        "geopolitical situation",
        "geopolitical",
        "geo political",
        "geopolitical sitrep",
        "sitrep",
        "current events",
        "latest news",
        "what's happening",
        "what is happening",
        "goings on",
        "going on in the",
        "what does the",
        "today's",
        "as of today",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

pub(super) fn requests_introspection(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        "introspection tool",
        "introspection skill",
        "introspect",
        "what tools can you use",
        "what tools do you have",
        "available tools",
        "subagent functionality",
        "current subagent functionality",
        "summarize the results",
        "summarize introspection",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

pub(super) fn requests_acknowledgement(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    (lower.contains("acknowledge") || lower.contains("acknowledg"))
        && (lower.contains("one sentence") || lower.contains("then wait"))
}

pub(super) fn requests_provider_inventory(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("which llm providers")
        || lower.contains("what llm providers")
        || lower.contains("which providers")
        || lower.contains("what providers")
}

pub(super) fn requests_personality_profile(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    (lower.contains("personality") && (lower.contains("your") || lower.contains("you")))
        || lower.contains("who are you")
}

pub(super) fn requests_capability_summary(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("what are you able to do")
        || lower.contains("what can you do")
        || lower.contains("what are you able")
        || lower.contains("what can you help")
}

pub(super) fn requests_wallet_address_scan(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let has_wallet_target = lower.contains("wallet address")
        || lower.contains("wallet addresses")
        || lower.contains("wallet credential")
        || lower.contains("wallet credentials")
        || lower.contains("private key")
        || lower.contains("seed phrase")
        || lower.contains("mnemonic")
        || lower.contains("keystore")
        || lower.contains("xprv")
        || lower.contains("xpub");
    has_wallet_target
        && (lower.contains("search")
            || lower.contains("find")
            || lower.contains("scan")
            || lower.contains("recursively")
            || lower.contains("look in")
            || lower.contains("check")
            || lower.contains("see if there are"))
}

pub(super) fn requests_image_count_scan(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let asks_for_count = lower.contains("how many")
        || lower.contains("count")
        || lower.contains("number of")
        || lower.contains("total");
    let mentions_images = lower.contains("image files")
        || lower.contains("images")
        || lower.contains("photos")
        || lower.contains("pictures");
    asks_for_count && mentions_images
}

pub(super) fn requests_obsidian_insights(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let mentions_vault = lower.contains("obsidian") || lower.contains("vault");
    let asks_for_summary = lower.contains("insight")
        || lower.contains("summary")
        || lower.contains("summarize")
        || lower.contains("what")
        || lower.contains("say about")
        || lower.contains("status");
    mentions_vault && asks_for_summary
}

pub(super) fn requests_email_triage(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let email_markers = [
        "check my email",
        "check email",
        "inbox",
        "mailbox",
        "important email",
        "important emails",
        "scan my email",
        "email triage",
        "email digest",
    ];
    let bridge_markers = ["proton bridge", "protonbridge", "himalaya"];
    email_markers.iter().any(|m| lower.contains(m))
        || (lower.contains("email") && bridge_markers.iter().any(|m| lower.contains(m)))
}

pub(super) fn requests_literary_quote_context(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("what's that from") || lower.contains("what is that from") {
        return true;
    }
    let asks_for_quote = lower.contains("quote")
        || lower.contains("line from")
        || lower.contains("appropriate line")
        || lower.contains("what quote");
    let literary_source = lower.contains("dune")
        || lower.contains("frank herbert")
        || lower.contains("litany against fear");
    let contextual_target = lower.contains("conflict")
        || lower.contains("iran")
        || lower.contains("geopolitical")
        || lower.contains("situation");
    asks_for_quote && (literary_source || contextual_target)
}

pub(super) fn should_bypass_cache_for_prompt(prompt: &str) -> bool {
    requests_execution(prompt)
        || requests_current_events(prompt)
        || requests_introspection(prompt)
        || requests_provider_inventory(prompt)
        || requests_personality_profile(prompt)
        || requests_capability_summary(prompt)
        || requests_acknowledgement(prompt)
        || requests_wallet_address_scan(prompt)
        || requests_image_count_scan(prompt)
        || requests_obsidian_insights(prompt)
        || requests_email_triage(prompt)
        || requests_literary_quote_context(prompt)
        || requests_folder_scan(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_markers_cover_shortcut_and_guard_triggers() {
        assert!(requests_execution(
            "tell me about the tools you can use, pick one at random, and use it"
        ));
        assert!(requests_execution("/status"));
        assert!(requests_execution("please execute ls /tmp"));
    }

    #[test]
    fn delegation_and_cron_markers_match_expected_prompts() {
        assert!(requests_delegation("order a subagent to do this"));
        assert!(requests_delegation("ask the sub agent to do this"));
        assert!(!requests_delegation(
            "use your introspection tool to discover current subagent functionality"
        ));
        assert!(requests_cron("schedule a cron job every 5 minute"));
    }

    #[test]
    fn model_identity_markers_match_expected_prompts() {
        assert!(requests_model_identity(
            "can you confirm for me that you are still using moonshot?"
        ));
        assert!(requests_model_identity("/status"));
    }

    #[test]
    fn current_events_markers_match_expected_prompts() {
        assert!(requests_current_events(
            "What's the geopolitical situation?"
        ));
        assert!(requests_current_events("Give me a geopolitical sitrep"));
        assert!(requests_current_events(
            "What does the geo political sub agent say about goings on in the US?"
        ));
        assert!(requests_current_events("What are today's current events?"));
    }

    #[test]
    fn introspection_markers_match_expected_prompts() {
        assert!(requests_introspection(
            "I want you to use your introspection skill"
        ));
        assert!(requests_introspection(
            "use your introspection tool to discover current subagent functionality"
        ));
        assert!(requests_introspection("what tools do you have available?"));
    }

    #[test]
    fn email_triage_markers_match_expected_prompts() {
        assert!(requests_email_triage(
            "Can you have a subagent check my email for anything important?"
        ));
        assert!(requests_email_triage(
            "Use proton bridge and triage inbox for urgent items"
        ));
        assert!(!requests_email_triage("Check my calendar for tomorrow"));
    }

    #[test]
    fn literary_quote_markers_match_expected_prompts() {
        assert!(requests_literary_quote_context(
            "Give me an appropriate dune quote for the conflict in Iran"
        ));
        assert!(requests_literary_quote_context("What's that from?"));
        assert!(!requests_literary_quote_context(
            "Give me a geopolitical situation update"
        ));
    }

    #[test]
    fn acknowledgement_markers_match_expected_prompts() {
        assert!(requests_acknowledgement(
            "Good evening Duncan. Acknowledge this request in one sentence, then wait."
        ));
        assert!(requests_acknowledgement(
            "acknowledge this in one sentence and then wait for my next command"
        ));
        assert!(!requests_acknowledgement("please acknowledge receipt"));
    }

    #[test]
    fn provider_inventory_markers_match_expected_prompts() {
        assert!(requests_provider_inventory("which llm providers?"));
        assert!(requests_provider_inventory("what providers are configured"));
        assert!(!requests_provider_inventory("what model are you using"));
    }

    #[test]
    fn personality_and_capability_markers_match_expected_prompts() {
        assert!(requests_personality_profile(
            "Tell me about your personality"
        ));
        assert!(requests_personality_profile("who are you"));
        assert!(requests_capability_summary(
            "Duncan, what are you able to do for me right now?"
        ));
    }

    #[test]
    fn wallet_scan_markers_match_expected_prompts() {
        assert!(requests_wallet_address_scan(
            "search the ~/code folder recursively and tell me files containing wallet address"
        ));
        assert!(requests_wallet_address_scan(
            "find wallet addresses in /tmp recursively"
        ));
        assert!(requests_wallet_address_scan(
            "I want you to check my ~/Downloads folder to see if there are any wallet credentials there"
        ));
        assert!(requests_wallet_address_scan(
            "Now look in my Downloads folder for wallet credentials"
        ));
        assert!(requests_wallet_address_scan(
            "Please check my Desktop folder for files containing private key and list full paths."
        ));
        assert!(!requests_wallet_address_scan("show me your wallet balance"));
    }

    #[test]
    fn image_count_markers_match_expected_prompts() {
        assert!(requests_image_count_scan(
            "How many image files are in my photos?"
        ));
        assert!(requests_image_count_scan(
            "count images in ~/Downloads recursively"
        ));
        assert!(!requests_image_count_scan("show me photos from yesterday"));
    }

    #[test]
    fn folder_scan_markers_match_expected_prompts() {
        assert!(requests_folder_scan(
            "Now look in my Downloads folder and summarize what is there"
        ));
        assert!(requests_folder_scan(
            "please check my ~/Documents folder for wallet credentials"
        ));
        assert!(!requests_folder_scan("what's your personality?"));
    }

    #[test]
    fn obsidian_insight_markers_match_expected_prompts() {
        assert!(requests_obsidian_insights(
            "Any insights you care to draw from the obsidian vault?"
        ));
        assert!(requests_obsidian_insights("summarize my vault"));
        assert!(!requests_obsidian_insights("vault token price"));
    }

    #[test]
    fn cache_bypass_markers_cover_shortcut_handled_prompts() {
        assert!(should_bypass_cache_for_prompt(
            "tell me about the tools you can use, pick one at random, and use it"
        ));
        assert!(should_bypass_cache_for_prompt(
            "Good evening Duncan. Acknowledge this request in one sentence, then wait."
        ));
        assert!(should_bypass_cache_for_prompt(
            "What does the geopolitical monitor have to say about today's news?"
        ));
        assert!(!should_bypass_cache_for_prompt(
            "Summarize this paragraph in one sentence."
        ));
    }
}
