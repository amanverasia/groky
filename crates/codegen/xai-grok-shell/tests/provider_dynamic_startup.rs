use std::sync::Arc;

use serial_test::serial;
use xai_grok_catalog::{
    CachedModel, CachedProviderModels, DynamicCache, DynamicProviderConfig, ModelId,
};
use xai_grok_shell::agent::config::Config;
use xai_grok_shell::agent::init::bootstrap;
use xai_grok_shell::auth::{AuthManager, GrokComConfig};
use xai_grok_test_support::env::EnvGuard;

#[tokio::test(flavor = "current_thread")]
#[serial(provider_env)]
async fn bootstrap_registers_static_dynamic_provider_models_before_returning() {
    let home = tempfile::tempdir().unwrap();
    let _groky_home = EnvGuard::set("GROKY_HOME", home.path());
    let _grok_home = EnvGuard::set("GROK_HOME", home.path());

    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(
            r#"
            [dynamic_provider.local]
            name = "Local Gateway"
            base_url = "https://gateway.example/v1"
            unauthenticated = true

            [dynamic_provider.local.models."openai/gpt-4o"]
            name = "Gateway GPT-4o"
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    let auth = Arc::new(AuthManager::new(home.path(), GrokComConfig::default()));

    let (_, models) = bootstrap(&cfg, &auth, None).unwrap();
    let entries = models.models();
    let entry = entries
        .get("local/openai/gpt-4o")
        .expect("static dynamic model must be visible when bootstrap returns");
    assert_eq!(entry.info.model, "openai/gpt-4o");
    assert_eq!(entry.info.base_url, "https://gateway.example/v1");
    assert!(models.provider_catalog().is_some());
}

#[tokio::test(flavor = "current_thread")]
#[serial(provider_env)]
async fn bootstrap_hydrates_fresh_same_origin_dynamic_cache() {
    let home = tempfile::tempdir().unwrap();
    let _groky_home = EnvGuard::set("GROKY_HOME", home.path());
    let _grok_home = EnvGuard::set("GROK_HOME", home.path());
    let config =
        DynamicProviderConfig::new("cached", "Cached Gateway", "https://gateway.example/v1")
            .unwrap();
    DynamicCache::new(home.path().join("dynamic_models_cache.json"))
        .store_provider(
            CachedProviderModels::new(
                &config,
                i64::MAX / 2,
                vec![CachedModel {
                    id: ModelId::new("cached-model").unwrap(),
                    name: Some("Cached Model".to_owned()),
                }],
            )
            .unwrap(),
        )
        .await
        .unwrap();

    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(
            r#"
            [dynamic_provider.cached]
            name = "Cached Gateway"
            base_url = "https://gateway.example/v1"
            unauthenticated = true
            discover = true
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    let auth = Arc::new(AuthManager::new(home.path(), GrokComConfig::default()));

    let (_, models) = bootstrap(&cfg, &auth, None).unwrap();
    assert!(
        models.models().contains_key("cached/cached-model"),
        "fresh applicable cache must be visible when bootstrap returns"
    );
}

#[tokio::test(flavor = "current_thread")]
#[serial(provider_env)]
async fn bootstrap_rejects_dynamic_catalog_id_collisions() {
    let home = tempfile::tempdir().unwrap();
    let _groky_home = EnvGuard::set("GROKY_HOME", home.path());
    let _grok_home = EnvGuard::set("GROK_HOME", home.path());

    let cfg = Config::new_from_toml_cfg(
        &toml::from_str(
            r#"
            [dynamic_provider.openai]
            name = "Shadow OpenAI"
            base_url = "https://gateway.example/v1"
            unauthenticated = true
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    let auth = Arc::new(AuthManager::new(home.path(), GrokComConfig::default()));

    let err = match bootstrap(&cfg, &auth, None) {
        Ok(_) => panic!("catalog collision must fail bootstrap"),
        Err(err) => err,
    };
    assert_eq!(err, "invalid dynamic provider configuration");
}
