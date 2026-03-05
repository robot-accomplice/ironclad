pub(super) fn requests_execution(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        " run ",
        " execute ",
        " use a tool",
        "use the tool",
        "tools you can use",
        "pick one at random",
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
    lower.contains("subagent") || lower.contains("delegate") || lower.contains("orchestrate")
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
        assert!(requests_cron("schedule a cron job every 5 minute"));
    }

    #[test]
    fn model_identity_markers_match_expected_prompts() {
        assert!(requests_model_identity(
            "can you confirm for me that you are still using moonshot?"
        ));
        assert!(requests_model_identity("/status"));
    }
}
