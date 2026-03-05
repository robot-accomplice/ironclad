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
    if lower.contains("delegate") || lower.contains("orchestrate") || lower.contains("assign") {
        return true;
    }
    lower.contains("subagent")
        && (lower.contains("order")
            || lower.contains("task")
            || lower.contains("run ")
            || lower.contains("to a subagent")
            || lower.contains("to the subagent"))
}

pub(super) fn requests_cron(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("cron") || (lower.contains("schedule") && lower.contains("minute"))
}

pub(super) fn requests_file_distribution(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("file distribution")
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
        "geopolitical sitrep",
        "sitrep",
        "current events",
        "latest news",
        "what's happening",
        "what is happening",
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
}
