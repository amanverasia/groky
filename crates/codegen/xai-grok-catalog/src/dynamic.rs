//! User-declared dynamic provider configuration with locked bounds.
//!
//! Dynamic providers are declared in local configuration (TOML) and may
//! optionally discover their model list at runtime. All identifier and
//! endpoint lengths are validated at the configuration boundary so that
//! neither the typed constructor nor serde deserialization can bypass the
//! limits in [`crate::limits`].

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use url::Url;

use crate::discovery::DiscoveredModel;
use crate::http::{HttpError, validate_url};
use crate::layer::ModelPatch;
use crate::limits::{MAX_ENDPOINT_BYTES, MAX_PROVIDER_ID_BYTES, MAX_PROVIDER_NAME_BYTES};
use crate::types::{CatalogModel, ModelCost, ModelId, Protocol, ProviderId};

/// Errors produced while validating dynamic provider configuration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DynamicConfigError {
    /// Provider ID exceeds [`MAX_PROVIDER_ID_BYTES`].
    #[error("provider id exceeds 64 bytes")]
    ProviderIdTooLong,
    /// Provider display name exceeds [`MAX_PROVIDER_NAME_BYTES`].
    #[error("provider name exceeds 128 bytes")]
    ProviderNameTooLong,
    /// A base URL or endpoint override exceeds [`MAX_ENDPOINT_BYTES`].
    #[error("endpoint exceeds 2048 bytes")]
    EndpointTooLong,
    /// Provider ID failed [`ProviderId`] validation.
    #[error("invalid provider id: {0}")]
    InvalidProviderId(String),
    /// A model key failed [`ModelId`] validation.
    #[error("invalid model id: {0}")]
    InvalidModelId(String),
    /// The provider base URL violates the dynamic-provider URL policy.
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(#[from] HttpError),
    /// A base URL must not contain a query string or fragment.
    #[error("base URL must not contain a query string or fragment")]
    BaseUrlHasQueryOrFragment,
}

/// A user-declared dynamic provider, validated at the configuration boundary.
///
/// Deserialization routes through a raw mirror type so that TOML/JSON input
/// cannot bypass the identifier and endpoint bounds enforced by
/// [`DynamicProviderConfig::new`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawDynamicProviderConfig")]
pub struct DynamicProviderConfig {
    /// Provider identifier.
    pub id: ProviderId,
    /// Human-readable display name.
    pub name: String,
    /// API base URL in its configured lexical form, retained for compatibility.
    pub base_url: String,
    /// Ordered environment-variable names used to locate credentials.
    ///
    /// This is metadata only; values are never accepted or serialized here.
    #[serde(default)]
    pub env_vars: Vec<String>,
    /// Wire protocol; serialized as `api_backend`.
    #[serde(rename = "api_backend")]
    pub protocol: Protocol,
    /// When true, a bearer API key is optional for this provider.
    pub unauthenticated: bool,
    /// When true, the model list is discovered at runtime.
    pub discover: bool,
    /// Override for the model-discovery endpoint.
    pub models_endpoint: Option<String>,
    /// Override for the health-check endpoint.
    pub health_endpoint: Option<String>,
    /// When true, plain-HTTP endpoints are permitted.
    pub allow_insecure_http: bool,
    /// Statically declared models, keyed by validated model ID.
    pub models: IndexMap<ModelId, ModelPatch>,
}

impl DynamicProviderConfig {
    /// Validates identifiers and lengths, then constructs a config with
    /// defaults: `ChatCompletions` protocol, no discovery, no overrides,
    /// no static models.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, DynamicConfigError> {
        Self::new_with_policy(id, name, base_url, false)
    }

    /// Constructs a provider after validating its base URL with the selected
    /// plain-HTTP policy.
    pub fn new_with_policy(
        id: impl Into<String>,
        name: impl Into<String>,
        base_url: impl Into<String>,
        allow_insecure_http: bool,
    ) -> Result<Self, DynamicConfigError> {
        let id = validate_provider_id(id.into())?;
        let name = validate_provider_name(name.into())?;
        let base_url = validate_endpoint(base_url.into())?;
        canonicalize_base_url(&base_url, allow_insecure_http)?;
        Ok(Self {
            id,
            name,
            base_url,
            env_vars: Vec::new(),
            protocol: Protocol::default(),
            unauthenticated: false,
            discover: false,
            models_endpoint: None,
            health_endpoint: None,
            allow_insecure_http,
            models: IndexMap::new(),
        })
    }

    /// Returns the canonical identity URL for this provider's base endpoint.
    pub fn canonical_base_url(&self) -> Result<String, DynamicConfigError> {
        canonicalize_base_url(&self.base_url, self.allow_insecure_http)
    }
}

/// Validates and canonicalizes a provider base URL for identity comparisons.
///
/// Hosts are lower-cased by `url`, default ports are omitted, and trailing
/// slashes are removed except at the origin root. Meaningful path segments are
/// otherwise preserved. Queries, fragments, and userinfo are not valid base
/// endpoint identity.
pub fn canonicalize_base_url(
    base_url: &str,
    allow_insecure_http: bool,
) -> Result<String, DynamicConfigError> {
    let mut url = Url::parse(base_url).map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
    validate_url(&url, allow_insecure_http)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(DynamicConfigError::BaseUrlHasQueryOrFragment);
    }
    if matches!(
        (url.scheme(), url.port()),
        ("https", Some(443)) | ("http", Some(80))
    ) {
        let _ = url.set_port(None);
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Ok(url.to_string())
}

fn validate_provider_id(id: String) -> Result<ProviderId, DynamicConfigError> {
    if id.len() > MAX_PROVIDER_ID_BYTES {
        return Err(DynamicConfigError::ProviderIdTooLong);
    }
    ProviderId::new(id).map_err(|err| DynamicConfigError::InvalidProviderId(err.to_string()))
}

fn validate_provider_name(name: String) -> Result<String, DynamicConfigError> {
    if name.len() > MAX_PROVIDER_NAME_BYTES {
        return Err(DynamicConfigError::ProviderNameTooLong);
    }
    Ok(name)
}

fn validate_endpoint(endpoint: String) -> Result<String, DynamicConfigError> {
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return Err(DynamicConfigError::EndpointTooLong);
    }
    Ok(endpoint)
}

/// Unvalidated mirror of [`DynamicProviderConfig`] used for deserialization.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDynamicProviderConfig {
    id: String,
    name: String,
    base_url: String,
    #[serde(default)]
    env_vars: Vec<String>,
    #[serde(default, rename = "api_backend")]
    protocol: Protocol,
    #[serde(default)]
    unauthenticated: bool,
    #[serde(default)]
    discover: bool,
    #[serde(default)]
    models_endpoint: Option<String>,
    #[serde(default)]
    health_endpoint: Option<String>,
    #[serde(default)]
    allow_insecure_http: bool,
    #[serde(default)]
    models: IndexMap<String, RawModelPatch>,
}

/// Unvalidated mirror of a static model entry; the model ID is the map key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModelPatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "api_backend")]
    protocol: Option<Protocol>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    cost: Option<ModelCost>,
    #[serde(default)]
    exclude: bool,
}

impl TryFrom<RawDynamicProviderConfig> for DynamicProviderConfig {
    type Error = DynamicConfigError;

    fn try_from(raw: RawDynamicProviderConfig) -> Result<Self, Self::Error> {
        let id = validate_provider_id(raw.id)?;
        let name = validate_provider_name(raw.name)?;
        let base_url = validate_endpoint(raw.base_url)?;
        canonicalize_base_url(&base_url, raw.allow_insecure_http)?;
        let models_endpoint = raw.models_endpoint.map(validate_endpoint).transpose()?;
        let health_endpoint = raw.health_endpoint.map(validate_endpoint).transpose()?;
        let mut models = IndexMap::with_capacity(raw.models.len());
        for (key, patch) in raw.models {
            let model_id = ModelId::new(key)
                .map_err(|err| DynamicConfigError::InvalidModelId(err.to_string()))?;
            models.insert(
                model_id.clone(),
                ModelPatch {
                    id: model_id,
                    name: patch.name,
                    protocol: patch.protocol,
                    context_window: patch.context_window,
                    reasoning: patch.reasoning,
                    cost: patch.cost,
                    exclude: patch.exclude,
                },
            );
        }
        Ok(Self {
            id,
            name,
            base_url,
            env_vars: raw.env_vars,
            protocol: raw.protocol,
            unauthenticated: raw.unauthenticated,
            discover: raw.discover,
            models_endpoint,
            health_endpoint,
            allow_insecure_http: raw.allow_insecure_http,
            models,
        })
    }
}

/// Merges runtime-discovered models with statically declared patches.
///
/// Discovered models become [`CatalogModel`]s using `default_protocol`, with
/// missing metadata enriched from `bundled_exact` on an exact full-ID match
/// only (a discovered `gpt-4o` never inherits from a bundled
/// `openai/gpt-4o`). Static patches then win field-by-field: `exclude`
/// removes a model even if discovered, supplied fields override, and
/// static-only models are appended after the discovered set.
pub fn merge_dynamic_models<'a>(
    default_protocol: Protocol,
    static_models: &IndexMap<ModelId, ModelPatch>,
    discovered: Vec<DiscoveredModel>,
    mut bundled_exact: impl FnMut(&ModelId) -> Option<&'a CatalogModel>,
) -> Vec<CatalogModel> {
    let mut merged: IndexMap<ModelId, CatalogModel> = IndexMap::new();
    for model in discovered {
        if merged.contains_key(&model.id) {
            continue;
        }
        let bundled = bundled_exact(&model.id);
        let entry = CatalogModel {
            id: model.id.clone(),
            name: model
                .name
                .or_else(|| bundled.map(|b| b.name.clone()))
                .unwrap_or_else(|| model.id.as_str().to_string()),
            protocol: default_protocol,
            context_window: bundled.and_then(|b| b.context_window),
            reasoning: bundled.is_some_and(|b| b.reasoning),
            cost: bundled.and_then(|b| b.cost.clone()),
        };
        merged.insert(model.id, entry);
    }

    for (id, patch) in static_models {
        if patch.exclude {
            merged.shift_remove(id);
            continue;
        }
        let model = match merged.get_mut(id) {
            Some(model) => model,
            None => {
                let bundled = bundled_exact(id);
                merged.insert(
                    id.clone(),
                    CatalogModel {
                        id: id.clone(),
                        name: bundled
                            .map(|b| b.name.clone())
                            .unwrap_or_else(|| id.as_str().to_string()),
                        protocol: default_protocol,
                        context_window: bundled.and_then(|b| b.context_window),
                        reasoning: bundled.is_some_and(|b| b.reasoning),
                        cost: bundled.and_then(|b| b.cost.clone()),
                    },
                );
                merged.get_mut(id).expect("model just inserted")
            }
        };
        if let Some(name) = &patch.name {
            model.name = name.clone();
        }
        if let Some(protocol) = patch.protocol {
            model.protocol = protocol;
        }
        if let Some(context_window) = patch.context_window {
            model.context_window = Some(context_window);
        }
        if let Some(reasoning) = patch.reasoning {
            model.reasoning = reasoning;
        }
        if let Some(cost) = &patch.cost {
            model.cost = Some(cost.clone());
        }
    }

    merged.into_values().collect()
}

#[cfg(test)]
mod tests {
    use crate::limits::{
        DISCOVERY_TIMEOUT, DYNAMIC_CACHE_MAX_AGE, HEALTH_TIMEOUT, MAX_DISCOVERED_MODELS,
        MAX_DISCOVERY_BODY_BYTES, MAX_ENDPOINT_BYTES, MAX_MODEL_ID_BYTES, MAX_MODEL_NAME_BYTES,
        MAX_PROVIDER_ID_BYTES, MAX_PROVIDER_NAME_BYTES, MAX_REDIRECTS,
    };
    use crate::types::Protocol;

    use super::*;

    #[test]
    fn numeric_bounds_are_locked() {
        assert_eq!(MAX_DISCOVERY_BODY_BYTES, 2 * 1024 * 1024);
        assert_eq!(MAX_DISCOVERED_MODELS, 2_000);
        assert_eq!(MAX_PROVIDER_ID_BYTES, 64);
        assert_eq!(MAX_PROVIDER_NAME_BYTES, 128);
        assert_eq!(MAX_MODEL_ID_BYTES, 512);
        assert_eq!(MAX_MODEL_NAME_BYTES, 512);
        assert_eq!(MAX_ENDPOINT_BYTES, 2_048);
        assert_eq!(MAX_REDIRECTS, 5);
        assert_eq!(DISCOVERY_TIMEOUT, std::time::Duration::from_secs(10));
        assert_eq!(HEALTH_TIMEOUT, std::time::Duration::from_secs(3));
        assert_eq!(
            DYNAMIC_CACHE_MAX_AGE,
            std::time::Duration::from_secs(24 * 60 * 60)
        );
    }

    #[test]
    fn config_defaults_to_chat_completions_and_models_discovery() {
        let config: DynamicProviderConfig = toml::from_str(
            r#"
            id = "local"
            name = "Local Gateway"
            base_url = "http://127.0.0.1:9000/v1"
            unauthenticated = true
            discover = true
            "#,
        )
        .expect("minimal dynamic provider config parses");
        assert_eq!(config.id.as_str(), "local");
        assert_eq!(config.name, "Local Gateway");
        assert_eq!(config.base_url, "http://127.0.0.1:9000/v1");
        assert_eq!(config.protocol, Protocol::ChatCompletions);
        assert!(config.unauthenticated);
        assert!(config.discover);
        assert_eq!(config.models_endpoint, None);
        assert_eq!(config.health_endpoint, None);
        assert!(!config.allow_insecure_http);
        assert!(config.models.is_empty());
    }

    #[test]
    fn provider_and_static_model_ids_keep_slashes_and_hyphens() {
        let config: DynamicProviderConfig = toml::from_str(
            r#"
            id = "my-gateway"
            name = "Gateway"
            base_url = "https://gateway.example/v1"

            [models."openai/gpt-4o"]
            name = "GPT-4o"

            [models.best-effort]
            context_window = 32768
            "#,
        )
        .expect("static model tables parse");
        assert_eq!(config.id.as_str(), "my-gateway");
        assert_eq!(config.models.len(), 2);
        let gpt = config
            .models
            .iter()
            .find(|(id, _)| id.as_str() == "openai/gpt-4o")
            .map(|(_, patch)| patch)
            .expect("slashed model id preserved");
        assert_eq!(gpt.name.as_deref(), Some("GPT-4o"));
        let best = config
            .models
            .iter()
            .find(|(id, _)| id.as_str() == "best-effort")
            .map(|(_, patch)| patch)
            .expect("hyphenated model id preserved");
        assert_eq!(best.context_window, Some(32768));
    }

    #[test]
    fn env_vars_are_names_only_ordered_metadata() {
        let config: DynamicProviderConfig = toml::from_str(
            r#"
            id = "local"
            name = "Local Gateway"
            base_url = "http://127.0.0.1:9000/v1"
            env_vars = ["FIRST_KEY", "SECOND_KEY", "FIRST_KEY"]
            "#,
        )
        .expect("env var names parse");
        assert_eq!(config.env_vars, ["FIRST_KEY", "SECOND_KEY", "FIRST_KEY"]);
        assert!(
            DynamicProviderConfig::new("local", "Local", "http://127.0.0.1:9000/v1")
                .unwrap()
                .env_vars
                .is_empty()
        );
    }

    #[test]
    fn secret_bearing_config_fields_are_rejected() {
        let err = toml::from_str::<DynamicProviderConfig>(
            r#"
            id = "local"
            name = "Local Gateway"
            base_url = "http://127.0.0.1:9000/v1"
            api_key = "not-accepted"
            "#,
        )
        .expect_err("secret-bearing fields must not deserialize");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn canonical_base_url_equates_lexical_variants_but_keeps_paths_distinct() {
        let first =
            DynamicProviderConfig::new("local", "Local", "https://EXAMPLE.com:443/v1/").unwrap();
        let equivalent =
            DynamicProviderConfig::new("local", "Local", "https://example.com/v1").unwrap();
        let other_path =
            DynamicProviderConfig::new("local", "Local", "https://example.com/v2").unwrap();
        assert_eq!(
            first.canonical_base_url().unwrap(),
            "https://example.com/v1"
        );
        assert_eq!(first.canonical_base_url(), equivalent.canonical_base_url());
        assert_ne!(first.canonical_base_url(), other_path.canonical_base_url());
    }

    #[test]
    fn base_url_enforces_url_policy_and_rejects_query_fragment_and_userinfo() {
        assert!(matches!(
            DynamicProviderConfig::new("remote", "Remote", "http://example.com/v1"),
            Err(DynamicConfigError::InvalidBaseUrl(
                HttpError::InsecureHttpDenied
            ))
        ));
        assert!(
            DynamicProviderConfig::new_with_policy(
                "remote",
                "Remote",
                "http://example.com/v1",
                true
            )
            .is_ok()
        );
        assert!(matches!(
            DynamicProviderConfig::new("local", "Local", "https://example.com/v1?x=y"),
            Err(DynamicConfigError::BaseUrlHasQueryOrFragment)
        ));
        assert!(matches!(
            DynamicProviderConfig::new("local", "Local", "https://key@example.com/v1"),
            Err(DynamicConfigError::InvalidBaseUrl(
                HttpError::CredentialsInUrl
            ))
        ));
    }

    #[test]
    fn invalid_lengths_are_rejected_at_config_boundary() {
        let err =
            DynamicProviderConfig::new("x".repeat(65), "Gateway", "https://gateway.example/v1")
                .expect_err("65-byte provider id must be rejected");
        assert_eq!(err, DynamicConfigError::ProviderIdTooLong);
    }

    #[test]
    fn toml_cannot_bypass_id_or_endpoint_bounds() {
        let overlong_id = format!(
            "id = \"{}\"\nname = \"Gateway\"\nbase_url = \"https://gateway.example/v1\"\n",
            "x".repeat(65)
        );
        let err = toml::from_str::<DynamicProviderConfig>(&overlong_id)
            .expect_err("overlong id via TOML must fail");
        assert!(
            err.to_string().contains("provider id exceeds 64 bytes"),
            "unexpected error: {err}"
        );

        let overlong_endpoint = format!(
            "id = \"local\"\nname = \"Gateway\"\nbase_url = \"https://gateway.example/v1\"\n\
             models_endpoint = \"{}\"\n",
            "e".repeat(2_049)
        );
        let err = toml::from_str::<DynamicProviderConfig>(&overlong_endpoint)
            .expect_err("overlong models_endpoint via TOML must fail");
        assert!(
            err.to_string().contains("endpoint exceeds 2048 bytes"),
            "unexpected error: {err}"
        );
    }

    fn discovered(id: &str) -> DiscoveredModel {
        DiscoveredModel::new(id).expect("valid discovered id")
    }

    fn patch(id: &str) -> ModelPatch {
        ModelPatch {
            id: ModelId::new(id).unwrap(),
            name: None,
            protocol: None,
            context_window: None,
            reasoning: None,
            cost: None,
            exclude: false,
        }
    }

    fn bundled_gpt4o() -> CatalogModel {
        CatalogModel {
            id: ModelId::new("openai/gpt-4o").unwrap(),
            name: "GPT-4o".to_string(),
            protocol: Protocol::ChatCompletions,
            context_window: Some(128_000),
            reasoning: true,
            cost: Some(ModelCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
            }),
        }
    }

    #[test]
    fn explicit_static_model_wins_and_exact_catalog_match_enriches_missing_fields() {
        let bundled = bundled_gpt4o();
        let mut static_models = IndexMap::new();
        let mut pinned = patch("openai/gpt-4o");
        pinned.name = Some("Pinned GPT".to_string());
        static_models.insert(pinned.id.clone(), pinned);

        let merged = merge_dynamic_models(
            Protocol::ChatCompletions,
            &static_models,
            vec![discovered("openai/gpt-4o"), discovered("best-effort")],
            |id| (id == &bundled.id).then_some(&bundled),
        );

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id.as_str(), "openai/gpt-4o");
        assert_eq!(merged[0].name, "Pinned GPT");
        assert_eq!(merged[0].context_window, Some(128_000));
        assert!(merged[0].reasoning);
        assert_eq!(
            merged[0].cost,
            Some(ModelCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
            })
        );
        assert_eq!(merged[1].id.as_str(), "best-effort");
        assert_eq!(merged[1].name, "best-effort");
        assert_eq!(merged[1].context_window, None);
        assert_eq!(merged[1].protocol, Protocol::ChatCompletions);
    }

    #[test]
    fn duplicate_discovered_ids_collapse_in_first_seen_order() {
        let merged = merge_dynamic_models(
            Protocol::ChatCompletions,
            &IndexMap::new(),
            vec![discovered("b"), discovered("a"), discovered("b")],
            |_| None,
        );
        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
    }

    #[test]
    fn enrichment_requires_exact_full_id_match() {
        let bundled = bundled_gpt4o();
        let merged = merge_dynamic_models(
            Protocol::ChatCompletions,
            &IndexMap::new(),
            vec![discovered("gpt-4o")],
            |id| (id == &bundled.id).then_some(&bundled),
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id.as_str(), "gpt-4o");
        assert_eq!(merged[0].name, "gpt-4o");
        assert_eq!(merged[0].context_window, None);
        assert!(!merged[0].reasoning);
        assert_eq!(merged[0].cost, None);
    }

    #[test]
    fn static_exclude_removes_model_even_if_discovered() {
        let mut static_models = IndexMap::new();
        let mut excluded = patch("openai/gpt-4o");
        excluded.exclude = true;
        static_models.insert(excluded.id.clone(), excluded);

        let merged = merge_dynamic_models(
            Protocol::ChatCompletions,
            &static_models,
            vec![discovered("openai/gpt-4o"), discovered("best-effort")],
            |_| None,
        );
        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["best-effort"]);
    }

    #[test]
    fn static_only_models_are_included_with_bundled_enrichment() {
        let bundled = bundled_gpt4o();
        let mut static_models = IndexMap::new();
        let mut pinned = patch("openai/gpt-4o");
        pinned.name = Some("Pinned GPT".to_string());
        static_models.insert(pinned.id.clone(), pinned);

        let merged = merge_dynamic_models(
            Protocol::Responses,
            &static_models,
            vec![discovered("best-effort")],
            |id| (id == &bundled.id).then_some(&bundled),
        );
        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["best-effort", "openai/gpt-4o"]);
        assert_eq!(merged[1].name, "Pinned GPT");
        assert_eq!(merged[1].context_window, Some(128_000));
        assert!(merged[1].reasoning);
    }
}
