//! Dynamic provider refresh orchestration through the shell adapter
//! (plan 3 Task 8): discovery publishes into the layered catalog snapshot,
//! offline refreshes fall back to the last-known-good cache, non-loopback
//! plain HTTP is rejected before any model becomes sampleable, credentials
//! never enter snapshots or serialized entries, and concurrent cache writes
//! do not lose updates.

use std::sync::Arc;

use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use xai_grok_catalog::{
    CachedModel, CachedProviderModels, DynamicCache, DynamicProviderConfig, HttpError, ModelId,
    ProviderId,
};
use xai_grok_shell::agent::config::{Config, resolve_credentials_with, sampling_config_for_model};
use xai_grok_shell::agent::models::ModelsManager;
use xai_grok_shell::agent::provider_catalog::{
    DYNAMIC_MODELS_CACHE_FILE, ProviderAdapterError, ProviderCatalogAdapter, ProviderCatalogEvent,
};
use xai_grok_shell::auth::{AuthManager, GrokComConfig};

fn provider_id(id: &str) -> ProviderId {
    ProviderId::new(id).unwrap()
}

/// An unauthenticated, discovery-enabled dynamic provider config.
fn dynamic_config(id: &str, base_url: &str) -> DynamicProviderConfig {
    let mut config = DynamicProviderConfig::new(id, format!("{id} gateway"), base_url).unwrap();
    config.unauthenticated = true;
    config.discover = true;
    config
}

async fn mock_models_server(bodies: &[(&str, Option<&str>)]) -> MockServer {
    let data: Vec<serde_json::Value> = bodies
        .iter()
        .map(|(id, name)| match name {
            Some(name) => serde_json::json!({"id": id, "name": name}),
            None => serde_json::json!({"id": id}),
        })
        .collect();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": data})))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
#[serial(provider_env)]
async fn refresh_publishes_discovered_models_without_changing_selection() {
    let tmp = tempfile::tempdir().unwrap();
    // Both vars so an ambient GROKY_HOME (which takes precedence) can't
    // leak the real home into the test; mirrors `GrokHomeFixture`.
    unsafe {
        std::env::set_var("GROKY_HOME", tmp.path());
        std::env::set_var("GROK_HOME", tmp.path());
    };

    let server = mock_models_server(&[("alpha", None), ("beta", Some("Beta"))]).await;
    let base_url = format!("{}/v1", server.uri());

    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    adapter
        .configure_dynamic(dynamic_config("localgw", &base_url))
        .unwrap();

    let cfg = Config::default();
    let auth = Arc::new(AuthManager::new(tmp.path(), GrokComConfig::default()));
    let mgr = ModelsManager::from_config(&cfg, None, auth).unwrap();
    mgr.set_provider_catalog(Arc::clone(&adapter));
    let selected_before = mgr.current_model_id();

    let event = adapter
        .refresh_dynamic(&provider_id("localgw"))
        .await
        .unwrap();
    assert_eq!(
        event,
        ProviderCatalogEvent::DynamicRefreshComplete {
            provider_id: provider_id("localgw"),
            model_count: 2,
            cached: false,
        }
    );

    // The layered snapshot carries the discovered models.
    let snapshot = adapter.snapshot();
    let provider = snapshot.catalog().provider_str("localgw").unwrap();
    let ids: Vec<&str> = provider.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["alpha", "beta"]);

    // Recomposition surfaces them as qualified entries without yanking the
    // current selection.
    mgr.rebuild_provider_models();
    let models = mgr.models();
    assert!(models.contains_key("localgw/alpha"));
    assert!(models.contains_key("localgw/beta"));
    assert_eq!(
        models["localgw/beta"].info.name.as_deref(),
        Some("Beta"),
        "discovered display name must survive composition"
    );
    assert_eq!(mgr.current_model_id(), selected_before);

    unsafe { std::env::remove_var("GROK_HOME") };
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_refresh_publishes_cached_models_and_warning() {
    let tmp = tempfile::tempdir().unwrap();

    // Pre-seed the last-known-good cache for the provider.
    let cache = DynamicCache::new(tmp.path().join(DYNAMIC_MODELS_CACHE_FILE));
    cache
        .store_provider(CachedProviderModels {
            provider_id: provider_id("localgw"),
            base_url: "http://127.0.0.1:9/v1".to_string(),
            fetched_at_unix: 1_721_088_000,
            models: vec![CachedModel {
                id: ModelId::new("cached-model").unwrap(),
                name: Some("Cached Model".to_string()),
            }],
        })
        .await
        .unwrap();

    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    // Port 9 (discard) on loopback: connection refused, i.e. offline.
    adapter
        .configure_dynamic(dynamic_config("localgw", "http://127.0.0.1:9/v1"))
        .unwrap();

    let event = adapter
        .refresh_dynamic(&provider_id("localgw"))
        .await
        .unwrap();
    assert_eq!(
        event,
        ProviderCatalogEvent::DynamicRefreshComplete {
            provider_id: provider_id("localgw"),
            model_count: 1,
            cached: true,
        }
    );

    let snapshot = adapter.snapshot();
    let provider = snapshot.catalog().provider_str("localgw").unwrap();
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].id.as_str(), "cached-model");
    assert_eq!(provider.models[0].name, "Cached Model");
}

#[tokio::test(flavor = "multi_thread")]
async fn non_loopback_http_is_rejected_before_model_becomes_sampleable() {
    let tmp = tempfile::tempdir().unwrap();
    let adapter = ProviderCatalogAdapter::from_grok_home(tmp.path().to_path_buf());

    let mut config = dynamic_config("lan", "http://192.168.1.20:20128/v1");
    // A static model that would become sampleable if configuration passed.
    config.models.insert(
        ModelId::new("lan-model").unwrap(),
        xai_grok_catalog::ModelPatch {
            id: ModelId::new("lan-model").unwrap(),
            name: None,
            protocol: None,
            context_window: None,
            reasoning: None,
            cost: None,
            exclude: false,
        },
    );

    let err = adapter.configure_dynamic(config).unwrap_err();
    assert!(
        matches!(
            err,
            ProviderAdapterError::Endpoint(HttpError::InsecureHttpDenied)
        ),
        "expected InsecureHttpDenied, got: {err:?}"
    );

    // Nothing was registered or published: no provider, no model.
    let snapshot = adapter.snapshot();
    assert!(snapshot.catalog().provider_str("lan").is_none());
    assert!(
        adapter.refresh_dynamic(&provider_id("lan")).await.is_err(),
        "a rejected provider must not be refreshable"
    );
}

/// Secret firewall across the whole handoff: the snapshot and the composed
/// `ModelEntry` are secret-free; the credential appears only in the resolved
/// `SamplerConfig.api_key`, and its `Debug` rendering stays redacted.
#[tokio::test(flavor = "multi_thread")]
async fn sampler_handoff_carries_key_without_leaking_it() {
    const SECRET: &str = "sk-dynamic-secret-4711";
    let tmp = tempfile::tempdir().unwrap();
    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));

    // Authenticated provider with one static model; statics publish at
    // configure time without any network I/O.
    let mut config: DynamicProviderConfig = toml::from_str(
        r#"
        id = "localgw"
        name = "Local Gateway"
        base_url = "https://gateway.example/v1"

        [models.gpt-mini]
        name = "GPT Mini"
        "#,
    )
    .unwrap();
    config.discover = false;
    adapter.configure_dynamic(config).unwrap();
    adapter.set_session_key(&provider_id("localgw"), SECRET.to_string());

    // Snapshot layer is secret-free.
    let snapshot = adapter.snapshot();
    let catalog_json = serde_json::to_string(snapshot.catalog()).unwrap();
    assert!(!catalog_json.contains(SECRET));

    // Composed entry carries provider context but no credential.
    let entries = adapter.configured_model_entries(&Config::default());
    let entry = entries
        .get("localgw/gpt-mini")
        .expect("session-keyed dynamic provider composes its static model");
    assert!(entry.api_key.is_none());
    let entry_json = serde_json::to_string(entry).unwrap();
    assert!(!entry_json.contains(SECRET));

    // Credential resolution at the seam injects the key into the sampler
    // config; base_url and backend flow from the dynamic provider entry.
    let resolved = resolve_credentials_with(
        entry,
        None,
        || None,
        |pid| {
            adapter
                .credential_for(&ProviderId::new(pid).unwrap())
                .map(|secret| secret.expose().to_string())
        },
        |_| None,
    );
    assert_eq!(resolved.api_key.as_deref(), Some(SECRET));
    let sampling = sampling_config_for_model(entry, resolved, None, None, None, None);
    assert_eq!(sampling.api_key.as_deref(), Some(SECRET));
    assert_eq!(sampling.base_url, "https://gateway.example/v1");
    assert_eq!(sampling.model, "gpt-mini");
    let debug = format!("{sampling:?}");
    assert!(!debug.contains(SECRET), "Debug must redact: {debug}");
}

/// A 401 with nothing to fall back to fails with concise auth guidance
/// that leaks neither the endpoint URL nor any bearer text.
#[tokio::test(flavor = "multi_thread")]
async fn auth_failure_without_fallback_reports_secret_free_guidance() {
    const SECRET: &str = "sk-rejected-secret-401";
    let tmp = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    // Authenticated provider, no cache, no static models.
    let mut config = dynamic_config("localgw", &format!("{}/v1", server.uri()));
    config.unauthenticated = false;
    adapter.configure_dynamic(config).unwrap();
    adapter.set_session_key(&provider_id("localgw"), SECRET.to_string());

    let event = adapter
        .refresh_dynamic(&provider_id("localgw"))
        .await
        .unwrap();
    let ProviderCatalogEvent::DynamicRefreshFailed {
        provider_id: failed_id,
        message,
    } = event
    else {
        panic!("expected DynamicRefreshFailed, got: {event:?}");
    };
    assert_eq!(failed_id, provider_id("localgw"));
    let lower = message.to_lowercase();
    assert!(
        lower.contains("authentication") && lower.contains("key"),
        "message must give auth/key guidance: {message}"
    );
    assert!(
        !message.contains(&server.uri()) && !message.contains("/v1/models"),
        "auth failure must not embed the endpoint URL: {message}"
    );
    assert!(
        !message.contains(SECRET) && !lower.contains("sk-") && !lower.contains("bearer"),
        "auth failure must not embed bearer text: {message}"
    );
}

/// A 401 with a warm last-known-good cache serves the cached models.
#[tokio::test(flavor = "multi_thread")]
async fn auth_failure_with_warm_cache_falls_back_to_cached_models() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let cache = DynamicCache::new(tmp.path().join(DYNAMIC_MODELS_CACHE_FILE));
    cache
        .store_provider(CachedProviderModels {
            provider_id: provider_id("localgw"),
            base_url: format!("{}/v1", server.uri()),
            fetched_at_unix: 1_721_088_000,
            models: vec![CachedModel {
                id: ModelId::new("cached-model").unwrap(),
                name: None,
            }],
        })
        .await
        .unwrap();

    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    let mut config = dynamic_config("localgw", &format!("{}/v1", server.uri()));
    config.unauthenticated = false;
    adapter.configure_dynamic(config).unwrap();

    let event = adapter
        .refresh_dynamic(&provider_id("localgw"))
        .await
        .unwrap();
    assert_eq!(
        event,
        ProviderCatalogEvent::DynamicRefreshComplete {
            provider_id: provider_id("localgw"),
            model_count: 1,
            cached: true,
        }
    );
    let snapshot = adapter.snapshot();
    let provider = snapshot.catalog().provider_str("localgw").unwrap();
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].id.as_str(), "cached-model");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_refreshes_do_not_lose_cache_updates() {
    let tmp = tempfile::tempdir().unwrap();
    let server_a = mock_models_server(&[("model-a", None)]).await;
    let server_b = mock_models_server(&[("model-b", None)]).await;

    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    adapter
        .configure_dynamic(dynamic_config("gw-a", &format!("{}/v1", server_a.uri())))
        .unwrap();
    adapter
        .configure_dynamic(dynamic_config("gw-b", &format!("{}/v1", server_b.uri())))
        .unwrap();

    let id_a = provider_id("gw-a");
    let id_b = provider_id("gw-b");
    let (a, b) = tokio::join!(
        adapter.refresh_dynamic(&id_a),
        adapter.refresh_dynamic(&id_b),
    );
    assert!(matches!(
        a.unwrap(),
        ProviderCatalogEvent::DynamicRefreshComplete {
            cached: false,
            model_count: 1,
            ..
        }
    ));
    assert!(matches!(
        b.unwrap(),
        ProviderCatalogEvent::DynamicRefreshComplete {
            cached: false,
            model_count: 1,
            ..
        }
    ));

    // Both providers must survive in the on-disk cache (no lost update).
    let cache = DynamicCache::new(tmp.path().join(DYNAMIC_MODELS_CACHE_FILE));
    let file = cache.load().await.unwrap();
    assert_eq!(file.providers.len(), 2, "a concurrent store was lost");
    assert!(file.provider(&provider_id("gw-a")).is_some());
    assert!(file.provider(&provider_id("gw-b")).is_some());
}
