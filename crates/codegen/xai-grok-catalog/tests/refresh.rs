//! Integration tests for the 24-hour conditional refresh and secret-free
//! atomic catalog cache.

use std::time::Duration;

use chrono::{DateTime, Utc};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use xai_grok_catalog::{
    CATALOG_SCHEMA_VERSION, CatalogCache, CatalogError, CatalogManager, CatalogModel,
    CatalogProvider, DEFAULT_REFRESH_INTERVAL, ModelId, NormalizedCatalog, ProviderId,
    RefreshLimits, RefreshOutcome, encode_cache,
};

fn sample_catalog() -> NormalizedCatalog {
    NormalizedCatalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        providers: vec![CatalogProvider {
            id: ProviderId::new("cachedprov").unwrap(),
            name: "Cached Provider".to_string(),
            api_base_url: "https://api.cached.example/v1".to_string(),
            env_vars: vec!["CACHED_API_KEY".to_string()],
            unauthenticated: false,
            models: vec![CatalogModel {
                id: ModelId::new("cached-model").unwrap(),
                name: "Cached Model".to_string(),
                protocol: xai_grok_catalog::Protocol::ChatCompletions,
                context_window: Some(8192),
                reasoning: false,
                cost: None,
            }],
        }],
    }
}

fn stale_time() -> DateTime<Utc> {
    Utc::now() - chrono::Duration::days(2)
}

fn stale_cache(etag: &str) -> CatalogCache {
    CatalogCache {
        schema_version: CATALOG_SCHEMA_VERSION,
        source_url: String::new(), // filled in by manager_with_cache
        fetched_at: stale_time(),
        etag: Some(etag.to_string()),
        last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string()),
        catalog: sample_catalog(),
    }
}

fn sample_cache() -> CatalogCache {
    CatalogCache {
        source_url: "https://models.dev/api.json".to_string(),
        ..stale_cache("catalog-v1")
    }
}

/// Writes `cache` beside a leaked tempdir and builds a manager pointed at
/// `<server>/api.json`. The tempdir is intentionally leaked so the cache
/// file outlives the returned manager.
fn manager_with_cache(server: &MockServer, mut cache: CatalogCache) -> CatalogManager {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("catalog-cache.json");
    std::mem::forget(dir);
    let source_url = format!("{}/api.json", server.uri());
    cache.source_url = source_url.clone();
    std::fs::write(&cache_path, encode_cache(&cache).unwrap()).unwrap();
    CatalogManager::new(cache_path, source_url)
}

/// Manager with a valid stale cache whose source responds 200 with `body`.
async fn manager_with_valid_cache_and_response(body: &[u8]) -> CatalogManager {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api.json"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
        .mount(&server)
        .await;
    let manager = manager_with_cache(&server, stale_cache("catalog-v1"));
    std::mem::forget(server); // keep the mock alive for the manager's lifetime
    manager
}

#[tokio::test]
async fn stale_cache_sends_validators_and_304_renews_timestamp() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api.json"))
        .and(header("if-none-match", "catalog-v1"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;
    let manager = manager_with_cache(&server, stale_cache("catalog-v1"));
    let before = manager.snapshot();
    assert_eq!(
        manager.refresh_if_stale().await.unwrap(),
        RefreshOutcome::NotModified
    );
    assert_eq!(manager.snapshot(), before);
    assert!(manager.cache_metadata().unwrap().fetched_at > stale_time());
}

#[tokio::test]
async fn fresh_cache_skips_the_network_entirely() {
    let server = MockServer::start().await;
    // No mock mounted: any request would 404 and fail a refresh.
    let fresh = CatalogCache {
        fetched_at: Utc::now(),
        ..stale_cache("catalog-v1")
    };
    let manager = manager_with_cache(&server, fresh);
    assert_eq!(
        manager.refresh_if_stale().await.unwrap(),
        RefreshOutcome::Fresh
    );
}

#[tokio::test]
async fn invalid_response_keeps_last_known_good_snapshot_and_cache() {
    let manager = manager_with_valid_cache_and_response(br#"{"broken":true}"#).await;
    let before_snapshot = manager.snapshot();
    let before_bytes = std::fs::read(manager.cache_path()).unwrap();
    assert!(matches!(
        manager.refresh().await,
        Err(CatalogError::InvalidDocument(_))
    ));
    assert_eq!(manager.snapshot(), before_snapshot);
    assert_eq!(std::fs::read(manager.cache_path()).unwrap(), before_bytes);
}

#[tokio::test]
async fn valid_200_updates_snapshot_and_rewrites_cache_atomically() {
    let body = br#"{
        "freshprov": {
            "name": "Fresh Provider",
            "api": "https://api.fresh.example/v1",
            "env": ["FRESH_API_KEY"],
            "npm": "@ai-sdk/openai-compatible",
            "models": {"fresh-model": {"name": "Fresh Model"}}
        }
    }"#;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(body.to_vec())
                .insert_header("etag", "catalog-v2"),
        )
        .mount(&server)
        .await;
    let manager = manager_with_cache(&server, stale_cache("catalog-v1"));
    assert_eq!(
        manager.refresh_if_stale().await.unwrap(),
        RefreshOutcome::Updated
    );
    let snapshot = manager.snapshot();
    assert!(snapshot.catalog().provider_str("freshprov").is_some());
    let meta = manager.cache_metadata().unwrap();
    assert_eq!(meta.etag.as_deref(), Some("catalog-v2"));
    // The rewritten cache on disk parses and carries the new catalog.
    let text = std::fs::read_to_string(manager.cache_path()).unwrap();
    assert!(text.contains("freshprov"));
    assert!(text.contains("catalog-v2"));
}

#[tokio::test]
async fn timeout_keeps_last_known_good_snapshot() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api.json"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("catalog-cache.json");
    let source_url = format!("{}/api.json", server.uri());
    let mut cache = stale_cache("catalog-v1");
    cache.source_url = source_url.clone();
    std::fs::write(&cache_path, encode_cache(&cache).unwrap()).unwrap();
    let manager = CatalogManager::with_limits(
        cache_path,
        source_url,
        RefreshLimits {
            total_timeout: Duration::from_millis(200),
            ..RefreshLimits::default()
        },
    );
    let before = manager.snapshot();
    assert!(matches!(
        manager.refresh().await,
        Err(CatalogError::Fetch(_))
    ));
    assert_eq!(manager.snapshot(), before);
}

#[tokio::test]
async fn oversized_response_is_rejected_and_snapshot_kept() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api.json"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 4096]))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("catalog-cache.json");
    let source_url = format!("{}/api.json", server.uri());
    let mut cache = stale_cache("catalog-v1");
    cache.source_url = source_url.clone();
    std::fs::write(&cache_path, encode_cache(&cache).unwrap()).unwrap();
    let manager = CatalogManager::with_limits(
        cache_path,
        source_url,
        RefreshLimits {
            max_response_bytes: 1024,
            ..RefreshLimits::default()
        },
    );
    let before = manager.snapshot();
    assert!(matches!(
        manager.refresh().await,
        Err(CatalogError::LimitExceeded { .. })
    ));
    assert_eq!(manager.snapshot(), before);
}

#[test]
fn cache_json_has_schema_source_timestamp_validators_and_no_secrets() {
    let bytes = encode_cache(&sample_cache()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for key in [
        "schema_version",
        "source_url",
        "fetched_at",
        "etag",
        "last_modified",
        "catalog",
    ] {
        assert!(text.contains(key), "cache JSON must contain {key}");
    }
    assert!(!text.contains("api_key"));
    assert!(!text.contains("authorization"));
}

#[test]
fn default_refresh_interval_is_exactly_24_hours() {
    assert_eq!(DEFAULT_REFRESH_INTERVAL, Duration::from_secs(24 * 60 * 60));
}

#[test]
fn missing_cache_falls_back_to_the_embedded_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let manager = CatalogManager::new(
        dir.path().join("catalog-cache.json"),
        "https://models.dev/api.json".to_string(),
    );
    let snapshot = manager.snapshot();
    assert!(snapshot.catalog().provider_str("xai").is_some());
    assert!(manager.cache_metadata().is_none());
}
