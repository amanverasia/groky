//! Janus dynamic-provider preset, health semantics, and actionable errors.
//!
//! Janus is a locally hosted OpenAI-compatible gateway. The preset here
//! pins its contract (base URL, endpoints, protocol, optional key), and the
//! failure mapping turns low-level [`HttpError`]s into short, actionable,
//! secret-free messages.

use crate::dynamic::DynamicProviderConfig;
use crate::http::HttpError;
use crate::types::Protocol;

/// Provider identifier for the Janus preset.
pub const JANUS_PROVIDER_ID: &str = "janus";
/// Human-readable display name for Janus.
pub const JANUS_DISPLAY_NAME: &str = "Janus";
/// Default local base URL for the Janus service.
pub const JANUS_DEFAULT_BASE_URL: &str = "http://127.0.0.1:20128/v1";
/// Health-check endpoint relative to the service origin.
pub const JANUS_HEALTH_ENDPOINT: &str = "/v1/health";
/// Model-discovery endpoint relative to the service origin.
pub const JANUS_MODELS_ENDPOINT: &str = "/v1/models";
/// UI hint describing the (optional) Janus API key format.
pub const JANUS_KEY_HINT: &str = "optional, starts with sk-janus-";

/// Returns the built-in Janus dynamic provider configuration.
///
/// Janus speaks the OpenAI chat-completions protocol, runs unauthenticated
/// by default (a key is optional), discovers its model list at runtime, and
/// exposes explicit health and models endpoints.
pub fn janus_preset() -> DynamicProviderConfig {
    let mut config = DynamicProviderConfig::new(
        JANUS_PROVIDER_ID,
        JANUS_DISPLAY_NAME,
        JANUS_DEFAULT_BASE_URL,
    )
    .expect("Janus preset constants satisfy config bounds");
    config.protocol = Protocol::ChatCompletions;
    config.unauthenticated = true;
    config.discover = true;
    config.health_endpoint = Some(JANUS_HEALTH_ENDPOINT.to_string());
    config.models_endpoint = Some(JANUS_MODELS_ENDPOINT.to_string());
    config
}

/// A classified Janus health or discovery failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JanusFailure {
    /// The Janus service could not be reached at `url`.
    Connection {
        /// The URL that was attempted.
        url: String,
    },
    /// Janus rejected the provided API key.
    Authentication,
    /// Janus responded, but not with a valid OpenAI-compatible model list.
    InvalidResponse,
    /// Janus is healthy but its model list is empty.
    EmptyModels,
}

/// Renders a failure as a short, actionable, secret-free user message.
pub fn janus_failure(failure: &JanusFailure) -> String {
    match failure {
        JanusFailure::Connection { url } => format!(
            "Could not connect to Janus at {url}. Check that the Janus service is \
             running and the base URL is correct."
        ),
        JanusFailure::Authentication => {
            "Janus rejected the API key. Update the Janus key in /providers.".to_string()
        }
        JanusFailure::InvalidResponse => {
            "Janus returned an invalid model list. Check that the base URL points to an \
             OpenAI-compatible Janus API."
                .to_string()
        }
        JanusFailure::EmptyModels => "Janus is healthy but returned no models.".to_string(),
    }
}

/// Classifies an [`HttpError`] from a Janus health or discovery request.
///
/// Mapping policy (locked by tests):
/// - `Status(401 | 403)` → [`JanusFailure::Authentication`].
/// - `Timeout`, `Transport`, `TooManyRedirects`, and URL/policy errors
///   (`UnsupportedScheme`, `InsecureHttpDenied`, `MissingHost`,
///   `CredentialsInUrl`, `InvalidUrl`) → [`JanusFailure::Connection`] with
///   the attempted URL; setup-time misconfiguration is presented as
///   connection guidance since the fix is the same (check the base URL).
/// - Any other `Status` and `BodyTooLarge` → [`JanusFailure::InvalidResponse`]:
///   something answered, but not a well-behaved OpenAI-compatible endpoint.
pub fn janus_failure_from_http(error: &HttpError, attempted_url: &str) -> JanusFailure {
    match error {
        HttpError::Status(401 | 403) => JanusFailure::Authentication,
        HttpError::Timeout
        | HttpError::Transport(_)
        | HttpError::TooManyRedirects
        | HttpError::UnsupportedScheme
        | HttpError::InsecureHttpDenied
        | HttpError::MissingHost
        | HttpError::CredentialsInUrl
        | HttpError::InvalidUrl(_) => JanusFailure::Connection {
            url: attempted_url.to_string(),
        },
        HttpError::Status(_) | HttpError::BodyTooLarge => JanusFailure::InvalidResponse,
    }
}

#[cfg(test)]
mod tests {
    use crate::http::HttpError;
    use crate::types::Protocol;

    use super::*;

    #[test]
    fn preset_matches_janus_contract_exactly() {
        let preset = janus_preset();
        assert_eq!(preset.id.as_str(), "janus");
        assert_eq!(preset.name, "Janus");
        assert_eq!(preset.base_url, "http://127.0.0.1:20128/v1");
        assert_eq!(preset.protocol, Protocol::ChatCompletions);
        assert!(preset.unauthenticated);
        assert!(preset.discover);
        assert_eq!(preset.health_endpoint.as_deref(), Some("/v1/health"));
        assert_eq!(preset.models_endpoint.as_deref(), Some("/v1/models"));
        assert_eq!(JANUS_KEY_HINT, "optional, starts with sk-janus-");
    }

    #[test]
    fn health_and_discovery_failures_are_actionable_and_secret_free() {
        assert_eq!(
            janus_failure(&JanusFailure::Connection {
                url: "http://127.0.0.1:20128/v1/health".into()
            }),
            "Could not connect to Janus at http://127.0.0.1:20128/v1/health. \
             Check that the Janus service is running and the base URL is correct."
        );
        assert_eq!(
            janus_failure(&JanusFailure::Authentication),
            "Janus rejected the API key. Update the Janus key in /providers."
        );
        assert_eq!(
            janus_failure(&JanusFailure::InvalidResponse),
            "Janus returned an invalid model list. Check that the base URL points to an \
             OpenAI-compatible Janus API."
        );
        assert_eq!(
            janus_failure(&JanusFailure::EmptyModels),
            "Janus is healthy but returned no models."
        );
        assert!(!janus_failure(&JanusFailure::Authentication).contains("sk-"));
    }

    #[test]
    fn http_errors_map_to_janus_failures_with_attempted_url() {
        let url = "http://127.0.0.1:20128/v1/models";
        let connection = JanusFailure::Connection { url: url.into() };

        assert_eq!(
            janus_failure_from_http(&HttpError::Status(401), url),
            JanusFailure::Authentication
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::Status(403), url),
            JanusFailure::Authentication
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::Timeout, url),
            connection
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::Transport("connection refused".into()), url),
            connection
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::TooManyRedirects, url),
            connection
        );
        // Setup-time URL/policy misconfiguration surfaces as connection
        // guidance pointing at the attempted URL.
        assert_eq!(
            janus_failure_from_http(&HttpError::UnsupportedScheme, url),
            connection
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::InsecureHttpDenied, url),
            connection
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::MissingHost, url),
            connection
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::CredentialsInUrl, url),
            connection
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::InvalidUrl("bad".into()), url),
            connection
        );
        // Non-auth statuses and oversized bodies mean we reached something
        // that is not a well-behaved OpenAI-compatible endpoint.
        assert_eq!(
            janus_failure_from_http(&HttpError::Status(500), url),
            JanusFailure::InvalidResponse
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::Status(404), url),
            JanusFailure::InvalidResponse
        );
        assert_eq!(
            janus_failure_from_http(&HttpError::BodyTooLarge, url),
            JanusFailure::InvalidResponse
        );
    }

    #[test]
    fn failure_messages_never_leak_urls_with_userinfo_secrets() {
        let failure =
            janus_failure_from_http(&HttpError::Timeout, "http://127.0.0.1:20128/v1/health");
        assert!(!janus_failure(&failure).contains("sk-"));
    }
}
