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

/// Map a catalog `refresh_status` wire string to the notice shown in the
/// `/model` picker. `fresh` (and unknown values) show nothing; `stale` and
/// `refreshing` announce the in-flight refresh; `cachedAfterError` states
/// that cached rows are in use. Never closes the picker or moves selection —
/// callers only swap this text.
pub fn catalog_notice_for_status(refresh_status: &str) -> Option<String> {
    match refresh_status {
        "stale" | "refreshing" => Some("Refreshing provider catalog".to_string()),
        "cachedAfterError" => Some("Using cached provider catalog; refresh failed".to_string()),
        _ => None,
    }
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

// ── Janus local-provider setup (x.ai/providers/setup_janus) ─────────

/// Default Janus base URL prefilled in the setup flow.
pub const JANUS_DEFAULT_BASE_URL: &str = "http://127.0.0.1:20128/v1";

/// Warning shown before accepting a plain-HTTP, non-loopback base URL.
pub const JANUS_INSECURE_URL_WARNING: &str = "This URL sends prompts and credentials over \
     plain HTTP. Continue only if you trust this network.";

/// Outcome state of `x.ai/providers/setup_janus`, matching the shell's
/// lowercase `"ready"`/`"empty"`/`"failed"` wire strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JanusSetupState {
    Ready,
    Empty,
    Failed,
}

/// Response of `x.ai/providers/setup_janus`. Secret-free.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JanusSetupResponse {
    pub state: JanusSetupState,
    /// Models published when `state == Ready`.
    pub model_count: usize,
    /// Last-known-good models still served when `state == Failed`.
    pub cached_models: usize,
    /// Concise, secret-free message for non-ready states.
    pub message: Option<String>,
}

/// Parameters of the `SetupJanus` effect. `Debug` is manual and prints
/// only `has_api_key` so the key can never leak through formatting.
#[derive(Clone)]
pub struct JanusSetupParams {
    pub base_url: String,
    /// Optional key; `None` leaves any stored key unchanged.
    pub api_key: Option<SecretKey>,
    pub allow_insecure_http: bool,
}

impl std::fmt::Debug for JanusSetupParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JanusSetupParams")
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .field("allow_insecure_http", &self.allow_insecure_http)
            .finish()
    }
}

/// User-facing result line for a completed Janus setup, with exact copy:
/// - Ready:  `Janus is ready. {count} models available.`
/// - Empty:  `Janus is healthy but returned no models.`
/// - Failed: `{message}`, plus ` {count} cached models remain available.`
///   only when cached models exist.
pub fn janus_result_message(resp: &JanusSetupResponse) -> String {
    match resp.state {
        JanusSetupState::Ready => {
            format!("Janus is ready. {} models available.", resp.model_count)
        }
        JanusSetupState::Empty => "Janus is healthy but returned no models.".to_string(),
        JanusSetupState::Failed => {
            let message = resp
                .message
                .clone()
                .unwrap_or_else(|| "Janus setup failed.".to_string());
            if resp.cached_models > 0 {
                format!(
                    "{message} {} cached models remain available.",
                    resp.cached_models
                )
            } else {
                message
            }
        }
    }
}

/// True when `url` is plain `http://` to a non-loopback host (anything
/// other than `127.0.0.1`, `[::1]`, or `localhost`). Such URLs send
/// prompts and credentials in cleartext and require explicit confirmation.
pub fn is_insecure_non_loopback_http(url: &str) -> bool {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("http://") else {
        return false; // https:// (or anything else) is not plain HTTP.
    };
    // Authority = up to the first '/'; drop any userinfo before '@'.
    let authority = rest.split('/').next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    !matches!(host, "127.0.0.1" | "::1" | "localhost")
}

/// Ensure a selectable Janus setup row is present. The shell's provider
/// list only includes `janus` once the preset has been registered, so an
/// unconfigured install would otherwise have no way to start setup.
pub fn ensure_janus_row(rows: &mut Vec<ProviderRowView>) {
    if rows.iter().any(|r| r.provider_id == "janus") {
        return;
    }
    rows.push(ProviderRowView {
        provider_id: "janus".to_string(),
        provider_name: "Janus (local)".to_string(),
        status: ProviderStatus::MissingKey,
        disabled: false,
    });
}
