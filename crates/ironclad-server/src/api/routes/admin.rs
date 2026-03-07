//! Config, stats, circuit breaker, wallet, plugins, browser, agents, workspace, A2A.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    http::header,
    response::IntoResponse,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config_runtime;
use ironclad_agent::policy::{PolicyContext, ToolCallRequest};
use ironclad_core::{
    InputAuthority, IroncladConfig, IroncladError, PolicyDecision, SurvivalTier,
    input_capability_scan,
};

use super::{
    AppState, JsonError, bad_request, internal_err, not_found, sanitize_html, validate_short,
};

// ── Key resolution helper ────────────────────────────────────

/// Where a provider's API key was found (or that none is needed/available).
pub(crate) enum KeySource {
    NotRequired,
    OAuth,
    Keystore(String),
    EnvVar(String),
    Missing,
}

impl KeySource {
    pub fn status_pair(&self) -> (&'static str, &'static str) {
        match self {
            Self::NotRequired => ("not_required", "local"),
            Self::OAuth => ("configured", "oauth"),
            Self::Keystore(_) => ("configured", "keystore"),
            Self::EnvVar(_) => ("configured", "env"),
            Self::Missing => ("missing", "none"),
        }
    }
}

/// Determine the source and value of a provider's API key using a priority
/// cascade:
///   1. Local provider → `NotRequired`
///   2. OAuth (auth_mode == "oauth") → `OAuth`
///   3. Explicit keystore ref (api_key_ref = "keystore:name")
///   4. Conventional keystore name ({provider_name}_api_key)
///   5. Non-empty environment variable (api_key_env)
///   6. `Missing`
fn resolve_key_source(
    provider_name: &str,
    is_local: bool,
    api_key_ref: Option<&str>,
    api_key_env: Option<&str>,
    auth_mode: Option<&str>,
    keystore: &ironclad_core::keystore::Keystore,
) -> KeySource {
    if is_local {
        return KeySource::NotRequired;
    }

    if auth_mode.is_some_and(|m| m == "oauth") {
        return KeySource::OAuth;
    }

    if let Some(ks_name) = api_key_ref.and_then(|r| r.strip_prefix("keystore:"))
        && let Some(val) = keystore.get(ks_name)
    {
        return KeySource::Keystore(val);
    }

    let conventional = format!("{provider_name}_api_key");
    if let Some(val) = keystore.get(&conventional)
        && !val.is_empty()
    {
        return KeySource::Keystore(val);
    }

    if let Some(env_name) = api_key_env
        && let Ok(val) = std::env::var(env_name)
        && !val.is_empty()
    {
        return KeySource::EnvVar(val);
    }

    KeySource::Missing
}

/// Resolve an API key for a provider. Returns `None` when no key is
/// configured (or when the provider is local and doesn't need one).
pub(crate) async fn resolve_provider_key(
    provider_name: &str,
    is_local: bool,
    auth_mode: &str,
    api_key_ref: Option<&str>,
    api_key_env: &str,
    oauth: &ironclad_llm::OAuthManager,
    keystore: &ironclad_core::keystore::Keystore,
) -> Option<String> {
    let source = resolve_key_source(
        provider_name,
        is_local,
        api_key_ref,
        Some(api_key_env),
        Some(auth_mode),
        keystore,
    );
    match source {
        KeySource::NotRequired | KeySource::Missing => None,
        KeySource::OAuth => oauth.resolve_token(provider_name).await
            .inspect_err(|e| tracing::warn!(error = %e, provider = provider_name, "OAuth token resolution failed"))
            .ok(),
        KeySource::Keystore(v) | KeySource::EnvVar(v) => Some(v),
    }
}

/// Check whether a key is present for a provider, returning (status, source).
pub(crate) fn check_key_status(
    provider_name: &str,
    is_local: bool,
    api_key_ref: Option<&str>,
    api_key_env: Option<&str>,
    auth_mode: Option<&str>,
    keystore: &ironclad_core::keystore::Keystore,
) -> (&'static str, &'static str) {
    resolve_key_source(
        provider_name,
        is_local,
        api_key_ref,
        api_key_env,
        auth_mode,
        keystore,
    )
    .status_pair()
}

// ── Approval management routes ───────────────────────────────

include!("admin/approvals.rs");

include!("admin/config_models.rs");
include!("admin/metrics.rs");
include!("admin/revenue.rs");
include!("admin/runtime_health.rs");
include!("admin/operator_tools.rs");
include!("admin/workspace_agents.rs");
include!("admin/advanced_ops.rs");
#[cfg(test)]
mod tests;
