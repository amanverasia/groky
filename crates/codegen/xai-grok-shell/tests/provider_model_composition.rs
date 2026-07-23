//! Provider model composition into the shell catalog (plan Task 9).
//!
//! Verifies that only configured/environment providers contribute models,
//! that ACP metadata carries secret-free provider context, that `[provider.*]`
//! env_key overrides reach the credential fallback, and that a still-valid
//! current selection survives a provider snapshot replacement.

use std::sync::Arc;

use serial_test::serial;
use xai_grok_catalog::{
    CATALOG_SCHEMA_VERSION, CatalogCache, CatalogModel, CatalogProvider, CredentialOrigin,
    ModelCost, ModelId, NormalizedCatalog, Protocol, ProviderId, encode_cache,
};
use xai_grok_shell::agent::config::{
    Config, ConfigProviderOverride, EnvKeys, resolve_credentials_with, to_acp_model_info,
};
use xai_grok_shell::agent::models::ModelsManager;
use xai_grok_shell::agent::provider_catalog::{
    PROVIDER_CATALOG_CACHE_FILE, PROVIDER_CATALOG_SOURCE_URL, ProviderCatalogAdapter,
    provider_model_entries,
};
use xai_grok_shell::auth::{AuthManager, GrokComConfig};

fn model(id: &str, name: &str) -> CatalogModel {
    CatalogModel {
        id: ModelId::new(id).unwrap(),
        name: name.to_string(),
        protocol: Protocol::ChatCompletions,
        context_window: Some(400_000),
        reasoning: true,
        cost: Some(ModelCost {
            input_per_million: 1.25,
            output_per_million: 10.0,
        }),
    }
}

fn provider(id: &str, name: &str, env_var: &str, models: Vec<CatalogModel>) -> CatalogProvider {
    CatalogProvider {
        id: ProviderId::new(id).unwrap(),
        name: name.to_string(),
        api_base_url: format!("https://api.{id}.example/v1"),
        env_vars: vec![env_var.to_string()],
        unauthenticated: false,
        models,
    }
}

fn sample_catalog() -> NormalizedCatalog {
    NormalizedCatalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        providers: vec![
            provider(
                "anthropic",
                "Anthropic",
                "ANTHROPIC_API_KEY",
                vec![model("claude-sonnet-4", "Claude Sonnet 4")],
            ),
            provider(
                "openai",
                "OpenAI",
                "OPENAI_API_KEY",
                vec![model("gpt-5", "GPT-5"), model("gpt-5-mini", "GPT-5 Mini")],
            ),
        ],
    }
}

#[test]
fn only_configured_provider_models_are_available() {
    let catalog = sample_catalog();
    let models = provider_model_entries(&catalog, &Config::default(), |p: &CatalogProvider| {
        (p.id.as_str() == "openai").then_some(CredentialOrigin::Stored)
    });
    assert!(models.contains_key("openai/gpt-5"));
    assert!(!models.contains_key("anthropic/claude-sonnet-4"));
}

#[test]
fn acp_metadata_contains_provider_context_reasoning_and_cost_but_no_secret() {
    let catalog = sample_catalog();
    let models = provider_model_entries(&catalog, &Config::default(), |_| {
        Some(CredentialOrigin::Stored)
    });
    let infos = to_acp_model_info(&models);
    let (_, info) = infos
        .iter()
        .find(|(id, _)| id.0.as_ref() == "openai/gpt-5")
        .expect("openai/gpt-5 present in ACP model info");
    let meta = info.meta.clone().expect("meta present");
    assert_eq!(meta["providerId"], "openai");
    assert_eq!(meta["providerName"], "OpenAI");
    assert_eq!(meta["totalContextTokens"], 400_000);
    assert_eq!(meta["supportsReasoningEffort"], true);
    assert_eq!(meta["inputCostPerMillion"], 1.25);
    assert_eq!(meta["outputCostPerMillion"], 10.0);
    let text = serde_json::to_string(&meta).unwrap();
    assert!(!text.contains("apiKey"));
    assert!(!text.contains("authorization"));
    assert!(!text.to_lowercase().contains("origin"));
}

/// Fold-in: a `[provider.<id>] env_key` config override must be honored by the
/// catalog-env-var credential fallback (it replaces the embedded env var names).
#[test]
#[serial(provider_env)]
fn provider_env_key_override_reaches_credential_fallback() {
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
        std::env::set_var("MY_OPENAI_KEY", "custom-env-key");
    }
    let mut cfg = Config::default();
    cfg.config_providers.insert(
        "openai".to_string(),
        ConfigProviderOverride {
            env_key: Some(EnvKeys::single("MY_OPENAI_KEY")),
            ..Default::default()
        },
    );
    let catalog = sample_catalog();
    let models = provider_model_entries(&catalog, &cfg, |_| Some(CredentialOrigin::Environment));
    let entry = models.get("openai/gpt-5").expect("entry present");
    let resolved = resolve_credentials_with(entry, None, || None, |_| None);
    assert_eq!(resolved.api_key.as_deref(), Some("custom-env-key"));
    unsafe { std::env::remove_var("MY_OPENAI_KEY") };
}

/// The catalog's `xai` provider must never leak into the generic pipeline:
/// xAI models come from the dedicated entitlement pipeline, even when an
/// XAI_API_KEY-style environment credential is configured.
#[test]
#[serial(provider_env)]
fn xai_catalog_provider_is_excluded_from_generic_composition() {
    unsafe { std::env::set_var("XAI_API_KEY", "xai-env-secret") };
    let mut catalog = sample_catalog();
    catalog.providers.push(provider(
        "xai",
        "xAI",
        "XAI_API_KEY",
        vec![model("grok-4", "Grok 4")],
    ));
    let models = provider_model_entries(&catalog, &Config::default(), |_| {
        Some(CredentialOrigin::Environment)
    });
    unsafe { std::env::remove_var("XAI_API_KEY") };
    assert!(models.contains_key("openai/gpt-5"));
    assert!(
        !models.keys().any(|k| k.starts_with("xai/")),
        "xai/... entries must not be produced by the generic pipeline"
    );
}

#[test]
#[serial(provider_env)]
fn provider_refresh_preserves_a_still_valid_current_selection() {
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };

    // Write a fresh provider catalog cache at $GROK_HOME/provider_catalog.json.
    let cache = CatalogCache {
        schema_version: CATALOG_SCHEMA_VERSION,
        source_url: PROVIDER_CATALOG_SOURCE_URL.to_string(),
        fetched_at: chrono::Utc::now(),
        etag: None,
        last_modified: None,
        catalog: sample_catalog(),
    };
    std::fs::write(
        tmp.path().join(PROVIDER_CATALOG_CACHE_FILE),
        encode_cache(&cache).unwrap(),
    )
    .unwrap();

    let cfg = Config::default();
    let auth = Arc::new(AuthManager::new(tmp.path(), GrokComConfig::default()));
    let mgr = ModelsManager::from_config(&cfg, None, auth).unwrap();

    xai_grok_shell::auth::store_provider_api_key(tmp.path(), "openai", "sk-test").unwrap();
    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));

    mgr.set_provider_catalog(adapter.clone());
    let models = mgr.models();
    assert!(models.contains_key("openai/gpt-5"));
    assert!(models.contains_key("openai/gpt-5-mini"));
    assert!(!models.contains_key("anthropic/claude-sonnet-4"));

    mgr.set_current_model_id(agent_client_protocol::ModelId::new("openai/gpt-5"));

    // Snapshot replacement recomposes the catalog; a still-valid selection
    // must survive.
    mgr.set_provider_catalog(adapter);
    assert_eq!(mgr.current_model_id().0.as_ref(), "openai/gpt-5");
}
