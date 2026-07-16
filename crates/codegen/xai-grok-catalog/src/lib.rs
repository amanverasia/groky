//! Dependency-neutral provider/model catalog.
//!
//! Owns normalized provider/model DTOs, deterministic models.dev ingestion,
//! and field-aware layering. This crate must not depend on shell, pager,
//! sampler, auth, ACP, or sampling-types.

pub mod layer;
pub mod normalize;
pub mod types;

pub use layer::{CatalogPatch, ModelPatch, ProviderPatch, apply_patch};
pub use normalize::normalize_models_dev;
pub use types::{
    CATALOG_SCHEMA_VERSION, CatalogError, CatalogModel, CatalogProvider, ModelCost, ModelId,
    NormalizationLimits, NormalizedCatalog, Protocol, ProviderId,
};

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
}
