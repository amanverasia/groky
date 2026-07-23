//! Credential-safe HTTP client for dynamic provider discovery.
//!
//! Enforces the dynamic-provider URL policy (https, or plain http restricted
//! to loopback unless explicitly opted in) and performs manual redirect
//! handling so bearer credentials are stripped on cross-origin redirects.
//! Response bodies are size-capped while streaming.

use futures_util::StreamExt as _;
use url::Url;

use crate::credentials::SecretString;
use crate::limits::{DISCOVERY_TIMEOUT, HEALTH_TIMEOUT, MAX_DISCOVERY_BODY_BYTES, MAX_REDIRECTS};

/// The kind of discovery request being made; selects the network timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    /// Full model-list discovery ([`DISCOVERY_TIMEOUT`]).
    Discovery,
    /// Lightweight provider health check ([`HEALTH_TIMEOUT`]).
    Health,
}

impl RequestKind {
    fn timeout(self) -> std::time::Duration {
        match self {
            RequestKind::Discovery => DISCOVERY_TIMEOUT,
            RequestKind::Health => HEALTH_TIMEOUT,
        }
    }
}

/// Policy or transport failure while performing a discovery request.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("unsupported URL scheme; use https or loopback http")]
    UnsupportedScheme,
    #[error("plain HTTP is allowed only for loopback unless allow_insecure_http=true")]
    InsecureHttpDenied,
    #[error("URL has no host")]
    MissingHost,
    #[error("URLs must not embed credentials (userinfo)")]
    CredentialsInUrl,
    #[error("endpoint override must be a query-free, fragment-free path on the provider origin")]
    InvalidEndpointOverride,
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("too many redirects (limit {MAX_REDIRECTS})")]
    TooManyRedirects,
    #[error("response body exceeds {MAX_DISCOVERY_BODY_BYTES} bytes")]
    BodyTooLarge,
    #[error("request timed out")]
    Timeout,
    #[error("transport error: {0}")]
    Transport(String),
    #[error("unexpected HTTP status {0}")]
    Status(u16),
}

/// A fully read, size-bounded HTTP response.
#[derive(Clone, Debug)]
pub struct BoundedResponse {
    /// The URL that produced the final (non-redirect) response.
    pub final_url: Url,
    /// Final response status.
    pub status: reqwest::StatusCode,
    /// Complete response body, at most [`MAX_DISCOVERY_BODY_BYTES`] bytes.
    pub body: Vec<u8>,
}

/// Derives the discovery endpoint from a provider base URL.
///
/// With `override_path` (origin-rooted, starting with `/`), the override
/// replaces the base URL's path. Otherwise `default_leaf` is appended to the
/// trailing-slash-normalized base path. Scheme/host/userinfo policy is
/// enforced; the loopback-only plain-HTTP rule is checked at request time.
pub fn derive_endpoint(
    base: &str,
    override_path: Option<&str>,
    default_leaf: &str,
) -> Result<Url, HttpError> {
    let base_url = Url::parse(base).map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
    let endpoint = match override_path {
        Some(path) => {
            if !path.starts_with('/') || path.starts_with("//") {
                return Err(HttpError::InvalidEndpointOverride);
            }
            let endpoint = base_url
                .join(path)
                .map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
            if endpoint.query().is_some() || endpoint.fragment().is_some() {
                return Err(HttpError::InvalidEndpointOverride);
            }
            endpoint
        }
        None => {
            let mut url = base_url.clone();
            let joined = format!("{}/{}", url.path().trim_end_matches('/'), default_leaf);
            url.set_path(&joined);
            url
        }
    };
    // Scheme, host, and userinfo checks apply regardless of the insecure-http
    // opt-in, which is enforced against the resolved flag in `get_bounded`.
    validate_url(&endpoint, true)?;
    if !same_origin(&base_url, &endpoint) {
        return Err(HttpError::InvalidEndpointOverride);
    }
    Ok(endpoint)
}

/// Validates a URL against the dynamic-provider policy.
///
/// `https` is always allowed. `http` is allowed only for literal loopback
/// addresses (or the exact host `localhost`) unless `allow_insecure_http`.
/// Embedded userinfo is always rejected.
pub fn validate_url(url: &Url, allow_insecure_http: bool) -> Result<(), HttpError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(HttpError::CredentialsInUrl);
    }
    match url.scheme() {
        "https" => {
            if url.host().is_none() {
                return Err(HttpError::MissingHost);
            }
            Ok(())
        }
        "http" => {
            let host = url.host().ok_or(HttpError::MissingHost)?;
            let loopback = match host {
                url::Host::Ipv4(ip) => ip.is_loopback(),
                url::Host::Ipv6(ip) => ip.is_loopback(),
                url::Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
            };
            if loopback || allow_insecure_http {
                Ok(())
            } else {
                Err(HttpError::InsecureHttpDenied)
            }
        }
        _ => Err(HttpError::UnsupportedScheme),
    }
}

/// Renders a URL with any embedded userinfo stripped, for use in
/// user-facing failure messages. Defense in depth: [`validate_url`] rejects
/// userinfo anyway, but a message-building path must never rely on that.
pub fn redact_userinfo(url: &Url) -> String {
    if url.username().is_empty() && url.password().is_none() {
        return url.to_string();
    }
    let mut url = url.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

/// Builds a client with automatic redirects disabled; redirects are followed
/// manually in [`get_bounded`] so credentials can be stripped cross-origin.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("discovery HTTP client construction cannot fail")
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Performs a GET with manual, policy-checked redirects and a streamed,
/// size-capped body read.
///
/// The bearer credential (if any) is sent on the initial request and on
/// same-origin redirects only; any cross-origin hop drops it for all
/// subsequent hops. Every hop is re-validated against the URL policy before
/// a connection is attempted.
pub async fn get_bounded(
    client: &reqwest::Client,
    endpoint: &str,
    credential: Option<&SecretString>,
    allow_insecure_http: bool,
    kind: RequestKind,
) -> Result<BoundedResponse, HttpError> {
    let url = Url::parse(endpoint).map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
    tokio::time::timeout(
        kind.timeout(),
        follow_and_read(client, url, credential, allow_insecure_http),
    )
    .await
    .map_err(|_| HttpError::Timeout)?
}

async fn follow_and_read(
    client: &reqwest::Client,
    mut url: Url,
    credential: Option<&SecretString>,
    allow_insecure_http: bool,
) -> Result<BoundedResponse, HttpError> {
    let mut send_credential = credential.is_some();
    for redirects in 0..=MAX_REDIRECTS {
        validate_url(&url, allow_insecure_http)?;
        let mut request = client.get(url.clone());
        if send_credential && let Some(secret) = credential {
            request = request.bearer_auth(secret.expose());
        }
        let response = request.send().await.map_err(|err| {
            if err.is_timeout() {
                HttpError::Timeout
            } else {
                HttpError::Transport(err.to_string())
            }
        })?;

        if response.status().is_redirection() {
            if redirects == MAX_REDIRECTS {
                return Err(HttpError::TooManyRedirects);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| HttpError::Status(response.status().as_u16()))?;
            let next = url
                .join(location)
                .map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
            validate_url(&next, allow_insecure_http)?;
            send_credential &= same_origin(&url, &next);
            url = next;
            continue;
        }

        if !response.status().is_success() {
            return Err(HttpError::Status(response.status().as_u16()));
        }
        if let Some(length) = response.content_length()
            && length > MAX_DISCOVERY_BODY_BYTES as u64
        {
            return Err(HttpError::BodyTooLarge);
        }

        let status = response.status();
        let final_url = response.url().clone();
        let mut body: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| HttpError::Transport(err.to_string()))?;
            if body.len().saturating_add(chunk.len()) > MAX_DISCOVERY_BODY_BYTES {
                return Err(HttpError::BodyTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(BoundedResponse {
            final_url,
            status,
            body,
        });
    }
    Err(HttpError::TooManyRedirects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_models_relative_to_normalized_base() {
        assert_eq!(
            derive_endpoint("https://gateway.example/v1", None, "models")
                .unwrap()
                .as_str(),
            "https://gateway.example/v1/models"
        );
        assert_eq!(
            derive_endpoint("https://gateway.example/v1/", None, "models")
                .unwrap()
                .as_str(),
            "https://gateway.example/v1/models"
        );
        assert_eq!(
            derive_endpoint(
                "https://gateway.example/api",
                Some("/custom/models"),
                "models"
            )
            .unwrap()
            .as_str(),
            "https://gateway.example/custom/models"
        );
    }

    #[test]
    fn endpoint_overrides_stay_on_origin_and_cannot_carry_request_parts() {
        for override_path in [
            "https://attacker.example/models",
            "//attacker.example/models",
            "/models?api_key=secret",
            "/models#secret",
            "models",
        ] {
            assert_eq!(
                derive_endpoint("https://gateway.example/v1", Some(override_path), "models")
                    .unwrap_err(),
                HttpError::InvalidEndpointOverride,
                "override {override_path:?} must be rejected"
            );
        }
        assert_eq!(
            derive_endpoint(
                "https://gateway.example/v1",
                Some("/custom/models"),
                "models"
            )
            .unwrap()
            .as_str(),
            "https://gateway.example/custom/models"
        );
    }

    #[test]
    fn plain_http_is_loopback_only_without_opt_in() {
        assert!(
            validate_url(
                &url::Url::parse("http://127.0.0.1:20128/v1/models").unwrap(),
                false
            )
            .is_ok()
        );
        assert!(
            validate_url(
                &url::Url::parse("http://[::1]:20128/v1/models").unwrap(),
                false
            )
            .is_ok()
        );
        assert_eq!(
            validate_url(
                &url::Url::parse("http://192.168.1.20:20128/v1/models").unwrap(),
                false
            )
            .unwrap_err(),
            HttpError::InsecureHttpDenied
        );
        assert!(
            validate_url(
                &url::Url::parse("http://192.168.1.20:20128/v1/models").unwrap(),
                true
            )
            .is_ok()
        );
        assert_eq!(
            validate_url(&url::Url::parse("file:///tmp/models").unwrap(), true).unwrap_err(),
            HttpError::UnsupportedScheme
        );
    }

    #[test]
    fn literal_localhost_is_accepted_without_opt_in() {
        assert!(validate_url(&url::Url::parse("http://localhost:9000/x").unwrap(), false).is_ok());
        // Only the exact literal "localhost" qualifies; other names are denied.
        assert_eq!(
            validate_url(&url::Url::parse("http://evil.example/x").unwrap(), false).unwrap_err(),
            HttpError::InsecureHttpDenied
        );
        assert_eq!(
            validate_url(
                &url::Url::parse("http://localhost.evil.example/x").unwrap(),
                false
            )
            .unwrap_err(),
            HttpError::InsecureHttpDenied
        );
    }

    #[test]
    fn urls_with_userinfo_are_rejected() {
        let with_both = url::Url::parse("https://user:pass@gateway.example/v1/models").unwrap();
        assert!(matches!(
            validate_url(&with_both, false).unwrap_err(),
            HttpError::CredentialsInUrl
        ));
        let with_user = url::Url::parse("https://user@gateway.example/v1/models").unwrap();
        assert!(matches!(
            validate_url(&with_user, false).unwrap_err(),
            HttpError::CredentialsInUrl
        ));
    }

    #[test]
    fn derive_endpoint_rejects_invalid_and_policy_violating_bases() {
        assert!(matches!(
            derive_endpoint("not a url", None, "models").unwrap_err(),
            HttpError::InvalidUrl(_)
        ));
        assert_eq!(
            derive_endpoint("ftp://gateway.example/v1", None, "models").unwrap_err(),
            HttpError::UnsupportedScheme
        );
    }
}
