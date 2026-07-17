//! Secret-free, schema-versioned last-known-good cache for dynamically
//! discovered provider models.
//!
//! The cache file stores only provider IDs, base URLs, timestamps, and
//! model IDs/names — never credentials. Writes are atomic (temp file +
//! `sync_all` + rename + parent-dir fsync, mirroring [`crate::cache`]) and
//! the file is created with mode 0600 on Unix. Load errors never delete or
//! modify the on-disk file.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::limits::{MAX_DISCOVERED_MODELS, MAX_ENDPOINT_BYTES, MAX_MODEL_NAME_BYTES};
use crate::types::{ModelId, ProviderId};

/// Current schema version of the dynamic discovery cache file.
pub const DYNAMIC_CACHE_SCHEMA_VERSION: u32 = 1;

/// Errors produced while reading or writing the dynamic model cache.
///
/// A load error never deletes or modifies the on-disk file.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DynamicCacheError {
    /// An I/O operation failed; carries concise, secret-free detail.
    #[error("dynamic cache I/O error: {0}")]
    Io(String),
    /// The file exists but is not valid JSON for the expected shape.
    #[error("dynamic cache is not valid JSON: {0}")]
    InvalidJson(String),
    /// The file's schema version is not [`DYNAMIC_CACHE_SCHEMA_VERSION`].
    #[error("unsupported dynamic cache schema version {found}")]
    SchemaVersion {
        /// The version found on disk.
        found: u32,
    },
    /// The file parsed but violates a validation bound.
    #[error("invalid dynamic cache: {0}")]
    Invalid(String),
}

/// A single discovered model: ID plus optional display name. Secret-free.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedModel {
    /// Validated model identifier.
    pub id: ModelId,
    /// Optional human-readable display name.
    pub name: Option<String>,
}

/// Last-known-good discovered models for one dynamic provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedProviderModels {
    /// Validated provider identifier.
    pub provider_id: ProviderId,
    /// Base URL the models were discovered from.
    pub base_url: String,
    /// Unix timestamp (seconds) of the successful discovery.
    pub fetched_at_unix: i64,
    /// Discovered models.
    pub models: Vec<CachedModel>,
}

/// The full on-disk dynamic cache document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicCacheFile {
    /// Must equal [`DYNAMIC_CACHE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Per-provider last-known-good entries, keyed by provider ID.
    pub providers: IndexMap<ProviderId, CachedProviderModels>,
}

impl DynamicCacheFile {
    /// Returns the cached entry for a provider, if any.
    pub fn provider(&self, id: &ProviderId) -> Option<&CachedProviderModels> {
        self.providers.get(id)
    }

    fn empty() -> Self {
        Self {
            schema_version: DYNAMIC_CACHE_SCHEMA_VERSION,
            providers: IndexMap::new(),
        }
    }

    fn validate(&self) -> Result<(), DynamicCacheError> {
        if self.schema_version != DYNAMIC_CACHE_SCHEMA_VERSION {
            return Err(DynamicCacheError::SchemaVersion {
                found: self.schema_version,
            });
        }
        for (key, entry) in &self.providers {
            if key != &entry.provider_id {
                return Err(DynamicCacheError::Invalid(format!(
                    "provider key {:?} does not match entry id {:?}",
                    key.as_str(),
                    entry.provider_id.as_str()
                )));
            }
            if entry.base_url.len() > MAX_ENDPOINT_BYTES {
                return Err(DynamicCacheError::Invalid(format!(
                    "base_url for provider {:?} exceeds {} bytes",
                    entry.provider_id.as_str(),
                    MAX_ENDPOINT_BYTES
                )));
            }
            if entry.models.len() > MAX_DISCOVERED_MODELS {
                return Err(DynamicCacheError::Invalid(format!(
                    "provider {:?} has {} models, exceeding {}",
                    entry.provider_id.as_str(),
                    entry.models.len(),
                    MAX_DISCOVERED_MODELS
                )));
            }
            for model in &entry.models {
                if let Some(name) = &model.name
                    && name.len() > MAX_MODEL_NAME_BYTES
                {
                    return Err(DynamicCacheError::Invalid(format!(
                        "model name for {:?} exceeds {} bytes",
                        model.id.as_str(),
                        MAX_MODEL_NAME_BYTES
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Handle to the on-disk dynamic model cache at a fixed path.
#[derive(Clone, Debug)]
pub struct DynamicCache {
    path: PathBuf,
}

impl DynamicCache {
    /// Creates a cache handle for the given file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Loads and validates the cache file.
    ///
    /// A missing file yields an empty schema-v1 cache. Any error leaves the
    /// on-disk file untouched.
    pub async fn load(&self) -> Result<DynamicCacheFile, DynamicCacheError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || load_sync(&path))
            .await
            .map_err(|err| DynamicCacheError::Io(err.to_string()))?
    }

    /// Upserts one provider's entry, preserving all others, and atomically
    /// replaces the cache file.
    ///
    /// If the existing file is corrupt (any load error other than a missing
    /// file), the write is refused rather than clobbering the file.
    pub async fn store_provider(
        &self,
        provider: CachedProviderModels,
    ) -> Result<(), DynamicCacheError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || store_provider_sync(&path, provider))
            .await
            .map_err(|err| DynamicCacheError::Io(err.to_string()))?
    }
}

fn load_sync(path: &Path) -> Result<DynamicCacheFile, DynamicCacheError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DynamicCacheFile::empty());
        }
        Err(err) => return Err(DynamicCacheError::Io(err.kind().to_string())),
    };
    // Check the schema version before full deserialization so an old or
    // newer file reports a version mismatch, not a shape mismatch.
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u32,
    }
    let probe: VersionProbe = serde_json::from_slice(&bytes)
        .map_err(|err| DynamicCacheError::InvalidJson(err.to_string()))?;
    if probe.schema_version != DYNAMIC_CACHE_SCHEMA_VERSION {
        return Err(DynamicCacheError::SchemaVersion {
            found: probe.schema_version,
        });
    }
    let file: DynamicCacheFile = serde_json::from_slice(&bytes)
        .map_err(|err| DynamicCacheError::InvalidJson(err.to_string()))?;
    file.validate()?;
    Ok(file)
}

fn store_provider_sync(
    path: &Path,
    provider: CachedProviderModels,
) -> Result<(), DynamicCacheError> {
    // Refuse to clobber a corrupt or wrong-version file; only a missing
    // file starts from empty.
    let mut file = load_sync(path)?;
    file.providers
        .insert(provider.provider_id.clone(), provider);
    file.validate()?;
    write_atomic(path, &file)
}

/// Atomically replaces the cache file, mirroring the pattern in
/// [`crate::cache::write_cache_atomic`]: write a sibling temp file with
/// `create_new(true)`, `sync_all`, rename over the target, and fsync the
/// parent directory on Unix. The file is created with mode 0600 on Unix.
fn write_atomic(path: &Path, file: &DynamicCacheFile) -> Result<(), DynamicCacheError> {
    static NONCE: AtomicU64 = AtomicU64::new(0);

    let bytes =
        serde_json::to_vec_pretty(file).map_err(|err| DynamicCacheError::Io(err.to_string()))?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "dynamic-cache.json".to_string());
    let tmp = path.with_file_name(format!(
        "{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut out = options.open(&tmp)?;
        out.write_all(&bytes)?;
        out.sync_all()?;
        drop(out);
        fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|err| DynamicCacheError::Io(format!("cache write failed: {}", err.kind())))
}

/// How the current model list for a provider was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicRefreshStatus {
    /// A successful discovery returned a non-empty model list.
    Fresh,
    /// A successful discovery returned an empty model list.
    Empty,
    /// Discovery failed; serving the last known good (possibly empty) list.
    CachedOffline,
}

/// Why a dynamic discovery refresh failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryRefreshError {
    /// The provider could not be reached.
    #[error("provider is offline")]
    Offline,
    /// The provider rejected the credentials.
    #[error("authentication failed")]
    Authentication,
    /// The provider responded with an invalid discovery document.
    #[error("discovery response is invalid")]
    InvalidResponse,
}

/// The models to serve after a refresh attempt, plus how they were obtained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicRefreshOutcome {
    models: Vec<CachedModel>,
    status: DynamicRefreshStatus,
    refresh_error: Option<DiscoveryRefreshError>,
}

impl DynamicRefreshOutcome {
    /// Combines a refresh result with the last known good cache entry.
    ///
    /// A successful refresh always replaces the cache — even when empty. A
    /// failed refresh keeps the cached models regardless of their age
    /// (`_now_unix` never causes cached models to disappear).
    pub fn from_result(
        result: Result<Vec<CachedModel>, DiscoveryRefreshError>,
        cached: Option<CachedProviderModels>,
        _now_unix: i64,
    ) -> Self {
        match result {
            Ok(models) => {
                let status = if models.is_empty() {
                    DynamicRefreshStatus::Empty
                } else {
                    DynamicRefreshStatus::Fresh
                };
                Self {
                    models,
                    status,
                    refresh_error: None,
                }
            }
            Err(err) => Self {
                models: cached.map(|entry| entry.models).unwrap_or_default(),
                status: DynamicRefreshStatus::CachedOffline,
                refresh_error: Some(err),
            },
        }
    }

    /// The models to serve.
    pub fn models(&self) -> &[CachedModel] {
        &self.models
    }

    /// How the models were obtained.
    pub fn status(&self) -> DynamicRefreshStatus {
        self.status
    }

    /// The refresh error, when discovery failed.
    pub fn refresh_error(&self) -> Option<&DiscoveryRefreshError> {
        self.refresh_error.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModelId, ProviderId};

    fn janus() -> ProviderId {
        ProviderId::new("janus").unwrap()
    }

    fn sample_entry() -> CachedProviderModels {
        CachedProviderModels {
            provider_id: janus(),
            base_url: "http://127.0.0.1:20128/v1".to_string(),
            fetched_at_unix: 1_721_088_000,
            models: vec![CachedModel {
                id: ModelId::new("openai/gpt-4o").unwrap(),
                name: None,
            }],
        }
    }

    #[tokio::test]
    async fn cache_round_trip_contains_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dynamic-cache.json");
        let cache = DynamicCache::new(&path);
        cache.store_provider(sample_entry()).await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap().to_lowercase();
        assert!(!text.contains("api_key"), "cache must not contain api_key");
        assert!(
            !text.contains("authorization"),
            "cache must not contain authorization"
        );

        let loaded = cache.load().await.unwrap();
        assert_eq!(loaded.schema_version, DYNAMIC_CACHE_SCHEMA_VERSION);
        assert_eq!(loaded.provider(&janus()), Some(&sample_entry()));
    }

    #[tokio::test]
    async fn load_on_missing_file_returns_empty_v1_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DynamicCache::new(dir.path().join("absent.json"));
        let loaded = cache.load().await.unwrap();
        assert_eq!(loaded.schema_version, DYNAMIC_CACHE_SCHEMA_VERSION);
        assert!(loaded.providers.is_empty());
    }

    #[tokio::test]
    async fn load_rejects_wrong_schema_version_and_leaves_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dynamic-cache.json");
        let bytes = br#"{"schema_version": 999, "providers": {}}"#;
        std::fs::write(&path, bytes).unwrap();

        let cache = DynamicCache::new(&path);
        assert!(matches!(
            cache.load().await,
            Err(DynamicCacheError::SchemaVersion { found: 999 })
        ));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "file must be untouched"
        );
    }

    #[tokio::test]
    async fn load_on_corrupt_json_errors_without_deleting_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dynamic-cache.json");
        std::fs::write(&path, b"{not json").unwrap();

        let cache = DynamicCache::new(&path);
        assert!(matches!(
            cache.load().await,
            Err(DynamicCacheError::InvalidJson(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[tokio::test]
    async fn store_provider_updates_one_provider_and_preserves_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dynamic-cache.json");
        let cache = DynamicCache::new(&path);

        let other = CachedProviderModels {
            provider_id: ProviderId::new("other").unwrap(),
            base_url: "http://127.0.0.1:9/v1".to_string(),
            fetched_at_unix: 1_700_000_000,
            models: Vec::new(),
        };
        cache.store_provider(other.clone()).await.unwrap();
        cache.store_provider(sample_entry()).await.unwrap();

        let mut updated = sample_entry();
        updated.fetched_at_unix = 1_721_090_000;
        updated.models.clear();
        cache.store_provider(updated.clone()).await.unwrap();

        let loaded = cache.load().await.unwrap();
        assert_eq!(loaded.providers.len(), 2);
        assert_eq!(loaded.provider(&other.provider_id), Some(&other));
        assert_eq!(loaded.provider(&janus()), Some(&updated));
    }

    #[tokio::test]
    async fn store_provider_refuses_to_clobber_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dynamic-cache.json");
        std::fs::write(&path, b"{not json").unwrap();

        let cache = DynamicCache::new(&path);
        assert!(cache.store_provider(sample_entry()).await.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[tokio::test]
    async fn failed_refresh_keeps_stale_last_known_good() {
        let cached = sample_entry();
        let outcome = DynamicRefreshOutcome::from_result(
            Err(DiscoveryRefreshError::Offline),
            Some(cached.clone()),
            2_000_000,
        );
        assert_eq!(outcome.models(), cached.models.as_slice());
        assert_eq!(outcome.status(), DynamicRefreshStatus::CachedOffline);
        assert_eq!(
            outcome.refresh_error(),
            Some(&DiscoveryRefreshError::Offline)
        );
    }

    #[tokio::test]
    async fn valid_empty_response_replaces_nonempty_cache() {
        let outcome =
            DynamicRefreshOutcome::from_result(Ok(Vec::new()), Some(sample_entry()), 2_000_000);
        assert!(outcome.models().is_empty());
        assert_eq!(outcome.status(), DynamicRefreshStatus::Empty);
        assert_eq!(outcome.refresh_error(), None);
    }

    #[tokio::test]
    async fn successful_nonempty_refresh_is_fresh() {
        let fresh = vec![CachedModel {
            id: ModelId::new("new-model").unwrap(),
            name: Some("New Model".to_string()),
        }];
        let outcome =
            DynamicRefreshOutcome::from_result(Ok(fresh.clone()), Some(sample_entry()), 2_000_000);
        assert_eq!(outcome.models(), fresh.as_slice());
        assert_eq!(outcome.status(), DynamicRefreshStatus::Fresh);
        assert_eq!(outcome.refresh_error(), None);
    }

    #[tokio::test]
    async fn failed_refresh_with_no_cache_yields_empty_models_and_the_error() {
        let outcome = DynamicRefreshOutcome::from_result(
            Err(DiscoveryRefreshError::Authentication),
            None,
            2_000_000,
        );
        assert!(outcome.models().is_empty());
        assert_eq!(outcome.status(), DynamicRefreshStatus::CachedOffline);
        assert_eq!(
            outcome.refresh_error(),
            Some(&DiscoveryRefreshError::Authentication)
        );
    }
}
