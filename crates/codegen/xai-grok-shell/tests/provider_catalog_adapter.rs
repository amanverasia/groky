//! Integration tests for `[provider.<id>]` static config parsing and the
//! catalog → shell `ModelEntry` adapter (plan Task 7).

use std::num::NonZeroU64;

use xai_grok_catalog::{CatalogModel, CatalogProvider, ModelCost, ModelId, Protocol, ProviderId};
use xai_grok_shell::agent::config::{Config, CredentialPolicy, ModelEntry};
use xai_grok_shell::agent::provider_catalog::{
    apply_config_provider_override, model_entry_from_catalog,
};
use xai_grok_shell::sampling::ApiBackend;

fn sample_model() -> CatalogModel {
    CatalogModel {
        id: ModelId::new("gpt-5").unwrap(),
        name: "GPT-5".to_string(),
        protocol: Protocol::ChatCompletions,
        context_window: Some(400_000),
        reasoning: true,
        cost: Some(ModelCost {
            input_per_million: 1.25,
            output_per_million: 10.0,
        }),
    }
}

fn sample_provider() -> CatalogProvider {
    CatalogProvider {
        id: ProviderId::new("openai").unwrap(),
        name: "OpenAI".to_string(),
        api_base_url: "https://api.openai.com/v1".to_string(),
        env_vars: vec!["OPENAI_API_KEY".to_string()],
        unauthenticated: false,
        models: vec![sample_model()],
    }
}

#[test]
fn parses_static_provider_override_without_discovery_fields() {
    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(
            r#"
        [provider.openai]
        name = "OpenAI Team"
        base_url = "https://gateway.example/v1"
        env_key = ["OPENAI_API_KEY"]
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    let openai = &cfg.config_providers["openai"];
    assert_eq!(openai.name.as_deref(), Some("OpenAI Team"));
    assert_eq!(
        openai.base_url.as_deref(),
        Some("https://gateway.example/v1")
    );
    assert_eq!(
        openai.env_key.as_ref().and_then(|k| k.primary()),
        Some("OPENAI_API_KEY")
    );
    assert_eq!(openai.unauthenticated, None);
}

#[test]
fn provider_override_survives_an_invalid_field() {
    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(
            r#"
        [provider.openai]
        name = "OpenAI Team"
        unauthenticated = "not-a-bool"
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    let openai = cfg
        .config_providers
        .get("openai")
        .expect("provider must survive a bad field");
    assert_eq!(openai.name.as_deref(), Some("OpenAI Team"));
    assert_eq!(openai.unauthenticated, None);
}

#[test]
fn provider_override_ignores_unknown_discovery_fields() {
    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(
            r#"
        [provider.openai]
        base_url = "https://gateway.example/v1"
        discovery = "models-endpoint"
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    let openai = cfg.config_providers.get("openai").expect("provider kept");
    assert_eq!(
        openai.base_url.as_deref(),
        Some("https://gateway.example/v1")
    );
}

#[test]
fn adapter_uses_qualified_ids_and_never_copies_credentials() {
    let entry = model_entry_from_catalog(&sample_provider(), &sample_model()).unwrap();
    assert_eq!(entry.info.id.as_deref(), Some("openai/gpt-5"));
    assert_eq!(entry.info.model, "gpt-5");
    assert_eq!(entry.info.base_url, "https://api.openai.com/v1");
    assert_eq!(entry.info.name.as_deref(), Some("GPT-5"));
    assert_eq!(entry.info.context_window, NonZeroU64::new(400_000).unwrap());
    assert!(entry.info.supports_reasoning_effort);
    assert_eq!(
        entry.provider_id.as_ref().map(ProviderId::as_str),
        Some("openai")
    );
    assert_eq!(entry.credential_policy, CredentialPolicy::ProviderApiKey);
    assert!(entry.api_key.is_none());
    assert!(entry.env_key.is_none());
}

#[test]
fn adapter_maps_protocols_to_api_backends() {
    let provider = sample_provider();
    let entry_for = |protocol: Protocol| {
        let model = CatalogModel {
            protocol,
            ..sample_model()
        };
        model_entry_from_catalog(&provider, &model).unwrap()
    };
    assert_eq!(
        entry_for(Protocol::ChatCompletions).info.api_backend,
        ApiBackend::ChatCompletions
    );
    assert_eq!(
        entry_for(Protocol::Responses).info.api_backend,
        ApiBackend::Responses
    );
    assert_eq!(
        entry_for(Protocol::Messages).info.api_backend,
        ApiBackend::Messages
    );
}

#[test]
fn adapter_rejects_unsupported_protocol() {
    let model = CatalogModel {
        protocol: Protocol::Unsupported,
        ..sample_model()
    };
    assert!(model_entry_from_catalog(&sample_provider(), &model).is_err());
}

#[test]
fn adapter_maps_unauthenticated_provider_to_none_policy() {
    let provider = CatalogProvider {
        unauthenticated: true,
        ..sample_provider()
    };
    let entry = model_entry_from_catalog(&provider, &sample_model()).unwrap();
    assert_eq!(entry.credential_policy, CredentialPolicy::None);
}

#[test]
fn config_provider_override_applies_onto_catalog_provider() {
    let mut provider = sample_provider();
    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(
            r#"
        [provider.openai]
        name = "OpenAI Team"
        base_url = "https://gateway.example/v1"
        env_key = ["OPENAI_TEAM_KEY"]
        unauthenticated = true
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    apply_config_provider_override(&mut provider, &cfg.config_providers["openai"]);
    assert_eq!(provider.name, "OpenAI Team");
    assert_eq!(provider.api_base_url, "https://gateway.example/v1");
    assert_eq!(provider.env_vars, ["OPENAI_TEAM_KEY"]);
    assert!(provider.unauthenticated);
}

/// Old `models_cache.json` entries predate `provider_id`/`credential_policy`;
/// they must load with the `XaiLegacy` default.
#[test]
fn legacy_model_entry_json_defaults_to_xai_legacy_policy() {
    let entry = ModelEntry::fallback("grok-4", &Default::default());
    let mut json = serde_json::to_value(&entry).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("provider_id");
    obj.remove("credential_policy");
    let reloaded: ModelEntry = serde_json::from_value(json).unwrap();
    assert_eq!(reloaded.credential_policy, CredentialPolicy::XaiLegacy);
    assert!(reloaded.provider_id.is_none());
}
