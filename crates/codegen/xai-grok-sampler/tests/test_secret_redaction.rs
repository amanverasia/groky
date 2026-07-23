//! Regression tests: configured credentials must never appear in Debug
//! renderings of the sampler config or client.

use indexmap::IndexMap;
use xai_grok_sampler::{ApiBackend, SamplerConfig, SamplingClient};

const API_KEY_CANARY: &str = "sk-janus-super-secret-0123456789";
const HEADER_VALUE_CANARY: &str = "proxy-secret-janus-987654321";
const USER_ID_CANARY: &str = "user-secret-janus-abcdef";
const URL_USERINFO_CANARY: &str = "url-user-secret-janus";
const URL_QUERY_CANARY: &str = "url-query-secret-janus";
const URL_FRAGMENT_CANARY: &str = "url-fragment-secret-janus";

fn config() -> SamplerConfig {
    SamplerConfig {
        api_key: Some(API_KEY_CANARY.to_owned()),
        base_url: format!(
            "https://{URL_USERINFO_CANARY}:url-password@api.example.test:8443/v1/{URL_QUERY_CANARY}?token={URL_QUERY_CANARY}%2Fjson%22#fragment={URL_FRAGMENT_CANARY}%2Fjson%22"
        ),
        model: "openai/gpt-4o".to_owned(),
        api_backend: ApiBackend::ChatCompletions,
        extra_headers: IndexMap::from([
            ("X-Zebra-Trace".to_owned(), HEADER_VALUE_CANARY.to_owned()),
            (
                "Authorization".to_owned(),
                "another-secret-value".to_owned(),
            ),
        ]),
        user_id: Some(USER_ID_CANARY.to_owned()),
        ..SamplerConfig::default()
    }
}

fn assert_safe_diagnostics(rendered: &str) {
    for canary in [
        API_KEY_CANARY,
        "sk-janus",
        HEADER_VALUE_CANARY,
        "proxy-secret-janus",
        USER_ID_CANARY,
        "user-secret-janus",
        URL_USERINFO_CANARY,
        "url-user-secret",
        URL_QUERY_CANARY,
        "url-query-secret",
        URL_FRAGMENT_CANARY,
        "url-fragment-secret",
        "%2Fjson%22",
        "/json\"",
    ] {
        assert!(
            !rendered.contains(canary),
            "secret or secret prefix leaked: {canary}; rendered: {rendered}"
        );
    }
}

#[test]
fn sampler_config_debug_redacts_values_and_retains_safe_fields() {
    let rendered = format!("{:?}", config());
    assert_safe_diagnostics(&rendered);
    assert!(
        rendered.contains("has_api_key: true"),
        "expected API-key presence marker: {rendered}"
    );
    assert!(
        rendered.contains("has_user_id: true"),
        "expected user-ID presence marker: {rendered}"
    );
    assert!(
        rendered.contains("endpoint_identity: \"https://api.example.test:8443\""),
        "expected sanitized endpoint identity: {rendered}"
    );
    assert!(
        rendered.contains("extra_header_names: [\"Authorization\", \"X-Zebra-Trace\"]"),
        "expected sorted header names: {rendered}"
    );
}

#[test]
fn sampling_client_debug_never_contains_secret_values_or_prefixes() {
    let rendered = format!("{:?}", SamplingClient::new(config()).unwrap());
    assert_safe_diagnostics(&rendered);
}
