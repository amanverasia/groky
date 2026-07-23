//! Credential precedence and cross-provider leakage regression tests for the
//! injectable `resolve_credentials_with` seam (plan Task 8).

use serial_test::serial;
use xai_grok_catalog::{CatalogModel, CatalogProvider, ModelId, Protocol, ProviderId};
use xai_grok_shell::agent::config::{
    CredentialPolicy, EndpointsConfig, ModelEntry, resolve_credentials_with,
};
use xai_grok_shell::agent::provider_catalog::model_entry_from_catalog;

fn openai_provider() -> CatalogProvider {
    CatalogProvider {
        id: ProviderId::new("openai").unwrap(),
        name: "OpenAI".to_string(),
        api_base_url: "https://api.openai.com/v1".to_string(),
        env_vars: vec!["OPENAI_API_KEY".to_string()],
        unauthenticated: false,
        models: Vec::new(),
    }
}

fn gpt5() -> CatalogModel {
    CatalogModel {
        id: ModelId::new("gpt-5").unwrap(),
        name: "GPT-5".to_string(),
        protocol: Protocol::ChatCompletions,
        context_window: Some(400_000),
        reasoning: true,
        cost: None,
    }
}

fn provider_entry() -> ModelEntry {
    model_entry_from_catalog(&openai_provider(), &gpt5()).unwrap()
}

#[test]
fn xai_legacy_resolution_is_unchanged() {
    let entry = ModelEntry::fallback("grok-4", &EndpointsConfig::default());
    let resolved = resolve_credentials_with(
        &entry,
        Some("xai-oauth-token"),
        || Some("xai-global-key".to_string()),
        |_| None,
    );
    assert_eq!(resolved.api_key.as_deref(), Some("xai-oauth-token"));
    assert_eq!(resolved.auth_type, xai_chat_state::AuthType::SessionToken);
}

#[test]
fn xai_legacy_falls_back_to_global_key_without_session() {
    let entry = ModelEntry::fallback("grok-4", &EndpointsConfig::default());
    let resolved = resolve_credentials_with(
        &entry,
        None,
        || Some("xai-global-key".to_string()),
        |_| None,
    );
    assert_eq!(resolved.api_key.as_deref(), Some("xai-global-key"));
    assert_eq!(resolved.auth_type, xai_chat_state::AuthType::ApiKey);
}

/// The leakage regression: an entry with the openai provider policy must
/// resolve NO credential even when both an xAI session token and a global
/// xAI key are available.
#[test]
#[serial(provider_env)]
fn xai_session_and_global_key_never_leak_to_openai() {
    unsafe { std::env::remove_var("OPENAI_API_KEY") };
    let entry = provider_entry();
    assert_eq!(entry.credential_policy, CredentialPolicy::ProviderApiKey);
    let resolved = resolve_credentials_with(
        &entry,
        Some("xai-oauth-token"),
        || Some("xai-global-key".to_string()),
        |_| None,
    );
    assert!(resolved.api_key.is_none());
    assert_eq!(resolved.base_url, "https://api.openai.com/v1");
}

#[test]
#[serial(provider_env)]
fn provider_credential_precedence_matches_design() {
    unsafe { std::env::set_var("OPENAI_API_KEY", "env-key") };
    let mut entry = provider_entry();
    entry.api_key = Some("model-key".to_string());

    // Stored key beats xAI credentials, catalog env vars, and model credentials.
    let resolved = resolve_credentials_with(
        &entry,
        None,
        || None,
        |provider| (provider == "openai").then(|| "stored-key".to_string()),
    );
    assert_eq!(resolved.api_key.as_deref(), Some("stored-key"));

    // Catalog env var beats model credentials.
    let resolved = resolve_credentials_with(&entry, None, || None, |_| None);
    assert_eq!(resolved.api_key.as_deref(), Some("env-key"));

    // Model credentials are the last resort.
    unsafe { std::env::remove_var("OPENAI_API_KEY") };
    let resolved = resolve_credentials_with(&entry, None, || None, |_| None);
    assert_eq!(resolved.api_key.as_deref(), Some("model-key"));
}

#[test]
#[serial(provider_env)]
fn explicit_model_policy_uses_model_credentials_only() {
    unsafe { std::env::set_var("OPENAI_API_KEY", "env-key") };
    let mut entry = provider_entry();
    entry.credential_policy = CredentialPolicy::ExplicitModel;
    entry.env_key = None;

    // Without model credentials nothing is attached, even with xAI keys,
    // provider keys, and catalog env vars available.
    let resolved = resolve_credentials_with(
        &entry,
        Some("xai-oauth-token"),
        || Some("xai-global-key".to_string()),
        |_| Some("stored-key".to_string()),
    );
    assert!(resolved.api_key.is_none());

    entry.api_key = Some("model-key".to_string());
    let resolved = resolve_credentials_with(&entry, None, || None, |_| None);
    assert_eq!(resolved.api_key.as_deref(), Some("model-key"));
    unsafe { std::env::remove_var("OPENAI_API_KEY") };
}

#[test]
fn none_policy_never_attaches_a_key() {
    let mut entry = provider_entry();
    entry.credential_policy = CredentialPolicy::None;
    entry.api_key = Some("model-key".to_string());
    let resolved = resolve_credentials_with(
        &entry,
        Some("xai-oauth-token"),
        || Some("xai-global-key".to_string()),
        |_| Some("stored-key".to_string()),
    );
    assert!(resolved.api_key.is_none());
    assert_eq!(resolved.base_url, "https://api.openai.com/v1");
}
