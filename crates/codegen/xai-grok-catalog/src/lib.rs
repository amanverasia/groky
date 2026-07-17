//! Dependency-neutral provider/model catalog.
//!
//! Owns normalized provider/model DTOs, deterministic models.dev ingestion,
//! and field-aware layering. This crate must not depend on shell, pager,
//! sampler, auth, ACP, or sampling-types.

pub mod cache;
pub mod credentials;
pub mod layer;
pub mod normalize;
pub mod refresh;
pub mod types;

pub use cache::{CatalogCache, decode_cache, encode_cache};
pub use credentials::{
    CredentialOrigin, CredentialSources, ProviderAvailability, ProviderStatus, ResolvedCredential,
    SecretString, classify_provider, resolve_credential,
};
pub use layer::{CatalogPatch, ModelPatch, ProviderPatch, apply_patch};
pub use normalize::normalize_models_dev;
pub use refresh::{
    CacheMetadata, CatalogManager, CatalogSnapshot, DEFAULT_MAX_RESPONSE_BYTES,
    DEFAULT_REFRESH_INTERVAL, RefreshLimits, RefreshOutcome, RefreshStatus,
};
pub use types::{
    CATALOG_SCHEMA_VERSION, CatalogError, CatalogModel, CatalogProvider, ModelCost, ModelId,
    NormalizationLimits, NormalizedCatalog, Protocol, ProviderId,
};

/// Loads the committed, generated catalog snapshot embedded in the binary.
///
/// The snapshot is a release artifact produced by `generate_catalog`, not
/// remote input: a malformed or wrong-version snapshot panics at embedded
/// startup with a path-specific message.
pub fn embedded_catalog() -> NormalizedCatalog {
    let catalog: NormalizedCatalog =
        serde_json::from_slice(include_bytes!("../data/models-dev.json")).unwrap_or_else(|err| {
            panic!(
                "malformed embedded catalog at \
                 crates/codegen/xai-grok-catalog/data/models-dev.json: {err}"
            )
        });
    assert_eq!(
        catalog.schema_version, CATALOG_SCHEMA_VERSION,
        "embedded catalog schema version mismatch at \
         crates/codegen/xai-grok-catalog/data/models-dev.json"
    );
    catalog
}

/// Loads the committed, reviewed override patch embedded in the binary.
///
/// The overrides file is a release artifact, not remote input: a malformed
/// committed override panics at embedded startup with a path-specific message.
pub fn load_overrides() -> CatalogPatch {
    serde_json::from_slice(include_bytes!("../data/overrides.json")).unwrap_or_else(|err| {
        panic!(
            "malformed embedded overrides at \
             crates/codegen/xai-grok-catalog/data/overrides.json: {err}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_overrides_parse_and_contain_xai() {
        let overrides = load_overrides();
        let xai = overrides
            .providers
            .iter()
            .find(|p| p.id.as_str() == "xai")
            .expect("xai override present");
        assert_eq!(xai.api_base_url.as_deref(), Some("https://api.x.ai/v1"));
        assert_eq!(
            xai.env_vars.as_deref(),
            Some(&["XAI_API_KEY".to_string()][..])
        );
        assert!(!xai.exclude);
    }

    #[test]
    fn embedded_overrides_apply_over_an_empty_catalog() {
        let base = NormalizedCatalog {
            schema_version: CATALOG_SCHEMA_VERSION,
            providers: Vec::new(),
        };
        let merged = apply_patch(base, load_overrides()).unwrap();
        let xai = merged.provider_str("xai").unwrap();
        assert_eq!(xai.api_base_url, "https://api.x.ai/v1");
        assert_eq!(xai.env_vars, ["XAI_API_KEY"]);
    }

    #[test]
    fn embedded_overrides_supply_base_urls_for_first_party_sdk_providers() {
        // Raw models.dev leaves `api` empty for providers addressed via
        // first-party SDKs; the reviewed overrides must supply base URLs.
        let overrides = load_overrides();
        let base = |id: &str| {
            overrides
                .providers
                .iter()
                .find(|p| p.id.as_str() == id)
                .and_then(|p| p.api_base_url.clone())
        };
        assert_eq!(
            base("anthropic").as_deref(),
            Some("https://api.anthropic.com/v1")
        );
        assert_eq!(base("openai").as_deref(), Some("https://api.openai.com/v1"));
        for id in [
            "cerebras",
            "deepinfra",
            "groq",
            "mistral",
            "perplexity",
            "togetherai",
            "xai",
        ] {
            assert!(
                base(id).is_some_and(|url| url.starts_with("https://")),
                "provider {id} must have an override base URL"
            );
        }
    }

    #[test]
    fn embedded_catalog_parses_and_has_no_empty_base_urls() {
        let catalog = embedded_catalog();
        assert_eq!(catalog.schema_version, CATALOG_SCHEMA_VERSION);
        assert!(!catalog.providers.is_empty());
        assert!(catalog.provider_str("xai").is_some());
        for provider in &catalog.providers {
            assert!(
                !provider.api_base_url.is_empty(),
                "provider {} has an empty api_base_url",
                provider.id.as_str()
            );
        }
    }
}
