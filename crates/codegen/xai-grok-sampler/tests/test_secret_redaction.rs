//! Regression tests: configured credentials must never appear in Debug
//! renderings of the sampler config or client.

use xai_grok_sampler::{ApiBackend, SamplerConfig, SamplingClient};

const SECRET: &str = "sk-janus-super-secret-0123456789";

fn config() -> SamplerConfig {
    SamplerConfig {
        api_key: Some(SECRET.to_owned()),
        base_url: "http://127.0.0.1:20128/v1".to_owned(),
        model: "openai/gpt-4o".to_owned(),
        api_backend: ApiBackend::ChatCompletions,
        ..SamplerConfig::default()
    }
}

#[test]
fn sampler_config_debug_never_contains_key_or_prefix() {
    let rendered = format!("{:?}", config());
    assert!(!rendered.contains(SECRET), "full key leaked: {rendered}");
    assert!(!rendered.contains("sk-janus"), "key prefix leaked: {rendered}");
    assert!(
        rendered.contains("has_api_key: true"),
        "expected boolean presence marker: {rendered}"
    );
}

#[test]
fn sampling_client_debug_never_contains_key_or_prefix() {
    let rendered = format!("{:?}", SamplingClient::new(config()).unwrap());
    assert!(!rendered.contains(SECRET), "full key leaked: {rendered}");
    assert!(!rendered.contains("sk-janus"), "key prefix leaked: {rendered}");
}
