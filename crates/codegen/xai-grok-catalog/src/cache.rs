//! Schema-versioned, secret-free on-disk catalog cache.
//!
//! The cache envelope stores only the source URL, timestamps, HTTP
//! validators, and the normalized catalog — never credentials.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{CATALOG_SCHEMA_VERSION, CatalogError, NormalizedCatalog};

/// On-disk cache envelope for a fetched provider catalog.
///
/// All fields are serialized unconditionally so consumers (and tests) can
/// rely on every envelope key being present.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogCache {
    /// Envelope/catalog schema version; must equal [`CATALOG_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The URL this catalog was fetched from.
    pub source_url: String,
    /// When the catalog was last fetched or revalidated.
    pub fetched_at: DateTime<Utc>,
    /// HTTP `ETag` validator from the last 200 response, when present.
    pub etag: Option<String>,
    /// HTTP `Last-Modified` validator from the last 200 response, when present.
    pub last_modified: Option<String>,
    /// The normalized, secret-free catalog payload.
    pub catalog: NormalizedCatalog,
}

/// Serializes a cache envelope to pretty JSON bytes.
pub fn encode_cache(cache: &CatalogCache) -> Result<Vec<u8>, CatalogError> {
    serde_json::to_vec_pretty(cache).map_err(|err| CatalogError::Cache(err.to_string()))
}

/// Parses and validates a cache envelope read from disk.
///
/// Rejects unsupported schema versions, an unexpected source URL, and
/// catalogs whose own schema version does not match.
pub fn decode_cache(bytes: &[u8], expected_source: &str) -> Result<CatalogCache, CatalogError> {
    let cache: CatalogCache = serde_json::from_slice(bytes)
        .map_err(|err| CatalogError::InvalidDocument(err.to_string()))?;
    if cache.schema_version != CATALOG_SCHEMA_VERSION {
        return Err(CatalogError::Cache(format!(
            "unsupported cache schema version {}",
            cache.schema_version
        )));
    }
    if cache.source_url != expected_source {
        return Err(CatalogError::Cache(
            "cache source URL does not match the configured source".to_string(),
        ));
    }
    if cache.catalog.schema_version != CATALOG_SCHEMA_VERSION {
        return Err(CatalogError::Cache(format!(
            "unsupported cached catalog schema version {}",
            cache.catalog.schema_version
        )));
    }
    Ok(cache)
}

/// Atomically replaces the cache file: write `<cache>.tmp-<pid>` with
/// `create_new(true)`, `sync_all`, rename over the target, and sync the
/// parent directory on Unix. A failed temp file is deleted.
pub(crate) fn write_cache_atomic(path: &Path, cache: &CatalogCache) -> Result<(), CatalogError> {
    let bytes = encode_cache(cache)?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "catalog-cache.json".to_string());
    let tmp = path.with_file_name(format!("{}.tmp-{}", file_name, std::process::id()));
    // A leftover temp from a crashed prior run would make create_new fail
    // forever; it is ours (same pid namespace), so clear it first.
    let _ = fs::remove_file(&tmp);

    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
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
    result.map_err(|err| CatalogError::Cache(format!("cache write failed: {}", err.kind())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CatalogCache {
        CatalogCache {
            schema_version: CATALOG_SCHEMA_VERSION,
            source_url: "https://models.dev/api.json".to_string(),
            fetched_at: Utc::now(),
            etag: None,
            last_modified: None,
            catalog: NormalizedCatalog {
                schema_version: CATALOG_SCHEMA_VERSION,
                providers: Vec::new(),
            },
        }
    }

    #[test]
    fn roundtrips_through_encode_and_decode() {
        let cache = sample();
        let bytes = encode_cache(&cache).unwrap();
        let decoded = decode_cache(&bytes, "https://models.dev/api.json").unwrap();
        assert_eq!(decoded, cache);
    }

    #[test]
    fn rejects_wrong_schema_and_wrong_source() {
        let mut wrong_schema = sample();
        wrong_schema.schema_version = 999;
        let bytes = encode_cache(&wrong_schema).unwrap();
        assert!(matches!(
            decode_cache(&bytes, "https://models.dev/api.json"),
            Err(CatalogError::Cache(_))
        ));

        let bytes = encode_cache(&sample()).unwrap();
        assert!(matches!(
            decode_cache(&bytes, "https://other.example/api.json"),
            Err(CatalogError::Cache(_))
        ));
    }

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog-cache.json");
        write_cache_atomic(&path, &sample()).unwrap();
        assert!(
            decode_cache(
                &std::fs::read(&path).unwrap(),
                "https://models.dev/api.json"
            )
            .is_ok()
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must not survive a write");
    }
}
