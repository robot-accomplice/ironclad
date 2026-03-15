/// Check if a target identifier is allowed by the given allowlist.
/// If the allowlist is empty, returns `!deny_on_empty` (i.e., denies if deny_on_empty is true).
pub fn check_allowlist(allowed: &[String], target: &str, deny_on_empty: bool) -> bool {
    if allowed.is_empty() {
        return !deny_on_empty;
    }
    allowed.iter().any(|a| a == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_with_deny_rejects() {
        assert!(!check_allowlist(&[], "any-sender", true));
    }

    #[test]
    fn empty_allowlist_without_deny_accepts() {
        assert!(check_allowlist(&[], "any-sender", false));
    }

    #[test]
    fn allowlist_match_accepts() {
        assert!(check_allowlist(&["alice".into()], "alice", true));
    }

    #[test]
    fn allowlist_no_match_rejects() {
        assert!(!check_allowlist(&["alice".into()], "bob", true));
    }
}
