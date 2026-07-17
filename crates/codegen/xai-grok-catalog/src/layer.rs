//! Field-aware provider/model patch layering.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::{
    CatalogError, CatalogModel, CatalogProvider, ModelCost, ModelId, NormalizedCatalog, Protocol,
    ProviderId,
};

/// A reviewed patch layered over a normalized catalog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogPatch {
    /// Provider patches, applied by provider ID.
    #[serde(default)]
    pub providers: Vec<ProviderPatch>,
}

/// A field-aware patch for a single provider.
///
/// `Option` fields distinguish "not supplied" from explicit values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderPatch {
    /// Provider to patch (or add).
    pub id: ProviderId,
    /// Replacement display name, when supplied.
    #[serde(default)]
    pub name: Option<String>,
    /// Replacement API base URL, when supplied.
    #[serde(default)]
    pub api_base_url: Option<String>,
    /// Replacement environment variable names, when supplied.
    #[serde(default)]
    pub env_vars: Option<Vec<String>>,
    /// Replacement unauthenticated flag, when supplied.
    #[serde(default)]
    pub unauthenticated: Option<bool>,
    /// When true, removes the provider entirely.
    #[serde(default)]
    pub exclude: bool,
    /// Model patches for this provider.
    #[serde(default)]
    pub models: Vec<ModelPatch>,
}

/// A field-aware patch for a single model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelPatch {
    /// Model to patch (or add).
    pub id: ModelId,
    /// Replacement display name, when supplied.
    #[serde(default)]
    pub name: Option<String>,
    /// Replacement protocol, when supplied.
    #[serde(default)]
    pub protocol: Option<Protocol>,
    /// Replacement context window, when supplied.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Replacement reasoning capability, when supplied.
    #[serde(default)]
    pub reasoning: Option<bool>,
    /// Replacement cost, when supplied.
    #[serde(default)]
    pub cost: Option<ModelCost>,
    /// When true, removes the model entirely.
    #[serde(default)]
    pub exclude: bool,
}

fn validate_merged(catalog: &NormalizedCatalog) -> Result<(), CatalogError> {
    for provider in &catalog.providers {
        for model in &provider.models {
            let context = format!("{}/{}", provider.id.as_str(), model.id.as_str());
            if model.context_window == Some(0) {
                return Err(CatalogError::InvalidField {
                    field: "context_window",
                    context,
                    reason: "context window must be greater than zero",
                });
            }
            if let Some(cost) = &model.cost {
                for (field, value) in [
                    ("input_per_million", cost.input_per_million),
                    ("output_per_million", cost.output_per_million),
                ] {
                    if !value.is_finite() || value < 0.0 {
                        return Err(CatalogError::InvalidField {
                            field,
                            context,
                            reason: "cost must be finite and non-negative",
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Applies a field-aware patch over a normalized catalog.
///
/// Exclusions are processed before additions/merges. Only fields a patch
/// supplies replace base values; the merged result is revalidated and returned
/// in lexicographic order.
pub fn apply_patch(
    base: NormalizedCatalog,
    patch: CatalogPatch,
) -> Result<NormalizedCatalog, CatalogError> {
    let schema_version = base.schema_version;
    let mut providers: BTreeMap<ProviderId, CatalogProvider> = base
        .providers
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect();

    // Pass 1: exclusions.
    for provider_patch in &patch.providers {
        if provider_patch.exclude {
            providers.remove(&provider_patch.id);
            continue;
        }
        if let Some(provider) = providers.get_mut(&provider_patch.id) {
            for model_patch in &provider_patch.models {
                if model_patch.exclude {
                    provider.models.retain(|m| m.id != model_patch.id);
                }
            }
        }
    }

    // Pass 2: additions and field merges.
    for provider_patch in patch.providers {
        if provider_patch.exclude {
            continue;
        }
        let provider = providers
            .entry(provider_patch.id.clone())
            .or_insert_with(|| CatalogProvider {
                id: provider_patch.id.clone(),
                name: provider_patch.id.as_str().to_string(),
                api_base_url: String::new(),
                env_vars: Vec::new(),
                unauthenticated: false,
                models: Vec::new(),
            });
        if let Some(name) = provider_patch.name {
            provider.name = name;
        }
        if let Some(api_base_url) = provider_patch.api_base_url {
            provider.api_base_url = api_base_url;
        }
        if let Some(mut env_vars) = provider_patch.env_vars {
            env_vars.sort();
            env_vars.dedup();
            provider.env_vars = env_vars;
        }
        if let Some(unauthenticated) = provider_patch.unauthenticated {
            provider.unauthenticated = unauthenticated;
        }
        for model_patch in provider_patch.models {
            if model_patch.exclude {
                continue;
            }
            let model = match provider.models.iter_mut().find(|m| m.id == model_patch.id) {
                Some(model) => model,
                None => {
                    provider.models.push(CatalogModel {
                        id: model_patch.id.clone(),
                        name: model_patch.id.as_str().to_string(),
                        protocol: Protocol::default(),
                        context_window: None,
                        reasoning: false,
                        cost: None,
                    });
                    provider.models.last_mut().expect("model just pushed")
                }
            };
            if let Some(name) = model_patch.name {
                model.name = name;
            }
            if let Some(protocol) = model_patch.protocol {
                model.protocol = protocol;
            }
            if let Some(context_window) = model_patch.context_window {
                model.context_window = Some(context_window);
            }
            if let Some(reasoning) = model_patch.reasoning {
                model.reasoning = reasoning;
            }
            if let Some(cost) = model_patch.cost {
                model.cost = Some(cost);
            }
        }
        provider.models.sort_by(|a, b| a.id.cmp(&b.id));
    }

    let merged = NormalizedCatalog {
        schema_version,
        providers: providers.into_values().collect(),
    };
    validate_merged(&merged)?;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CATALOG_SCHEMA_VERSION, CatalogModel, CatalogProvider, ModelId, Protocol, ProviderId,
    };

    fn catalog_with_openai(
        name: &str,
        api_base_url: &str,
        model_name: &str,
        context_window: u64,
    ) -> NormalizedCatalog {
        NormalizedCatalog {
            schema_version: CATALOG_SCHEMA_VERSION,
            providers: vec![CatalogProvider {
                id: ProviderId::new("openai").unwrap(),
                name: name.to_string(),
                api_base_url: api_base_url.to_string(),
                env_vars: vec!["OPENAI_API_KEY".to_string()],
                unauthenticated: false,
                models: vec![CatalogModel {
                    id: ModelId::new("gpt-5").unwrap(),
                    name: model_name.to_string(),
                    protocol: Protocol::Responses,
                    context_window: Some(context_window),
                    reasoning: true,
                    cost: None,
                }],
            }],
        }
    }

    fn catalog_with_two_models() -> NormalizedCatalog {
        let mut catalog =
            catalog_with_openai("OpenAI", "https://api.openai.com/v1", "GPT 5", 400_000);
        catalog.providers[0].models.push(CatalogModel {
            id: ModelId::new("legacy").unwrap(),
            name: "Legacy".to_string(),
            protocol: Protocol::ChatCompletions,
            context_window: Some(8_192),
            reasoning: false,
            cost: None,
        });
        catalog.providers[0].models.sort_by(|a, b| a.id.cmp(&b.id));
        catalog
    }

    fn patch_excluding(provider: &str, model: &str) -> CatalogPatch {
        CatalogPatch {
            providers: vec![ProviderPatch {
                id: ProviderId::new(provider).unwrap(),
                name: None,
                api_base_url: None,
                env_vars: None,
                unauthenticated: None,
                exclude: false,
                models: vec![ModelPatch {
                    id: ModelId::new(model).unwrap(),
                    name: None,
                    context_window: None,
                    protocol: None,
                    reasoning: None,
                    cost: None,
                    exclude: true,
                }],
            }],
        }
    }

    #[test]
    fn higher_layer_replaces_only_fields_it_supplies() {
        let base = catalog_with_openai("OpenAI", "https://api.openai.com/v1", "GPT 5", 400_000);
        let patch = CatalogPatch {
            providers: vec![ProviderPatch {
                id: ProviderId::new("openai").unwrap(),
                name: None,
                api_base_url: None,
                env_vars: None,
                unauthenticated: None,
                exclude: false,
                models: vec![ModelPatch {
                    id: ModelId::new("gpt-5").unwrap(),
                    name: Some("GPT-5".into()),
                    context_window: None,
                    protocol: None,
                    reasoning: None,
                    cost: None,
                    exclude: false,
                }],
            }],
        };
        let merged = apply_patch(base, patch).unwrap();
        let provider = merged.provider_str("openai").unwrap();
        assert_eq!(provider.api_base_url, "https://api.openai.com/v1");
        assert_eq!(provider.model_str("gpt-5").unwrap().name, "GPT-5");
        assert_eq!(
            provider.model_str("gpt-5").unwrap().context_window,
            Some(400_000)
        );
    }

    #[test]
    fn exclusions_remove_known_incompatible_entries() {
        let merged = apply_patch(
            catalog_with_two_models(),
            patch_excluding("openai", "legacy"),
        )
        .unwrap();
        assert!(
            merged
                .provider_str("openai")
                .unwrap()
                .model_str("legacy")
                .is_none()
        );
    }
}
