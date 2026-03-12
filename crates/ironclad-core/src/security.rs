//! Claim-based RBAC authority resolution.
//!
//! Every message entry point calls into this module to derive the sender's
//! effective [`InputAuthority`] from all authentication layers.
//!
//! # Algorithm
//!
//! ```text
//! effective = min(max(positive_grants…), min(negative_ceilings…))
//! ```
//!
//! Positive grants **OR** across layers (any layer can grant authority).
//! Negative ceilings **AND** across layers (strictest restriction wins).

use crate::config::SecurityConfig;
use crate::types::{ClaimSource, InputAuthority, SecurityClaim};

/// Inputs describing what a channel adapter knows about the sender.
#[derive(Debug, Clone)]
pub struct ChannelContext<'a> {
    /// Sender's platform-specific ID (chat ID, phone number, etc.).
    pub sender_id: &'a str,
    /// Chat/group/guild ID, if distinct from sender (e.g., Telegram chat ID).
    pub chat_id: &'a str,
    /// Platform name (e.g., "telegram", "discord", "api").
    pub channel: &'a str,
    /// Whether the sender passed the adapter's allow-list check.
    pub sender_in_allowlist: bool,
    /// Whether the adapter's allow-list is non-empty (has entries).
    pub allowlist_configured: bool,
    /// Whether the threat scanner flagged this input as Caution-level.
    pub threat_is_caution: bool,
    /// The global `channels.trusted_sender_ids` list.
    pub trusted_sender_ids: &'a [String],
}

/// Resolve a [`SecurityClaim`] for a channel-originated message.
///
/// This is the single authority resolution path for Telegram, Discord,
/// WhatsApp, Signal, and Email messages.
pub fn resolve_channel_claim(ctx: &ChannelContext<'_>, sec: &SecurityConfig) -> SecurityClaim {
    let mut grants: Vec<InputAuthority> = Vec::new();
    let mut sources: Vec<ClaimSource> = Vec::new();
    let mut ceilings: Vec<InputAuthority> = Vec::new();

    // ── Positive grants ─────────────────────────────────────────────

    // Layer 1: Channel allow-list
    if ctx.allowlist_configured {
        // Non-empty allow-list — grant if sender passed it
        if ctx.sender_in_allowlist {
            grants.push(sec.allowlist_authority);
            sources.push(ClaimSource::ChannelAllowList);
        }
        // Sender NOT in a configured allow-list: no grant from this layer
    }
    // Empty allow-list: no grant (secure default)

    // Layer 2: trusted_sender_ids
    if !ctx.trusted_sender_ids.is_empty() {
        let is_trusted = ctx
            .trusted_sender_ids
            .iter()
            .any(|id| id == ctx.chat_id || id == ctx.sender_id);
        if is_trusted {
            grants.push(sec.trusted_authority);
            sources.push(ClaimSource::TrustedSenderId);
        }
    }

    // ── Negative ceilings ───────────────────────────────────────────

    if ctx.threat_is_caution {
        ceilings.push(sec.threat_caution_ceiling);
    }

    // ── Compose ─────────────────────────────────────────────────────

    compose_claim(grants, sources, ceilings, ctx.sender_id, ctx.channel)
}

/// Resolve a [`SecurityClaim`] for an HTTP API or WebSocket request.
///
/// API callers are authenticated by API key (currently implicit — the API
/// is only accessible on localhost). The threat scanner can still apply a
/// ceiling.
pub fn resolve_api_claim(
    threat_is_caution: bool,
    channel: &str,
    sec: &SecurityConfig,
) -> SecurityClaim {
    let grants = vec![sec.api_authority];
    let sources = vec![ClaimSource::ApiKey];
    let mut ceilings: Vec<InputAuthority> = Vec::new();

    if threat_is_caution {
        ceilings.push(sec.threat_caution_ceiling);
    }

    compose_claim(grants, sources, ceilings, "api", channel)
}

/// Resolve a [`SecurityClaim`] for an A2A (agent-to-agent) session.
///
/// A2A peers are authenticated via ECDH X25519 key exchange. They receive
/// `Peer` authority — never `Creator`. The threat scanner still applies.
pub fn resolve_a2a_claim(
    threat_is_caution: bool,
    sender_id: &str,
    sec: &SecurityConfig,
) -> SecurityClaim {
    let grants = vec![InputAuthority::Peer];
    let sources = vec![ClaimSource::A2aSession];
    let mut ceilings: Vec<InputAuthority> = Vec::new();

    if threat_is_caution {
        ceilings.push(sec.threat_caution_ceiling);
    }

    compose_claim(grants, sources, ceilings, sender_id, "a2a")
}

/// Compose grants and ceilings into a final [`SecurityClaim`].
///
/// ```text
/// effective = min(max(grants…), min(ceilings…))
/// ```
fn compose_claim(
    grants: Vec<InputAuthority>,
    mut sources: Vec<ClaimSource>,
    ceilings: Vec<InputAuthority>,
    sender_id: &str,
    channel: &str,
) -> SecurityClaim {
    // Best grant (OR — any layer can grant).
    // NOTE: The unwrap_or_else closure mutates `sources` to record Anonymous
    // when no grants were provided. This is safe because `sources` is owned
    // by this function and consumed into the returned SecurityClaim.
    let effective_grant = grants.iter().copied().max().unwrap_or_else(|| {
        sources.push(ClaimSource::Anonymous);
        InputAuthority::External
    });

    // Strictest ceiling (AND — all must allow)
    let effective_ceiling = ceilings
        .iter()
        .copied()
        .min()
        .unwrap_or(InputAuthority::Creator); // no restrictions

    let final_authority = effective_grant.min(effective_ceiling);
    let threat_downgraded = !ceilings.is_empty() && final_authority < effective_grant;

    SecurityClaim {
        authority: final_authority,
        sources,
        ceiling: effective_ceiling,
        threat_downgraded,
        sender_id: sender_id.to_string(),
        channel: channel.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_sec() -> SecurityConfig {
        SecurityConfig::default()
    }

    fn channel_ctx<'a>(
        sender_id: &'a str,
        chat_id: &'a str,
        channel: &'a str,
        sender_in_allowlist: bool,
        allowlist_configured: bool,
        threat_is_caution: bool,
        trusted: &'a [String],
    ) -> ChannelContext<'a> {
        ChannelContext {
            sender_id,
            chat_id,
            channel,
            sender_in_allowlist,
            allowlist_configured,
            threat_is_caution,
            trusted_sender_ids: trusted,
        }
    }

    // ── Grant composition ───────────────────────────────────────────

    #[test]
    fn no_grants_yields_external() {
        let sec = default_sec();
        let ctx = channel_ctx("u1", "c1", "telegram", false, true, false, &[]);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::External);
        assert!(claim.sources.contains(&ClaimSource::Anonymous));
    }

    #[test]
    fn allowlist_only_yields_peer() {
        let sec = default_sec();
        let ctx = channel_ctx("u1", "c1", "telegram", true, true, false, &[]);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::Peer);
        assert!(claim.sources.contains(&ClaimSource::ChannelAllowList));
    }

    #[test]
    fn trusted_only_yields_creator() {
        let sec = default_sec();
        let trusted = vec!["u1".to_string()];
        let ctx = channel_ctx("u1", "c1", "telegram", false, true, false, &trusted);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::Creator);
        assert!(claim.sources.contains(&ClaimSource::TrustedSenderId));
    }

    #[test]
    fn trusted_by_chat_id() {
        let sec = default_sec();
        let trusted = vec!["c1".to_string()];
        let ctx = channel_ctx("u1", "c1", "telegram", false, true, false, &trusted);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::Creator);
    }

    #[test]
    fn both_allowlist_and_trusted_yields_creator() {
        // OR: best grant wins
        let sec = default_sec();
        let trusted = vec!["u1".to_string()];
        let ctx = channel_ctx("u1", "c1", "telegram", true, true, false, &trusted);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::Creator);
        assert!(claim.sources.contains(&ClaimSource::ChannelAllowList));
        assert!(claim.sources.contains(&ClaimSource::TrustedSenderId));
    }

    // ── Ceiling composition ─────────────────────────────────────────

    #[test]
    fn threat_ceiling_downgrades_creator() {
        let sec = default_sec();
        let trusted = vec!["u1".to_string()];
        let ctx = channel_ctx("u1", "c1", "telegram", true, true, true, &trusted);
        let claim = resolve_channel_claim(&ctx, &sec);
        // Creator grant capped by External ceiling
        assert_eq!(claim.authority, InputAuthority::External);
        assert!(claim.threat_downgraded);
        assert_eq!(claim.ceiling, InputAuthority::External);
    }

    #[test]
    fn custom_threat_ceiling() {
        let mut sec = default_sec();
        sec.threat_caution_ceiling = InputAuthority::Peer;
        let trusted = vec!["u1".to_string()];
        let ctx = channel_ctx("u1", "c1", "telegram", true, true, true, &trusted);
        let claim = resolve_channel_claim(&ctx, &sec);
        // Creator grant capped by Peer ceiling
        assert_eq!(claim.authority, InputAuthority::Peer);
        assert!(claim.threat_downgraded);
    }

    #[test]
    fn no_threat_means_no_ceiling() {
        let sec = default_sec();
        let trusted = vec!["u1".to_string()];
        let ctx = channel_ctx("u1", "c1", "telegram", true, true, false, &trusted);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::Creator);
        assert!(!claim.threat_downgraded);
        assert_eq!(claim.ceiling, InputAuthority::Creator); // no restriction
    }

    // ── Empty allow-list behavior ───────────────────────────────────

    #[test]
    fn empty_allowlist_deny_on_empty_true_rejects() {
        let sec = default_sec(); // deny_on_empty_allowlist = true
        let ctx = channel_ctx("u1", "c1", "telegram", false, false, false, &[]);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::External);
        assert!(claim.sources.contains(&ClaimSource::Anonymous));
    }

    #[test]
    fn empty_allowlist_still_rejects_even_if_flag_is_false() {
        let mut sec = default_sec();
        // Runtime no longer supports permissive empty allow-lists. Repair/update
        // migrates this value back to true before persisted configs are reloaded.
        sec.deny_on_empty_allowlist = false;
        let ctx = channel_ctx("u1", "c1", "telegram", false, false, false, &[]);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::External);
        assert!(claim.sources.contains(&ClaimSource::Anonymous));
    }

    // ── API claims ──────────────────────────────────────────────────

    #[test]
    fn api_claim_default_creator() {
        let sec = default_sec();
        let claim = resolve_api_claim(false, "api", &sec);
        assert_eq!(claim.authority, InputAuthority::Creator);
        assert!(claim.sources.contains(&ClaimSource::ApiKey));
    }

    #[test]
    fn api_claim_threat_downgrade() {
        let sec = default_sec();
        let claim = resolve_api_claim(true, "api", &sec);
        assert_eq!(claim.authority, InputAuthority::External);
        assert!(claim.threat_downgraded);
    }

    // ── A2A claims ──────────────────────────────────────────────────

    #[test]
    fn a2a_claim_always_peer() {
        let sec = default_sec();
        let claim = resolve_a2a_claim(false, "peer-agent", &sec);
        assert_eq!(claim.authority, InputAuthority::Peer);
        assert!(claim.sources.contains(&ClaimSource::A2aSession));
    }

    #[test]
    fn a2a_claim_threat_downgrade() {
        let sec = default_sec();
        let claim = resolve_a2a_claim(true, "peer-agent", &sec);
        assert_eq!(claim.authority, InputAuthority::External);
        assert!(claim.threat_downgraded);
    }

    // ── Configurable authority levels ───────────────────────────────

    #[test]
    fn custom_allowlist_authority() {
        let mut sec = default_sec();
        sec.allowlist_authority = InputAuthority::Creator;
        let ctx = channel_ctx("u1", "c1", "telegram", true, true, false, &[]);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::Creator);
    }

    #[test]
    fn custom_api_authority_downgraded() {
        let mut sec = default_sec();
        sec.api_authority = InputAuthority::Peer;
        let claim = resolve_api_claim(false, "api", &sec);
        assert_eq!(claim.authority, InputAuthority::Peer);
    }

    // ── Monotonicity properties ─────────────────────────────────────

    #[test]
    fn adding_grant_never_decreases_authority() {
        let sec = default_sec();
        // Without trusted
        let ctx1 = channel_ctx("u1", "c1", "telegram", true, true, false, &[]);
        let claim1 = resolve_channel_claim(&ctx1, &sec);

        // With trusted (additional grant)
        let trusted = vec!["u1".to_string()];
        let ctx2 = channel_ctx("u1", "c1", "telegram", true, true, false, &trusted);
        let claim2 = resolve_channel_claim(&ctx2, &sec);

        assert!(claim2.authority >= claim1.authority);
    }

    #[test]
    fn adding_ceiling_never_increases_authority() {
        let sec = default_sec();
        let trusted = vec!["u1".to_string()];

        // Without threat
        let ctx1 = channel_ctx("u1", "c1", "telegram", true, true, false, &trusted);
        let claim1 = resolve_channel_claim(&ctx1, &sec);

        // With threat (additional ceiling)
        let ctx2 = channel_ctx("u1", "c1", "telegram", true, true, true, &trusted);
        let claim2 = resolve_channel_claim(&ctx2, &sec);

        assert!(claim2.authority <= claim1.authority);
    }

    // ── Edge cases: threat_downgraded correctness ───────────────────

    #[test]
    fn threat_present_but_not_binding_does_not_set_downgraded() {
        // External user with no grants + threat_is_caution = true.
        // Ceiling is External, grant is External → ceiling is not binding.
        let sec = default_sec();
        let ctx = channel_ctx("unknown", "c1", "telegram", false, true, true, &[]);
        let claim = resolve_channel_claim(&ctx, &sec);
        assert_eq!(claim.authority, InputAuthority::External);
        // Ceiling exists but didn't actually reduce authority
        assert!(!claim.threat_downgraded);
    }

    #[test]
    fn api_claim_with_custom_ceiling_and_threat() {
        // API with Peer authority + Peer ceiling → no downgrade (ceiling = grant)
        let mut sec = default_sec();
        sec.api_authority = InputAuthority::Peer;
        sec.threat_caution_ceiling = InputAuthority::Peer;
        let claim = resolve_api_claim(true, "api", &sec);
        assert_eq!(claim.authority, InputAuthority::Peer);
        assert!(!claim.threat_downgraded); // ceiling = grant, not binding

        // API with Creator authority + Peer ceiling → downgrade
        let mut sec2 = default_sec();
        sec2.threat_caution_ceiling = InputAuthority::Peer;
        let claim2 = resolve_api_claim(true, "api", &sec2);
        assert_eq!(claim2.authority, InputAuthority::Peer);
        assert!(claim2.threat_downgraded); // ceiling < grant, binding
    }
}
