//! Sampler configuration types.
//!
//! [`SamplerConfig`] is the per-request configuration handed to the
//! sampler. It deliberately does **not** alias
//! `xai_grok_sampling_types::SamplingConfig` so that the sampler crate
//! avoids transitive dependencies on shell-specific types
//! (`xai-grok-tools`, etc.).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use xai_grok_sampling_types::{
    ApiBackend, CompactionAtTokens, CompactionsRemaining, DoomLoopRecoveryPolicy, ReasoningEffort,
};

use crate::attribution::SharedAttributionCallback;
use crate::retry::{DEFAULT_MAX_RETRIES, RATE_LIMIT_RETRY_THRESHOLD};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthScheme {
    #[default]
    Bearer,
    XApiKey,
}

/// A serializable, credential-free diagnostic view of [`SamplerConfig`].
///
/// This is intentionally a crate-private, one-way projection. It contains
/// operational settings and presence flags, never credential, header, user-ID,
/// or client-identity values. `model` is retained as necessary operational
/// context; callers must not place secrets in model identifiers.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SamplerConfigSnapshot {
    pub has_api_key: bool,
    pub endpoint_identity: String,
    pub model: String,
    pub max_completion_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub api_backend: ApiBackend,
    pub auth_scheme: AuthScheme,
    pub extra_header_names: Vec<String>,
    pub context_window: u64,
    pub force_http1: bool,
    pub max_retries: Option<u32>,
    pub stream_tool_calls: bool,
    pub idle_timeout_secs: Option<u64>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub has_user_id: bool,
    pub has_attribution_callback: bool,
    pub has_bearer_resolver: bool,
    pub supports_backend_search: bool,
    pub compactions_remaining: Option<CompactionsRemaining>,
    pub compaction_at_tokens: Option<CompactionAtTokens>,
    pub doom_loop_recovery: Option<DoomLoopRecoveryPolicy>,
    pub has_header_injector: bool,
}

/// All knobs that control a single sampling request.
///
/// The session typically owns one `SamplerConfig` per active model
/// and passes it (or a per-request override) to the actor on every
/// submit.
///
/// # Construction in `xai-grok-shell`
///
/// `SamplerConfig` is the single source of truth for sampler
/// configuration. The shell builds it directly (see
/// `agent::config::resolve_model_to_sampling_config` and
/// `session::acp_session::SessionActor::reconstruct_full_config`) by
/// composing chat-state's `xai_grok_sampling_types::SamplingConfig`
/// with `Credentials` (api key, client version).
///
/// URL-derived request headers (e.g. `X-XAI-Token-Auth` for the
/// cli-chat-proxy) are
/// folded into [`Self::extra_headers`] by
/// `agent::config::inject_url_derived_headers` before the
/// `SamplerConfig` is handed to the actor. Auth is selected separately
/// via `auth_scheme`, while `api_backend` controls only the request/response
/// protocol shape.
#[derive(Clone)]
pub struct SamplerConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub max_completion_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub api_backend: ApiBackend,
    pub auth_scheme: AuthScheme,
    /// Extra request headers applied verbatim. The sampler never inspects
    /// the URL to derive headers; callers (the session) inject proxy auth
    /// and other access headers here before constructing the config.
    pub extra_headers: IndexMap<String, String>,
    /// Total context window size in tokens. The sampler does not enforce
    /// it; it is informational metadata used by the session for compaction
    /// decisions.
    pub context_window: u64,
    pub force_http1: bool,
    pub max_retries: Option<u32>,
    pub stream_tool_calls: bool,
    pub idle_timeout_secs: Option<u64>,

    // Reasoning effort
    pub reasoning_effort: Option<ReasoningEffort>,

    // Client identity
    pub origin_client: Option<OriginClientInfo>,
    pub client_identifier: Option<String>,
    pub deployment_id: Option<String>,
    pub user_id: Option<String>,
    pub client_version: Option<String>,

    /// Optional hook invoked at every UNAUTHORIZED (401) response
    /// site. The sampler passes the bearer that was actually sent on
    /// the wire to the callback; the implementation is free to do
    /// whatever it wants with it (typically: join it with a live
    /// credential source and emit an attribution event for diagnosis
    /// of stale-token vs. server-rejected-live-token 401s). `None`
    /// (default) is a no-op -- the 401 arm returns the same
    /// `SamplingError::Auth` it always did.
    pub attribution_callback: Option<SharedAttributionCallback>,

    /// Live bearer resolve per request. `None` uses construction-time `api_key`.
    pub bearer_resolver: Option<SharedBearerResolver>,

    pub supports_backend_search: bool,

    /// Per-model config for the `x-compactions-remaining` header; `None` disables it.
    pub compactions_remaining: Option<CompactionsRemaining>,

    /// Per-model config for the `x-compaction-at` header; `None` disables it.
    pub compaction_at_tokens: Option<CompactionAtTokens>,

    /// Server-side doom-loop check policy; `None` disables it. When set, the
    /// client itself sends the opt-in `x-grok-doom-loop-check` header on
    /// streaming Responses API requests and absorbs the reported trigger
    /// events (unlike the environment headers in [`Self::extra_headers`],
    /// this header gates the client's own decode behavior, so it lives with
    /// the decoder).
    pub doom_loop_recovery: Option<DoomLoopRecoveryPolicy>,

    /// Per-request header injector (e.g. OTel traceparent). Called in `post()`.
    pub header_injector: Option<SharedHeaderInjector>,
}

impl SamplerConfig {
    /// Return a serializable diagnostics projection with secrets redacted.
    pub(crate) fn safe_snapshot(&self) -> SamplerConfigSnapshot {
        let mut extra_header_names = self.extra_headers.keys().cloned().collect::<Vec<_>>();
        extra_header_names.sort_unstable();

        SamplerConfigSnapshot {
            has_api_key: self.api_key.is_some(),
            endpoint_identity: endpoint_identity(&self.base_url),
            model: self.model.clone(),
            max_completion_tokens: self.max_completion_tokens,
            temperature: self.temperature,
            top_p: self.top_p,
            api_backend: self.api_backend.clone(),
            auth_scheme: self.auth_scheme,
            extra_header_names,
            context_window: self.context_window,
            force_http1: self.force_http1,
            max_retries: self.max_retries,
            stream_tool_calls: self.stream_tool_calls,
            idle_timeout_secs: self.idle_timeout_secs,
            reasoning_effort: self.reasoning_effort,
            has_user_id: self.user_id.is_some(),
            has_attribution_callback: self.attribution_callback.is_some(),
            has_bearer_resolver: self.bearer_resolver.is_some(),
            supports_backend_search: self.supports_backend_search,
            compactions_remaining: self.compactions_remaining,
            compaction_at_tokens: self.compaction_at_tokens,
            doom_loop_recovery: self.doom_loop_recovery,
            has_header_injector: self.header_injector.is_some(),
        }
    }
}

/// Return a stable endpoint identity without URL credentials or request parts.
pub(crate) fn endpoint_identity(base_url: &str) -> String {
    let Ok(url) = url::Url::parse(base_url) else {
        return "invalid".to_owned();
    };
    let Some(host) = url.host() else {
        return "invalid".to_owned();
    };

    let mut identity = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        identity.push(':');
        identity.push_str(&port.to_string());
    }
    identity
}

/// Manual `Debug` delegates to the vetted diagnostics projection so secret
/// redaction cannot drift between debug output and serialized diagnostics.
impl std::fmt::Debug for SamplerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.safe_snapshot().fmt(f)
    }
}

impl Default for SamplerConfig {
    /// Empty defaults so callers can use `..Default::default()` and
    /// new fields don't ripple through every literal site.
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: String::new(),
            model: String::new(),
            max_completion_tokens: None,
            temperature: None,
            top_p: None,
            api_backend: ApiBackend::default(),
            auth_scheme: AuthScheme::default(),
            extra_headers: IndexMap::new(),
            context_window: 0,
            force_http1: false,
            max_retries: None,
            stream_tool_calls: false,
            idle_timeout_secs: None,
            reasoning_effort: None,
            origin_client: None,
            client_identifier: None,
            deployment_id: None,
            user_id: None,
            client_version: None,
            attribution_callback: None,
            bearer_resolver: None,
            supports_backend_search: false,
            compactions_remaining: None,
            compaction_at_tokens: None,
            doom_loop_recovery: None,
            header_injector: None,
        }
    }
}

/// Cheap sync read of the current bearer for [`SamplerConfig::bearer_resolver`].
pub trait BearerResolver: Send + Sync + std::fmt::Debug {
    fn current_bearer(&self) -> Option<String>;
}

pub type SharedBearerResolver = std::sync::Arc<dyn BearerResolver>;

/// Per-request header injection (e.g. OTel `traceparent`).
pub trait HeaderInjector: Send + Sync + std::fmt::Debug {
    fn inject(&self, headers: &mut reqwest::header::HeaderMap);
}

pub type SharedHeaderInjector = std::sync::Arc<dyn HeaderInjector>;

/// Retry knobs for the sampler's internal transport-error retry loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retries before giving up.
    pub max_retries: u32,
    /// After this many rate-limit (429) retries, escalate to the caller.
    /// Lower than `max_retries` because rate-limit waits can be long.
    pub rate_limit_retry_threshold: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            rate_limit_retry_threshold: RATE_LIMIT_RETRY_THRESHOLD,
        }
    }
}

/// Identity of the client that originated the request, used for
/// User-Agent rendering. The shell layer composes this with platform
/// info into a final UA string.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OriginClientInfo {
    pub product: String,
    pub version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_defaults() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_retries, DEFAULT_MAX_RETRIES);
        assert_eq!(
            policy.rate_limit_retry_threshold,
            RATE_LIMIT_RETRY_THRESHOLD
        );
    }

    #[test]
    fn safe_snapshot_json_redacts_url_credentials_and_request_parts() {
        let api_key_canary = "sk-janus-super-secret-0123456789";
        let header_value_canary = "proxy-secret-janus-987654321";
        let user_id_canary = "user-secret-janus-abcdef";
        let url_userinfo_canary = "url-user-secret-janus";
        let url_query_canary = "url-query-secret-janus";
        let url_fragment_canary = "url-fragment-secret-janus";
        let config = SamplerConfig {
            api_key: Some(api_key_canary.to_owned()),
            base_url: format!(
                "https://{url_userinfo_canary}:url-password@api.example.test:8443/v1/{url_query_canary}?token={url_query_canary}%2Fjson%22#fragment={url_fragment_canary}%2Fjson%22"
            ),
            model: "openai/gpt-4o".to_owned(),
            extra_headers: IndexMap::from([
                ("X-Zebra-Trace".to_owned(), header_value_canary.to_owned()),
                (
                    "Authorization".to_owned(),
                    "another-secret-value".to_owned(),
                ),
            ]),
            user_id: Some(user_id_canary.to_owned()),
            ..Default::default()
        };

        let rendered = serde_json::to_string(&config.safe_snapshot()).unwrap();
        for canary in [
            api_key_canary,
            "sk-janus",
            header_value_canary,
            "proxy-secret-janus",
            user_id_canary,
            "user-secret-janus",
            url_userinfo_canary,
            "url-user-secret",
            url_query_canary,
            "url-query-secret",
            url_fragment_canary,
            "url-fragment-secret",
            "%2Fjson%22",
            "/json\"",
        ] {
            assert!(
                !rendered.contains(canary),
                "secret or request URL part leaked: {canary}; rendered: {rendered}"
            );
        }
        assert!(rendered.contains("\"endpoint_identity\":\"https://api.example.test:8443\""));
        assert!(rendered.contains("\"has_api_key\":true"));
        assert!(rendered.contains("\"has_user_id\":true"));
        assert!(rendered.contains("\"extra_header_names\":[\"Authorization\",\"X-Zebra-Trace\"]"));
    }
}
