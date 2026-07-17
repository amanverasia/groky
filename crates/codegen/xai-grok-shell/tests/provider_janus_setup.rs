//! Janus setup flow through the shell adapter and ACP surface
//! (plan 3 Task 9): `setup_janus` persists the secret-free provider config
//! plus optional key, health-checks with the resolved credential, runs
//! discovery, and reports Ready/Empty/Failed without ever leaking secrets.
//! Forced provider refresh re-runs dynamic discovery even with a fresh
//! cache.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use serial_test::serial;
use xai_grok_catalog::{ProviderId, SecretString};
use xai_grok_shell::agent::config::Config;
use xai_grok_shell::agent::models::ModelsManager;
use xai_grok_shell::agent::provider_catalog::{
    DYNAMIC_PROVIDERS_FILE, JanusSetupRequest, JanusSetupResult, ProviderCatalogAdapter,
};
use xai_grok_shell::auth::{AuthManager, GrokComConfig};
use xai_grok_shell::extensions::providers::{
    ProviderSurface, RefreshRequest, SetupJanusRequest, refresh_providers, setup_janus,
};
use xai_grok_test_support::{MockInferenceServer, MockModelEntry};

fn janus_id() -> ProviderId {
    ProviderId::new("janus").unwrap()
}

fn adapter(grok_home: &std::path::Path) -> Arc<ProviderCatalogAdapter> {
    Arc::new(ProviderCatalogAdapter::from_grok_home(
        grok_home.to_path_buf(),
    ))
}

fn setup_request(base_url: &str, api_key: Option<&str>) -> JanusSetupRequest {
    JanusSetupRequest {
        base_url: base_url.to_owned(),
        api_key: api_key.map(SecretString::new),
        allow_insecure_http: false,
    }
}

/// Fixture: ModelsManager + adapter over a temp `$GROK_HOME`, mirroring
/// `tests/provider_extension.rs`.
fn surface(tmp: &std::path::Path, adapter: Arc<ProviderCatalogAdapter>) -> ProviderSurface {
    let auth = Arc::new(AuthManager::new(tmp, GrokComConfig::default()));
    let mgr = ModelsManager::from_config(&Config::default(), None, auth).unwrap();
    mgr.set_provider_catalog(adapter);
    ProviderSurface {
        models_manager: mgr,
        grok_home: tmp.to_path_buf(),
        xai_configured: false,
    }
}

fn models_request_count(server: &MockInferenceServer) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path == "/v1/models")
        .count()
}

/// Happy path: setup persists the secret-free config file and the optional
/// key, health-checks and discovers with the stored bearer, publishes the
/// discovered models, and survives an adapter restart.
#[tokio::test(flavor = "multi_thread")]
async fn janus_setup_persists_base_and_optional_key_then_health_checks_and_discovers() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockInferenceServer::start_with_required_auth(
        vec![
            MockModelEntry::new("gpt-4o"),
            MockModelEntry::new("best-effort"),
        ],
        "sk-janus-test",
    )
    .await
    .unwrap();

    let adapter = adapter(tmp.path());
    let result = adapter
        .setup_janus(setup_request(&server.url(), Some("sk-janus-test")))
        .await
        .unwrap();
    assert_eq!(result, JanusSetupResult::Ready { model_count: 2 });

    // The key landed in the provider auth scope.
    assert_eq!(
        xai_grok_shell::auth::read_provider_api_key(tmp.path(), "janus").as_deref(),
        Some("sk-janus-test"),
    );

    // The layered snapshot carries the discovered models.
    let snapshot = adapter.snapshot();
    let provider = snapshot.catalog().provider_str("janus").unwrap();
    let ids: Vec<&str> = provider.models.iter().map(|m| m.id.as_str()).collect();
    assert!(
        ids.contains(&"gpt-4o") && ids.contains(&"best-effort"),
        "{ids:?}"
    );

    // Health then models were fetched, both with the stored bearer.
    let requests = server.requests();
    let health = requests
        .iter()
        .find(|r| r.method == "GET" && r.path == "/v1/health")
        .expect("setup must GET /v1/health");
    assert_eq!(
        health.authorization.as_deref(),
        Some("Bearer sk-janus-test")
    );
    let models = requests
        .iter()
        .find(|r| r.method == "GET" && r.path == "/v1/models")
        .expect("setup must GET /v1/models");
    assert_eq!(
        models.authorization.as_deref(),
        Some("Bearer sk-janus-test")
    );

    // Persistence is secret-free and re-registers Janus on restart.
    let persisted = std::fs::read_to_string(tmp.path().join(DYNAMIC_PROVIDERS_FILE)).unwrap();
    assert!(persisted.contains("janus") && persisted.contains(&server.url()));
    assert!(!persisted.contains("sk-janus-test"));
    let reborn = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    assert!(
        reborn.snapshot().catalog().provider_str("janus").is_some(),
        "restart must re-register the persisted Janus provider"
    );
    let event = reborn.refresh_dynamic(&janus_id()).await.unwrap();
    assert!(
        matches!(
            event,
            xai_grok_shell::agent::provider_catalog::ProviderCatalogEvent::DynamicRefreshComplete {
                model_count: 2,
                ..
            }
        ),
        "re-registered provider must be refreshable: {event:?}"
    );
}

/// A 401 health probe maps to the exact Janus auth message (no URL, no
/// secret); a healthy service with an empty model list maps to `Empty`.
#[tokio::test(flavor = "multi_thread")]
async fn janus_setup_reports_auth_and_empty_list_exactly() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockInferenceServer::start().await.unwrap();
    server.set_health(
        StatusCode::UNAUTHORIZED,
        serde_json::json!({"error": "unauthorized"}),
    );

    let adapter = adapter(tmp.path());
    let result = adapter
        .setup_janus(setup_request(&server.url(), Some("sk-janus-bad")))
        .await
        .unwrap();
    let JanusSetupResult::Failed {
        message,
        cached_models,
    } = result
    else {
        panic!("expected Failed, got {result:?}");
    };
    assert_eq!(
        message,
        "Janus rejected the API key. Update the Janus key in /providers."
    );
    assert_eq!(cached_models, 0);
    assert!(!message.contains(&server.url()) && !message.contains("sk-janus-bad"));

    // Healthy but empty model list → Empty.
    server.set_health(StatusCode::OK, serde_json::json!({"status": "ok"}));
    server.set_models(vec![]);
    let result = adapter
        .setup_janus(setup_request(&server.url(), Some("sk-janus-bad")))
        .await
        .unwrap();
    assert_eq!(result, JanusSetupResult::Empty);
}

/// The serialized ACP response after a keyed setup carries neither the key
/// value nor an `apiKey` field.
#[tokio::test(flavor = "multi_thread")]
#[serial(provider_env)]
async fn setup_janus_acp_response_is_secret_free() {
    const SECRET: &str = "sk-janus-acp-secret-4711";
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("gpt-4o")])
        .await
        .unwrap();

    let surface = surface(tmp.path(), adapter(tmp.path()));
    let response = setup_janus(
        &surface,
        SetupJanusRequest {
            base_url: server.url(),
            api_key: Some(SECRET.to_owned()),
            allow_insecure_http: false,
        },
    )
    .await
    .unwrap();
    let json = serde_json::to_string(&response).unwrap();
    assert!(!json.contains(SECRET), "response leaked the key: {json}");
    assert!(
        !json.contains("apiKey"),
        "response echoed the key field: {json}"
    );
    assert!(json.contains("\"state\":\"ready\""), "{json}");

    unsafe { std::env::remove_var("GROK_HOME") };
}

/// Omitting the key on setup leaves an existing stored key intact (and the
/// stored key authenticates the health/discovery round-trip).
#[tokio::test(flavor = "multi_thread")]
async fn omitted_key_leaves_stored_key_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockInferenceServer::start_with_required_auth(
        vec![MockModelEntry::new("gpt-4o")],
        "sk-existing",
    )
    .await
    .unwrap();
    xai_grok_shell::auth::store_provider_api_key(tmp.path(), "janus", "sk-existing").unwrap();

    let adapter = adapter(tmp.path());
    let result = adapter
        .setup_janus(setup_request(&server.url(), None))
        .await
        .unwrap();
    assert_eq!(result, JanusSetupResult::Ready { model_count: 1 });
    assert_eq!(
        xai_grok_shell::auth::read_provider_api_key(tmp.path(), "janus").as_deref(),
        Some("sk-existing"),
    );
}

/// `x.ai/providers/refresh { force: true }` re-runs dynamic discovery even
/// when the last-known-good cache is fresh.
#[tokio::test(flavor = "multi_thread")]
#[serial(provider_env)]
async fn forced_refresh_rehits_models_even_when_cache_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("gpt-4o")])
        .await
        .unwrap();

    let adapter = adapter(tmp.path());
    let result = adapter
        .setup_janus(setup_request(&server.url(), None))
        .await
        .unwrap();
    assert_eq!(result, JanusSetupResult::Ready { model_count: 1 });
    let baseline = models_request_count(&server);
    assert!(baseline >= 1);

    // Cache is fresh: a non-forced refresh must not re-discover.
    let surface = surface(tmp.path(), Arc::clone(&adapter));
    refresh_providers(&surface, RefreshRequest { force: false });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        models_request_count(&server),
        baseline,
        "non-forced refresh must respect the fresh dynamic cache"
    );

    // Forced refresh re-hits /v1/models unconditionally.
    refresh_providers(&surface, RefreshRequest { force: true });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if models_request_count(&server) > baseline {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "forced refresh never re-hit /v1/models"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    unsafe { std::env::remove_var("GROK_HOME") };
}
