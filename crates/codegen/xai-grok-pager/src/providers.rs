//! Provider catalog state for the `/providers` picker.
//!
//! Mirrors the secret-free wire types served by the shell's
//! `x.ai/providers/*` extension methods. API keys entered in the picker
//! travel only inside [`SecretKey`], whose `Debug` output is redacted so
//! key text can never reach logs, toasts, or scrollback via formatting.

use serde::Deserialize;

/// Availability of one provider, matching the shell's
/// `ProviderAvailability` snake_case wire encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Usable: unauthenticated, or keyed via a stored credential.
    Configured,
    /// Usable via an environment-variable key.
    Environment,
    /// Supported but no credential is available.
    MissingKey,
    /// No supported models; cannot be used by this client.
    Unavailable,
}

impl ProviderStatus {
    /// Human-readable status label shown in the picker row.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "Configured",
            Self::Environment => "Environment",
            Self::MissingKey => "Missing key",
            Self::Unavailable => "Unavailable",
        }
    }

    /// Whether a stored key exists that `/providers` can clear.
    pub fn has_stored_key(self) -> bool {
        matches!(self, Self::Configured)
    }
}

/// One provider row from `x.ai/providers/list` (secret-free).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub provider_id: String,
    pub provider_name: String,
    pub status: ProviderStatus,
}

/// Response for `x.ai/providers/list` and payload of the
/// `x.ai/providers/update` broadcast (which omits the refresh fields).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderInfo>,
    /// Catalog freshness: `fresh`, `stale`, `refreshing`, `cachedAfterError`.
    #[serde(default)]
    pub refresh_status: String,
    /// Whether the list call started a background catalog refresh.
    #[serde(default)]
    pub refresh_started: bool,
}

impl Default for ProviderStatus {
    fn default() -> Self {
        Self::Unavailable
    }
}

/// Row view-model for the providers modal. Unavailable rows are disabled
/// (unsupported protocol or authentication) and cannot be selected.
#[derive(Clone, Debug)]
pub struct ProviderRowView {
    pub provider_id: String,
    pub provider_name: String,
    pub status: ProviderStatus,
    /// Disabled rows render dimmed and ignore selection.
    pub disabled: bool,
}

/// Map a provider list response into picker rows.
pub fn provider_rows(list: &ProviderListResponse) -> Vec<ProviderRowView> {
    list.providers
        .iter()
        .map(|p| ProviderRowView {
            provider_id: p.provider_id.clone(),
            provider_name: p.provider_name.clone(),
            status: p.status,
            disabled: p.status == ProviderStatus::Unavailable,
        })
        .collect()
}

/// An API key in transit from the masked input to effect execution.
///
/// `Debug` is implemented manually and never prints the key, so actions
/// and effects carrying it can keep their derived `Debug` without leaking.
#[derive(Clone)]
pub struct SecretKey(String);

impl SecretKey {
    pub fn new(key: String) -> Self {
        Self(key)
    }

    /// Expose the key text. Call only at effect execution when building
    /// the `x.ai/providers/store_key` request payload.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(\u{ab}redacted\u{bb})")
    }
}
