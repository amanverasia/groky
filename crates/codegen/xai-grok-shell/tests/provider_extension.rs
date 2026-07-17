//! Provider management ACP surface (plan Task 10).
//!
//! Drives the `extensions::providers` handlers directly against a real
//! `ModelsManager` + `ProviderCatalogAdapter` fixture (temp `$GROK_HOME`,
//! on-disk catalog cache) rather than a full `MvpAgent`, which is not
//! constructible in integration tests. Broadcasts are captured through a
//! bare `AcpAgentGatewaySender` channel.

use std::sync::Arc;

use agent_client_protocol as acp;
use serial_test::serial;
use xai_acp_lib::{AcpAgentGatewaySender, AcpClientMessage};
use xai_grok_catalog::{
    CATALOG_SCHEMA_VERSION, CatalogCache, CatalogManager, CatalogModel, CatalogProvider, ModelId,
    NormalizedCatalog, Protocol, ProviderAvailability, ProviderId, RefreshOutcome, encode_cache,
};
use xai_grok_shell::agent::config::Config;
use xai_grok_shell::agent::models::ModelsManager;
use xai_grok_shell::agent::provider_catalog::{
    PROVIDER_CATALOG_CACHE_FILE, PROVIDER_CATALOG_SOURCE_URL, ProviderCatalogAdapter,
};
use xai_grok_shell::auth::{AuthManager, GrokComConfig};
use xai_grok_shell::extensions::providers::{
    ClearProviderKeyRequest, ProviderSurface, RefreshRequest, StoreProviderKeyRequest,
    clear_provider_key, list_providers, parse_refresh_request, refresh_providers,
    store_provider_key,
};

fn model(id: &str, name: &str) -> CatalogModel {
    CatalogModel {
        id: ModelId::new(id).unwrap(),
        name: name.to_string(),
        protocol: Protocol::ChatCompletions,
        context_window: Some(400_000),
        reasoning: true,
        cost: None,
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
                vec![model("gpt-5", "GPT-5")],
            ),
            provider("emptyco", "EmptyCo", "EMPTYCO_API_KEY", vec![]),
            provider("xai", "xAI", "XAI_API_KEY", vec![model("grok-4", "Grok 4")]),
        ],
    }
}

fn write_cache(dir: &std::path::Path, source_url: &str, fetched_at: chrono::DateTime<chrono::Utc>) {
    let cache = CatalogCache {
        schema_version: CATALOG_SCHEMA_VERSION,
        source_url: source_url.to_string(),
        fetched_at,
        etag: None,
        last_modified: None,
        catalog: sample_catalog(),
    };
    std::fs::write(
        dir.join(PROVIDER_CATALOG_CACHE_FILE),
        encode_cache(&cache).unwrap(),
    )
    .unwrap();
}

/// Fixture: ModelsManager (no gateway yet) + adapter over a temp GROK_HOME.
fn surface(
    tmp: &std::path::Path,
    adapter: Arc<ProviderCatalogAdapter>,
    xai_configured: bool,
) -> ProviderSurface {
    let auth = Arc::new(AuthManager::new(tmp, GrokComConfig::default()));
    let mgr = ModelsManager::from_config(&Config::default(), None, auth).unwrap();
    mgr.set_provider_catalog(adapter);
    ProviderSurface {
        models_manager: mgr,
        grok_home: tmp.to_path_buf(),
        xai_configured,
    }
}

fn clear_provider_env() {
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("EMPTYCO_API_KEY");
        std::env::remove_var("XAI_API_KEY");
    }
}

/// The `xai` provider is managed by the built-in login flow: the generic key
/// management surface must reject it with `invalid_params`, even though the
/// upstream catalog carries an `xai` row.
#[tokio::test]
#[serial(provider_env)]
async fn xai_provider_key_management_is_rejected() {
    clear_provider_env();
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    write_cache(tmp.path(), PROVIDER_CATALOG_SOURCE_URL, chrono::Utc::now());
    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    let surface = surface(tmp.path(), adapter, true);

    let err = store_provider_key(
        &surface,
        StoreProviderKeyRequest {
            provider_id: "xai".into(),
            api_key: "sk-x".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, agent_client_protocol::ErrorCode::InvalidParams);
    assert!(
        xai_grok_shell::auth::read_provider_api_key(tmp.path(), "xai").is_none(),
        "rejected store must not write an xai key scope"
    );

    let err = clear_provider_key(
        &surface,
        ClearProviderKeyRequest {
            provider_id: "xai".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, agent_client_protocol::ErrorCode::InvalidParams);
}

/// `clear_key` must validate the provider id BEFORE clearing anything or
/// broadcasting: unknown and blank ids map to `invalid_params` with no
/// side effects.
#[tokio::test]
#[serial(provider_env)]
async fn clear_key_validates_provider_before_side_effects() {
    clear_provider_env();
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    write_cache(tmp.path(), PROVIDER_CATALOG_SOURCE_URL, chrono::Utc::now());
    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    let surface = surface(tmp.path(), adapter, true);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    surface
        .models_manager
        .set_gateway(AcpAgentGatewaySender::new(tx));

    let err = clear_provider_key(
        &surface,
        ClearProviderKeyRequest {
            provider_id: "nonesuch".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, agent_client_protocol::ErrorCode::InvalidParams);

    let err = clear_provider_key(
        &surface,
        ClearProviderKeyRequest {
            provider_id: "  ".into(),
        },
    )
    .unwrap_err();
    assert_eq!(
        err.code,
        agent_client_protocol::ErrorCode::InvalidParams,
        "blank providerId must be invalid_params, not internal_error"
    );

    assert!(
        rx.try_recv().is_err(),
        "rejected clear_key must not broadcast any update"
    );
}

#[tokio::test]
#[serial(provider_env)]
async fn provider_list_is_secret_free_and_reports_all_states() {
    clear_provider_env();
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    // Fresh cache: no background refresh may start (would hit the network).
    write_cache(tmp.path(), PROVIDER_CATALOG_SOURCE_URL, chrono::Utc::now());
    xai_grok_shell::auth::store_provider_api_key(tmp.path(), "openai", "sk-secret-value").unwrap();

    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    let surface = surface(tmp.path(), adapter, true);

    let response = list_providers(&surface).unwrap();
    assert!(!response.refresh_started);
    assert_eq!(response.refresh_status, "fresh");

    let status_of = |id: &str| {
        response
            .providers
            .iter()
            .find(|p| p.provider_id == id)
            .unwrap_or_else(|| panic!("provider {id} present"))
            .status
    };
    assert_eq!(status_of("xai"), ProviderAvailability::Configured);
    assert_eq!(status_of("openai"), ProviderAvailability::Configured);
    assert_eq!(status_of("anthropic"), ProviderAvailability::MissingKey);
    assert_eq!(status_of("emptyco"), ProviderAvailability::Unavailable);
    // xAI row is pinned first.
    assert_eq!(response.providers[0].provider_id, "xai");

    let json = serde_json::to_string(&response).unwrap();
    assert!(!json.contains("sk-"));
    assert!(!json.contains("api_key"));
    assert!(!json.contains("apiKey"));
}

#[tokio::test]
#[serial(provider_env)]
async fn store_key_rebuilds_models_and_broadcasts_both_updates() {
    clear_provider_env();
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    write_cache(tmp.path(), PROVIDER_CATALOG_SOURCE_URL, chrono::Utc::now());

    let adapter = Arc::new(ProviderCatalogAdapter::from_grok_home(
        tmp.path().to_path_buf(),
    ));
    let surface = surface(tmp.path(), adapter, true);
    assert!(
        !surface.models_manager.models().contains_key("openai/gpt-5"),
        "openai must start unconfigured"
    );

    // Attach a capture gateway AFTER setup so only the store broadcasts land.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    surface
        .models_manager
        .set_gateway(AcpAgentGatewaySender::new(tx));

    let response = store_provider_key(
        &surface,
        StoreProviderKeyRequest {
            provider_id: "openai".into(),
            api_key: "sk-test-secret".into(),
        },
    )
    .unwrap();
    assert_eq!(response.status, ProviderAvailability::Configured);

    assert!(
        surface.models_manager.models().contains_key("openai/gpt-5"),
        "stored key must rebuild model availability"
    );

    let mut methods = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let AcpClientMessage::ExtNotification(args) = msg {
            methods.push(args.request.method.to_string());
        }
    }
    assert_eq!(methods, ["x.ai/providers/update", "x.ai/models/update"]);

    // Blank keys and unknown providers are rejected without writes.
    assert!(
        store_provider_key(
            &surface,
            StoreProviderKeyRequest {
                provider_id: "openai".into(),
                api_key: "   ".into(),
            },
        )
        .is_err()
    );
    assert!(
        store_provider_key(
            &surface,
            StoreProviderKeyRequest {
                provider_id: "nonesuch".into(),
                api_key: "sk-x".into(),
            },
        )
        .is_err()
    );
    // Unavailable providers (no supported models) cannot take keys.
    assert!(
        store_provider_key(
            &surface,
            StoreProviderKeyRequest {
                provider_id: "emptyco".into(),
                api_key: "sk-x".into(),
            },
        )
        .is_err()
    );

    // clear_key removes only that provider scope and reverts availability.
    let cleared = clear_provider_key(
        &surface,
        ClearProviderKeyRequest {
            provider_id: "openai".into(),
        },
    )
    .unwrap();
    assert_eq!(cleared.status, ProviderAvailability::MissingKey);
    assert!(!surface.models_manager.models().contains_key("openai/gpt-5"));
}

#[tokio::test]
#[serial(provider_env)]
async fn opening_surface_returns_immediately_and_starts_one_stale_refresh() {
    clear_provider_env();
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    // Stale cache pointed at an unreachable origin: refresh must coalesce and
    // fail without touching the real network.
    let source_url = "http://127.0.0.1:9/api.json";
    write_cache(
        tmp.path(),
        source_url,
        chrono::Utc::now() - chrono::Duration::days(7),
    );
    let manager = CatalogManager::new(
        tmp.path().join(PROVIDER_CATALOG_CACHE_FILE),
        source_url.to_string(),
    );
    let adapter = Arc::new(ProviderCatalogAdapter::new(
        manager,
        tmp.path().to_path_buf(),
    ));
    let surface = surface(tmp.path(), adapter, false);

    // Single-threaded runtime: the spawned refresh cannot run between these
    // two calls, so the AtomicBool coalescing is observable deterministically.
    let first = list_providers(&surface).unwrap();
    assert!(first.refresh_started);
    assert_eq!(first.refresh_status, "refreshing");

    let second = list_providers(&surface).unwrap();
    assert!(!second.refresh_started, "in-flight refresh must coalesce");
    assert_eq!(second.refresh_status, "refreshing");

    // Explicit refresh also coalesces while one is in flight.
    let explicit = refresh_providers(&surface, RefreshRequest { force: true });
    assert!(!explicit.started);
}

/// Non-forced (picker-open) refresh defers staleness entirely to the catalog
/// manager: the request claims the coalescing slot, but with a fresh cache
/// the background task's `refresh_if_stale` performs no network fetch. This
/// must hold even though the snapshot's `RefreshStatus` is frozen at load
/// time (it never decays Fresh→Stale in a long-lived process), so the
/// handler must NOT gate on the snapshot status itself.
#[tokio::test]
#[serial(provider_env)]
async fn refresh_respects_staleness_gate_unless_forced() {
    clear_provider_env();
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    // Fresh cache pointed at an unreachable origin: any real fetch would
    // error ("connection failed"), so an Ok(Fresh) outcome proves no
    // network I/O happened.
    let source_url = "http://127.0.0.1:9/api.json";
    write_cache(tmp.path(), source_url, chrono::Utc::now());
    let manager = CatalogManager::new(
        tmp.path().join(PROVIDER_CATALOG_CACHE_FILE),
        source_url.to_string(),
    );
    let adapter = Arc::new(ProviderCatalogAdapter::new(
        manager,
        tmp.path().to_path_buf(),
    ));
    let surface = surface(tmp.path(), Arc::clone(&adapter), false);

    // Non-forced must still start (the manager, not the frozen snapshot
    // status, is the staleness authority)...
    let gated = refresh_providers(&surface, RefreshRequest { force: false });
    assert!(
        gated.started,
        "non-forced refresh must spawn; staleness is decided by the manager"
    );
    // ...and the spawned task must complete without wedging the slot.
    for _ in 0..100 {
        if !adapter.refresh_in_flight() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(!adapter.refresh_in_flight(), "refresh slot must be released");
    // The manager-side gate is what the non-forced task runs: with a fresh
    // cache it returns Fresh without touching the (unreachable) network.
    assert!(matches!(
        adapter.refresh_if_stale().await.unwrap(),
        RefreshOutcome::Fresh
    ));

    let forced = refresh_providers(&surface, RefreshRequest { force: true });
    assert!(forced.started, "explicit refresh must run unconditionally");
}

/// ACP clients may omit params entirely (`params: null`); the refresh
/// handler must treat that as the default, non-forced request rather than
/// failing with invalid_params.
#[test]
fn refresh_accepts_null_params_as_non_forced() {
    let null_params = serde_json::value::to_raw_value(&serde_json::Value::Null).unwrap();
    let args = acp::ExtRequest::new("x.ai/providers/refresh", null_params.into());
    let req = parse_refresh_request(&args).expect("null params must be accepted");
    assert!(!req.force);

    let empty = serde_json::value::to_raw_value(&serde_json::json!({})).unwrap();
    let args = acp::ExtRequest::new("x.ai/providers/refresh", empty.into());
    assert!(!parse_refresh_request(&args).unwrap().force);

    let forced = serde_json::value::to_raw_value(&serde_json::json!({ "force": true })).unwrap();
    let args = acp::ExtRequest::new("x.ai/providers/refresh", forced.into());
    assert!(parse_refresh_request(&args).unwrap().force);
}

/// Wire backward compatibility: old clients send `{}`, which must parse as
/// a non-forced (staleness-gated) refresh.
#[test]
fn refresh_request_defaults_to_non_forced() {
    let req: RefreshRequest = serde_json::from_str("{}").unwrap();
    assert!(!req.force);
    let req: RefreshRequest = serde_json::from_str(r#"{"force":true}"#).unwrap();
    assert!(req.force);
}

/// A stale cache must still allow a non-forced (picker-open) refresh.
#[tokio::test]
#[serial(provider_env)]
async fn stale_cache_allows_non_forced_refresh() {
    clear_provider_env();
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("GROK_HOME", tmp.path()) };
    let source_url = "http://127.0.0.1:9/api.json";
    write_cache(
        tmp.path(),
        source_url,
        chrono::Utc::now() - chrono::Duration::days(7),
    );
    let manager = CatalogManager::new(
        tmp.path().join(PROVIDER_CATALOG_CACHE_FILE),
        source_url.to_string(),
    );
    let adapter = Arc::new(ProviderCatalogAdapter::new(
        manager,
        tmp.path().to_path_buf(),
    ));
    let surface = surface(tmp.path(), adapter, false);

    let gated = refresh_providers(&surface, RefreshRequest { force: false });
    assert!(gated.started, "stale cache: picker-open refresh must start");
}
