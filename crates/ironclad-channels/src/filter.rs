/// Composable filter chain for addressability detection.
///
/// In group chats the agent should only respond when explicitly addressed
/// (mentioned by name, replied-to, etc.). These filters compose with OR
/// semantics so any single match is sufficient.
///
/// Determines whether the agent should process a given message.
pub trait MessageFilter: Send + Sync {
    fn name(&self) -> &str;
    fn accept(&self, msg: &super::InboundMessage) -> bool;
}

/// Composes multiple filters with OR logic — accepts if ANY filter matches.
pub struct FilterChain {
    filters: Vec<Box<dyn MessageFilter>>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    pub fn add(&mut self, filter: Box<dyn MessageFilter>) {
        self.filters.push(filter);
    }

    /// Returns `true` if ANY filter accepts the message, or if the chain is
    /// empty (pass-through — no filters means no restrictions).
    pub fn accepts(&self, msg: &super::InboundMessage) -> bool {
        if self.filters.is_empty() {
            return true;
        }
        self.filters.iter().any(|f| f.accept(msg))
    }

    pub fn filters(&self) -> Vec<&str> {
        self.filters.iter().map(|f| f.name()).collect()
    }
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Concrete filters
// ---------------------------------------------------------------------------

/// Accepts if the message content contains the agent's name (case-insensitive).
pub struct MentionFilter {
    agent_name_lower: String,
}

impl MentionFilter {
    pub fn new(agent_name: String) -> Self {
        Self {
            agent_name_lower: agent_name.to_lowercase(),
        }
    }
}

impl MessageFilter for MentionFilter {
    fn name(&self) -> &str {
        "MentionFilter"
    }

    fn accept(&self, msg: &super::InboundMessage) -> bool {
        msg.content.to_lowercase().contains(&self.agent_name_lower)
    }
}

/// Accepts if the message metadata contains `"reply_to_bot": true`.
pub struct ReplyFilter;

impl MessageFilter for ReplyFilter {
    fn name(&self) -> &str {
        "ReplyFilter"
    }

    fn accept(&self, msg: &super::InboundMessage) -> bool {
        msg.metadata
            .as_ref()
            .and_then(|m| m.get("reply_to_bot"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

/// Accepts direct messages. A message is considered a DM if metadata is `None`
/// or `metadata["is_group"]` is `false`.
pub struct ConversationFilter;

impl MessageFilter for ConversationFilter {
    fn name(&self) -> &str {
        "ConversationFilter"
    }

    fn accept(&self, msg: &super::InboundMessage) -> bool {
        match &msg.metadata {
            None => true,
            Some(meta) => match meta.get("is_group") {
                None => true,
                Some(v) => !v.as_bool().unwrap_or(false),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Factory & helpers
// ---------------------------------------------------------------------------

/// Creates a [`FilterChain`] pre-loaded with the standard addressability
/// filters: mention, reply, and conversation (DM).
pub fn default_addressability_chain(agent_name: &str) -> FilterChain {
    let mut chain = FilterChain::new();
    chain.add(Box::new(MentionFilter::new(agent_name.to_owned())));
    chain.add(Box::new(ReplyFilter));
    chain.add(Box::new(ConversationFilter));
    chain
}

/// Returns `true` if `metadata["is_group"]` is `true`.
pub fn is_group_message(msg: &super::InboundMessage) -> bool {
    msg.metadata
        .as_ref()
        .and_then(|m| m.get("is_group"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn make_msg(content: &str, metadata: Option<serde_json::Value>) -> crate::InboundMessage {
        crate::InboundMessage {
            id: "test-1".into(),
            platform: "test".into(),
            sender_id: "user-1".into(),
            content: content.into(),
            timestamp: Utc::now(),
            metadata,
        }
    }

    // -- MentionFilter -------------------------------------------------------

    #[test]
    fn mention_filter_matches_name() {
        let f = MentionFilter::new("ironclad".into());
        let msg = make_msg("hey ironclad, what's up?", None);
        assert!(f.accept(&msg));
    }

    #[test]
    fn mention_filter_case_insensitive() {
        let f = MentionFilter::new("ironclad".into());
        let msg = make_msg("IRONCLAD do something", None);
        assert!(f.accept(&msg));
    }

    #[test]
    fn mention_filter_rejects_no_mention() {
        let f = MentionFilter::new("ironclad".into());
        let msg = make_msg("hello world", None);
        assert!(!f.accept(&msg));
    }

    // -- ReplyFilter ---------------------------------------------------------

    #[test]
    fn reply_filter_matches_reply() {
        let f = ReplyFilter;
        let msg = make_msg("thanks", Some(json!({"reply_to_bot": true})));
        assert!(f.accept(&msg));
    }

    #[test]
    fn reply_filter_rejects_non_reply() {
        let f = ReplyFilter;
        let msg = make_msg("random", Some(json!({"reply_to_bot": false})));
        assert!(!f.accept(&msg));
    }

    // -- ConversationFilter --------------------------------------------------

    #[test]
    fn conversation_filter_accepts_dm() {
        let f = ConversationFilter;
        let msg = make_msg("hi", Some(json!({"is_group": false})));
        assert!(f.accept(&msg));
    }

    #[test]
    fn conversation_filter_accepts_no_metadata() {
        let f = ConversationFilter;
        let msg = make_msg("hi", None);
        assert!(f.accept(&msg));
    }

    #[test]
    fn conversation_filter_rejects_group() {
        let f = ConversationFilter;
        let msg = make_msg("hi", Some(json!({"is_group": true})));
        assert!(!f.accept(&msg));
    }

    // -- FilterChain ---------------------------------------------------------

    #[test]
    fn filter_chain_empty_passes_all() {
        let chain = FilterChain::new();
        let msg = make_msg("anything", Some(json!({"is_group": true})));
        assert!(chain.accepts(&msg));
    }

    #[test]
    fn filter_chain_or_logic() {
        let mut chain = FilterChain::new();
        chain.add(Box::new(MentionFilter::new("ironclad".into())));
        chain.add(Box::new(ReplyFilter));

        // Mention alone is enough
        let msg = make_msg("hey ironclad", Some(json!({"is_group": true})));
        assert!(chain.accepts(&msg));

        // Reply alone is enough
        let msg = make_msg("ok", Some(json!({"reply_to_bot": true, "is_group": true})));
        assert!(chain.accepts(&msg));
    }

    #[test]
    fn filter_chain_rejects_when_none_match() {
        let mut chain = FilterChain::new();
        chain.add(Box::new(MentionFilter::new("ironclad".into())));
        chain.add(Box::new(ReplyFilter));
        chain.add(Box::new(ConversationFilter));

        // Group message, no mention, no reply
        let msg = make_msg("hello everyone", Some(json!({"is_group": true})));
        assert!(!chain.accepts(&msg));
    }

    // -- default_addressability_chain ----------------------------------------

    #[test]
    fn default_chain_accepts_mention() {
        let chain = default_addressability_chain("ironclad");
        let msg = make_msg("ironclad help me", Some(json!({"is_group": true})));
        assert!(chain.accepts(&msg));
    }

    // -- is_group_message helper ---------------------------------------------

    #[test]
    fn is_group_message_true() {
        let msg = make_msg("hi", Some(json!({"is_group": true})));
        assert!(is_group_message(&msg));
    }

    #[test]
    fn is_group_message_false() {
        let msg = make_msg("hi", Some(json!({"is_group": false})));
        assert!(!is_group_message(&msg));
    }

    #[test]
    fn is_group_message_no_metadata() {
        let msg = make_msg("hi", None);
        assert!(!is_group_message(&msg));
    }

    #[test]
    fn is_group_message_no_is_group_key() {
        let msg = make_msg("hi", Some(json!({"other": "value"})));
        assert!(!is_group_message(&msg));
    }

    #[test]
    fn filter_chain_filters_accessor() {
        let mut chain = FilterChain::new();
        chain.add(Box::new(MentionFilter::new("bot".into())));
        chain.add(Box::new(ReplyFilter));
        let names = chain.filters();
        assert_eq!(names, vec!["MentionFilter", "ReplyFilter"]);
    }

    #[test]
    fn filter_chain_default_is_empty() {
        let chain = FilterChain::default();
        assert!(chain.filters().is_empty());
    }

    #[test]
    fn mention_filter_name() {
        let f = MentionFilter::new("bot".into());
        assert_eq!(f.name(), "MentionFilter");
    }

    #[test]
    fn reply_filter_name() {
        let f = ReplyFilter;
        assert_eq!(f.name(), "ReplyFilter");
    }

    #[test]
    fn conversation_filter_name() {
        let f = ConversationFilter;
        assert_eq!(f.name(), "ConversationFilter");
    }

    #[test]
    fn reply_filter_no_metadata() {
        let f = ReplyFilter;
        let msg = make_msg("hello", None);
        assert!(!f.accept(&msg));
    }

    #[test]
    fn reply_filter_no_reply_to_bot_key() {
        let f = ReplyFilter;
        let msg = make_msg("hello", Some(json!({"other": "value"})));
        assert!(!f.accept(&msg));
    }

    #[test]
    fn conversation_filter_no_is_group_key() {
        let f = ConversationFilter;
        let msg = make_msg("hi", Some(json!({"other": "value"})));
        assert!(f.accept(&msg));
    }

    #[test]
    fn default_chain_has_three_filters() {
        let chain = default_addressability_chain("bot");
        assert_eq!(chain.filters().len(), 3);
        assert!(chain.filters().contains(&"MentionFilter"));
        assert!(chain.filters().contains(&"ReplyFilter"));
        assert!(chain.filters().contains(&"ConversationFilter"));
    }

    #[test]
    fn default_chain_accepts_dm() {
        let chain = default_addressability_chain("bot");
        let msg = make_msg("hello", Some(json!({"is_group": false})));
        assert!(chain.accepts(&msg));
    }

    #[test]
    fn default_chain_accepts_reply() {
        let chain = default_addressability_chain("bot");
        let msg = make_msg("thanks", Some(json!({"reply_to_bot": true, "is_group": true})));
        assert!(chain.accepts(&msg));
    }

    #[test]
    fn default_chain_rejects_group_no_mention_no_reply() {
        let chain = default_addressability_chain("bot");
        let msg = make_msg("hello everyone", Some(json!({"is_group": true})));
        assert!(!chain.accepts(&msg));
    }

    #[test]
    fn mention_filter_partial_match() {
        let f = MentionFilter::new("ironclad".into());
        let msg = make_msg("hey @ironclad-bot do something", None);
        assert!(f.accept(&msg));
    }
}
