//! Bounded, conditional catalog refresh with last-known-good fallback.
//!
//! [`CatalogManager`] serves immutable [`CatalogSnapshot`]s from an
//! `ArcSwap`: embedded catalog first, overlaid by a valid on-disk cache
//! regardless of age; age only decides whether a refresh is attempted.
//! Every failure keeps the prior in-memory snapshot and on-disk cache.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwap;
use chrono::Utc;

use crate::cache::{CatalogCache, decode_cache, write_cache_atomic};
use crate::types::{
    CATALOG_SCHEMA_VERSION, CatalogError, CatalogModel, ModelId, NormalizationLimits,
    NormalizedCatalog, ProviderId,
};

/// How often the catalog is refreshed by default: exactly 24 hours.
pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Default maximum accepted response size: 8 MiB, enforced while streaming.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Network bounds for catalog refresh requests.
#[derive(Clone, Debug)]
pub struct RefreshLimits {
    /// TCP connect timeout.
    pub connect_timeout: Duration,
    /// Total request timeout.
    pub total_timeout: Duration,
    /// Maximum response size, enforced while streaming.
    pub max_response_bytes: usize,
}

impl Default for RefreshLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_secs(8),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

/// Result of a refresh attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The cache was fresh; no request was made.
    Fresh,
    /// The origin returned 304; validators renewed, catalog unchanged.
    NotModified,
    /// A new catalog was fetched, validated, stored, and cached.
    Updated,
}

/// Freshness of the catalog behind a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefreshStatus {
    /// Cache younger than the refresh interval.
    Fresh,
    /// No cache, or cache older than the refresh interval.
    Stale,
    /// A refresh is in flight.
    Refreshing,
    /// Serving cached data after a refresh failure; concise status text only.
    CachedAfterError(String),
}

/// Immutable view of the effective catalog plus its bundled fallback.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogSnapshot {
    effective: Arc<NormalizedCatalog>,
    bundled: Arc<NormalizedCatalog>,
    status: RefreshStatus,
}

impl CatalogSnapshot {
    /// The effective catalog (cache overlay when valid, embedded otherwise).
    pub fn catalog(&self) -> &NormalizedCatalog {
        &self.effective
    }

    /// Looks up a model within a provider.
    pub fn model(&self, provider_id: &ProviderId, model_id: &ModelId) -> Option<&CatalogModel> {
        self.effective.provider(provider_id)?.model(model_id)
    }

    /// Finds a model by its complete ID across the effective layers and the
    /// bundled snapshot. The ID is matched exactly; a slash prefix is never
    /// stripped.
    pub fn bundled_model_by_exact_id(&self, id: &ModelId) -> Option<&CatalogModel> {
        self.effective
            .providers
            .iter()
            .chain(self.bundled.providers.iter())
            .find_map(|provider| provider.model(id))
    }

    /// Freshness of this snapshot.
    pub fn status(&self) -> &RefreshStatus {
        &self.status
    }

    /// Copy-on-write snapshot with `provider`'s entry replaced (or appended)
    /// in the effective catalog.
    ///
    /// This is the dynamic-provider layer: it sits on top of the
    /// embedded+cache catalog and carries a runtime-discovered (or
    /// statically declared) model list. The receiver is untouched; indexes
    /// are positional lookups over `providers`, so upserting the entry is a
    /// complete rebuild. Snapshots stay fully secret-free: `provider` holds
    /// only IDs, names, URLs, and env var *names*.
    pub fn with_dynamic_models(
        &self,
        provider: crate::types::CatalogProvider,
    ) -> Arc<CatalogSnapshot> {
        let mut catalog = (*self.effective).clone();
        match catalog
            .providers
            .iter_mut()
            .find(|existing| existing.id == provider.id)
        {
            Some(existing) => *existing = provider,
            None => catalog.providers.push(provider),
        }
        Arc::new(CatalogSnapshot {
            effective: Arc::new(catalog),
            bundled: Arc::clone(&self.bundled),
            status: self.status.clone(),
        })
    }
}

/// Secret-free metadata about the on-disk cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheMetadata {
    /// When the catalog was last fetched or revalidated.
    pub fetched_at: chrono::DateTime<Utc>,
    /// `ETag` validator, when known.
    pub etag: Option<String>,
    /// `Last-Modified` validator, when known.
    pub last_modified: Option<String>,
}

/// Owns the catalog snapshot, the on-disk cache, and the refresh policy.
pub struct CatalogManager {
    snapshot: ArcSwap<CatalogSnapshot>,
    cache: Mutex<Option<CatalogCache>>,
    cache_path: PathBuf,
    source_url: String,
    refresh_interval: Duration,
    limits: RefreshLimits,
    client: reqwest::Client,
}

impl CatalogManager {
    /// Builds a manager with default refresh limits.
    pub fn new(cache_path: PathBuf, source_url: String) -> Self {
        Self::with_limits(cache_path, source_url, RefreshLimits::default())
    }

    /// Builds a manager with explicit refresh limits (used by tests).
    ///
    /// Loads the embedded catalog first, then overlays a valid on-disk cache
    /// regardless of age; age is used only to decide whether to refresh.
    pub fn with_limits(cache_path: PathBuf, source_url: String, limits: RefreshLimits) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.total_timeout)
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("catalog refresh client construction cannot fail");

        let bundled = Arc::new(crate::embedded_catalog());
        let refresh_interval = DEFAULT_REFRESH_INTERVAL;

        let cache = std::fs::read(&cache_path)
            .ok()
            .and_then(|bytes| decode_cache(&bytes, &source_url).ok());
        let (effective, status) = match &cache {
            Some(cache) => {
                let stale = is_stale(cache.fetched_at, refresh_interval);
                (
                    Arc::new(cache.catalog.clone()),
                    if stale {
                        RefreshStatus::Stale
                    } else {
                        RefreshStatus::Fresh
                    },
                )
            }
            None => (Arc::clone(&bundled), RefreshStatus::Stale),
        };

        Self {
            snapshot: ArcSwap::from_pointee(CatalogSnapshot {
                effective,
                bundled,
                status,
            }),
            cache: Mutex::new(cache),
            cache_path,
            source_url,
            refresh_interval,
            limits,
            client,
        }
    }

    /// Returns the current immutable snapshot.
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        self.snapshot.load_full()
    }

    /// Path of the on-disk cache file.
    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }

    /// Secret-free metadata of the current cache, when one is loaded.
    pub fn cache_metadata(&self) -> Option<CacheMetadata> {
        self.cache
            .lock()
            .expect("catalog cache lock poisoned")
            .as_ref()
            .map(|cache| CacheMetadata {
                fetched_at: cache.fetched_at,
                etag: cache.etag.clone(),
                last_modified: cache.last_modified.clone(),
            })
    }

    /// Refreshes only when the cache is missing or older than the refresh
    /// interval; otherwise returns [`RefreshOutcome::Fresh`] without any
    /// network traffic.
    pub async fn refresh_if_stale(&self) -> Result<RefreshOutcome, CatalogError> {
        let stale = self
            .cache
            .lock()
            .expect("catalog cache lock poisoned")
            .as_ref()
            .is_none_or(|cache| is_stale(cache.fetched_at, self.refresh_interval));
        if !stale {
            return Ok(RefreshOutcome::Fresh);
        }
        self.refresh().await
    }

    /// Unconditionally performs a bounded, conditional HTTP refresh.
    ///
    /// On any failure the prior in-memory snapshot and on-disk cache are
    /// kept, and the error message is concise status text that never embeds
    /// response bodies.
    pub async fn refresh(&self) -> Result<RefreshOutcome, CatalogError> {
        let validators = self
            .cache
            .lock()
            .expect("catalog cache lock poisoned")
            .as_ref()
            .map(|cache| (cache.etag.clone(), cache.last_modified.clone()));

        let mut request = self.client.get(&self.source_url);
        if let Some((etag, last_modified)) = &validators {
            if let Some(etag) = etag {
                request = request.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            if let Some(last_modified) = last_modified {
                request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
            }
        }

        let mut response = request.send().await.map_err(concise_fetch_error)?;
        match response.status().as_u16() {
            304 => {
                let mut guard = self.cache.lock().expect("catalog cache lock poisoned");
                let Some(cache) = guard.as_mut() else {
                    return Err(CatalogError::Fetch(
                        "origin returned 304 without a cached catalog".to_string(),
                    ));
                };
                let mut renewed = cache.clone();
                renewed.fetched_at = Utc::now();
                write_cache_atomic(&self.cache_path, &renewed)?;
                *cache = renewed;
                Ok(RefreshOutcome::NotModified)
            }
            200 => {
                let etag = header_string(&response, reqwest::header::ETAG);
                let last_modified = header_string(&response, reqwest::header::LAST_MODIFIED);

                let mut bytes: Vec<u8> = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(concise_fetch_error)? {
                    if bytes.len() + chunk.len() > self.limits.max_response_bytes {
                        return Err(CatalogError::LimitExceeded {
                            field: "response bytes",
                            limit: self.limits.max_response_bytes,
                            actual: bytes.len() + chunk.len(),
                        });
                    }
                    bytes.extend_from_slice(&chunk);
                }

                let normalized = crate::normalize_models_dev(
                    &bytes,
                    NormalizationLimits {
                        max_bytes: self.limits.max_response_bytes,
                        ..NormalizationLimits::default()
                    },
                )?;
                // Keep parity with the embedded artifact, which has the
                // reviewed overrides applied by the generator.
                let catalog = crate::apply_patch(normalized, crate::load_overrides())?;

                let cache = CatalogCache {
                    schema_version: CATALOG_SCHEMA_VERSION,
                    source_url: self.source_url.clone(),
                    fetched_at: Utc::now(),
                    etag,
                    last_modified,
                    catalog,
                };
                write_cache_atomic(&self.cache_path, &cache)?;

                let bundled = Arc::clone(&self.snapshot.load().bundled);
                self.snapshot.store(Arc::new(CatalogSnapshot {
                    effective: Arc::new(cache.catalog.clone()),
                    bundled,
                    status: RefreshStatus::Fresh,
                }));
                *self.cache.lock().expect("catalog cache lock poisoned") = Some(cache);
                Ok(RefreshOutcome::Updated)
            }
            other => Err(CatalogError::Fetch(format!(
                "unexpected status {other}; using cached provider catalog"
            ))),
        }
    }
}

fn is_stale(fetched_at: chrono::DateTime<Utc>, interval: Duration) -> bool {
    let interval = chrono::Duration::from_std(interval).unwrap_or(chrono::Duration::MAX);
    Utc::now().signed_duration_since(fetched_at) >= interval
}

/// Concise, body-free status text for transport failures.
fn concise_fetch_error(err: reqwest::Error) -> CatalogError {
    let reason = if err.is_timeout() {
        "request timed out"
    } else if err.is_connect() {
        "connection failed"
    } else {
        "request failed"
    };
    CatalogError::Fetch(format!(
        "{reason}; using cached provider catalog; refresh failed"
    ))
}

fn header_string(
    response: &reqwest::Response,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}
