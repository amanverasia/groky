//! Dependency-neutral provider/model catalog.
//!
//! Owns normalized provider/model DTOs, deterministic models.dev ingestion,
//! and field-aware layering. This crate must not depend on shell, pager,
//! sampler, auth, ACP, or sampling-types.

pub mod normalize;
pub mod types;

pub use normalize::normalize_models_dev;
pub use types::{
    CatalogError, CatalogModel, CatalogProvider, ModelCost, ModelId, NormalizationLimits,
    NormalizedCatalog, Protocol, ProviderId, CATALOG_SCHEMA_VERSION,
};
