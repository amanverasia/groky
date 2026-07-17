//! Bounded, deterministic normalization of raw models.dev documents.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::types::{
    CATALOG_SCHEMA_VERSION, CatalogError, CatalogModel, CatalogProvider, ModelCost, ModelId,
    NormalizationLimits, NormalizedCatalog, Protocol, ProviderId,
};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawProvider {
    name: String,
    api: String,
    env: Vec<String>,
    npm: String,
    models: BTreeMap<String, RawModel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawModel {
    name: String,
    reasoning: bool,
    limit: RawLimit,
    cost: Option<RawCost>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawLimit {
    context: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawCost {
    input: f64,
    output: f64,
}

/// A typed local diagnostic for entries excluded during normalization.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ExcludedEntry {
    UnsupportedProtocol { provider_id: String, npm: String },
    InvalidProviderId { provider_id: String },
}

/// npm SDK package → wire protocol, grounded in each provider's documented
/// API. ChatCompletions entries expose an OpenAI-compatible
/// `/chat/completions` endpoint. SDKs speaking proprietary or cloud-scoped
/// protocols (Google, Vertex, Bedrock, Azure deployments, Cohere, gateways)
/// are intentionally absent and normalize to `Unsupported`.
const NPM_PROTOCOLS: &[(&str, Protocol)] = &[
    ("@ai-sdk/anthropic", Protocol::Messages),
    ("@ai-sdk/cerebras", Protocol::ChatCompletions),
    ("@ai-sdk/deepinfra", Protocol::ChatCompletions),
    ("@ai-sdk/deepseek", Protocol::ChatCompletions),
    ("@ai-sdk/fireworks", Protocol::ChatCompletions),
    ("@ai-sdk/groq", Protocol::ChatCompletions),
    ("@ai-sdk/mistral", Protocol::ChatCompletions),
    ("@ai-sdk/openai", Protocol::Responses),
    ("@ai-sdk/openai-compatible", Protocol::ChatCompletions),
    ("@ai-sdk/perplexity", Protocol::ChatCompletions),
    ("@ai-sdk/togetherai", Protocol::ChatCompletions),
    ("@ai-sdk/xai", Protocol::ChatCompletions),
    ("@openrouter/ai-sdk-provider", Protocol::ChatCompletions),
];

fn protocol_from_npm(npm: &str) -> Protocol {
    NPM_PROTOCOLS
        .iter()
        .find(|(pkg, _)| *pkg == npm)
        .map(|(_, protocol)| *protocol)
        .unwrap_or(Protocol::Unsupported)
}

fn check_string(
    value: &str,
    field: &'static str,
    limits: &NormalizationLimits,
) -> Result<(), CatalogError> {
    if value.len() > limits.max_string_bytes {
        return Err(CatalogError::LimitExceeded {
            field,
            limit: limits.max_string_bytes,
            actual: value.len(),
        });
    }
    Ok(())
}

fn validate_cost(cost: &ModelCost, context: &str) -> Result<(), CatalogError> {
    for (field, value) in [
        ("input_per_million", cost.input_per_million),
        ("output_per_million", cost.output_per_million),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(CatalogError::InvalidField {
                field,
                context: context.to_string(),
                reason: "cost must be finite and non-negative",
            });
        }
    }
    Ok(())
}

fn normalize_context_window(window: Option<u64>) -> Option<u64> {
    // Real models.dev documents contain `context: 0` placeholders; a zero
    // window is meaningless, so it is normalized to unknown rather than
    // rejecting the whole document.
    window.filter(|&w| w > 0)
}

/// Parses and normalizes a raw models.dev document.
///
/// Any limit violation or invalid field rejects the whole document. Providers
/// whose protocol is unsupported are excluded with a local diagnostic. Output
/// ordering is deterministic: providers and models are sorted lexicographically
/// by ID and environment variable names are sorted and deduplicated.
pub fn normalize_models_dev(
    raw: &[u8],
    limits: NormalizationLimits,
) -> Result<NormalizedCatalog, CatalogError> {
    if raw.len() > limits.max_bytes {
        return Err(CatalogError::LimitExceeded {
            field: "bytes",
            limit: limits.max_bytes,
            actual: raw.len(),
        });
    }

    let raw_providers: BTreeMap<String, RawProvider> = serde_json::from_slice(raw)
        .map_err(|err| CatalogError::InvalidDocument(err.to_string()))?;

    if raw_providers.len() > limits.max_providers {
        return Err(CatalogError::LimitExceeded {
            field: "providers",
            limit: limits.max_providers,
            actual: raw_providers.len(),
        });
    }

    let mut excluded: Vec<ExcludedEntry> = Vec::new();
    let mut providers: Vec<CatalogProvider> = Vec::new();

    for (raw_id, raw_provider) in raw_providers {
        let protocol = protocol_from_npm(&raw_provider.npm);
        if protocol == Protocol::Unsupported {
            excluded.push(ExcludedEntry::UnsupportedProtocol {
                provider_id: raw_id,
                npm: raw_provider.npm,
            });
            continue;
        }

        let id = match ProviderId::new(raw_id) {
            Ok(id) => id,
            Err(err) => {
                // Real models.dev documents contain provider keys outside the
                // validated ID alphabet (for example, `wafer.ai`); such
                // providers are excluded locally instead of rejecting the
                // whole document.
                let provider_id = match err {
                    CatalogError::InvalidProviderId { value, .. } => value,
                    _ => String::new(),
                };
                excluded.push(ExcludedEntry::InvalidProviderId { provider_id });
                continue;
            }
        };
        let name = if raw_provider.name.is_empty() {
            id.as_str().to_string()
        } else {
            raw_provider.name
        };
        check_string(&name, "provider.name", &limits)?;
        check_string(&raw_provider.api, "provider.api_base_url", &limits)?;

        if raw_provider.models.len() > limits.max_models_per_provider {
            return Err(CatalogError::LimitExceeded {
                field: "models",
                limit: limits.max_models_per_provider,
                actual: raw_provider.models.len(),
            });
        }

        let mut env_vars = raw_provider.env;
        for var in &env_vars {
            check_string(var, "provider.env_var", &limits)?;
        }
        env_vars.sort();
        env_vars.dedup();

        let mut models: Vec<CatalogModel> = Vec::new();
        for (raw_model_id, raw_model) in raw_provider.models {
            let model_id = ModelId::new(raw_model_id)?;
            let context = format!("{}/{}", id.as_str(), model_id.as_str());
            let model_name = if raw_model.name.is_empty() {
                model_id.as_str().to_string()
            } else {
                raw_model.name
            };
            check_string(&model_name, "model.name", &limits)?;
            let context_window = normalize_context_window(raw_model.limit.context);
            let cost = raw_model.cost.map(|c| ModelCost {
                input_per_million: c.input,
                output_per_million: c.output,
            });
            if let Some(cost) = &cost {
                validate_cost(cost, &context)?;
            }
            models.push(CatalogModel {
                id: model_id,
                name: model_name,
                protocol,
                context_window,
                reasoning: raw_model.reasoning,
                cost,
            });
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));

        providers.push(CatalogProvider {
            id,
            name,
            api_base_url: raw_provider.api,
            env_vars,
            unauthenticated: false,
            models,
        });
    }
    drop(excluded);

    providers.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(NormalizedCatalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        providers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_fields_and_ignores_unknown_fields() {
        let raw = include_bytes!("../tests/fixtures/models-dev-small.json");
        let catalog = normalize_models_dev(raw, NormalizationLimits::default()).unwrap();
        assert_eq!(
            catalog
                .providers
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["anthropic", "openai"]
        );
        let gpt = catalog
            .provider_str("openai")
            .unwrap()
            .model_str("gpt-5")
            .unwrap();
        assert_eq!(gpt.protocol, Protocol::Responses);
        assert_eq!(gpt.context_window, Some(400_000));
        assert_eq!(gpt.cost.as_ref().unwrap().input_per_million, 1.25);
        assert!(gpt.reasoning);
    }

    #[test]
    fn rejects_the_whole_document_when_limits_are_exceeded() {
        let err = normalize_models_dev(
            br#"{"p":{"id":"p","name":"p","models":{}}}"#,
            NormalizationLimits {
                max_providers: 0,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CatalogError::LimitExceeded {
                field: "providers",
                ..
            }
        ));
    }

    #[test]
    fn excludes_unsupported_protocol_providers() {
        let raw = include_bytes!("../tests/fixtures/models-dev-small.json");
        let catalog = normalize_models_dev(raw, NormalizationLimits::default()).unwrap();
        assert!(catalog.provider_str("unsupported").is_none());
    }

    #[test]
    fn maps_openai_compatible_sdks_to_chat_completions() {
        for npm in [
            "@openrouter/ai-sdk-provider",
            "@ai-sdk/groq",
            "@ai-sdk/mistral",
            "@ai-sdk/deepseek",
            "@ai-sdk/cerebras",
            "@ai-sdk/togetherai",
            "@ai-sdk/fireworks",
            "@ai-sdk/xai",
            "@ai-sdk/deepinfra",
            "@ai-sdk/perplexity",
        ] {
            let raw = format!(
                r#"{{"p":{{"name":"P","api":"https://p.example/v1","env":[],"npm":"{npm}","models":{{"m":{{"name":"M"}}}}}}}}"#
            );
            let catalog =
                normalize_models_dev(raw.as_bytes(), NormalizationLimits::default()).unwrap();
            let model = catalog.provider_str("p").unwrap().model_str("m").unwrap();
            assert_eq!(model.protocol, Protocol::ChatCompletions, "npm: {npm}");
        }
    }

    #[test]
    fn keeps_incompatible_sdks_unsupported() {
        for npm in [
            "@ai-sdk/google",
            "@ai-sdk/google-vertex",
            "@ai-sdk/google-vertex/anthropic",
            "@ai-sdk/amazon-bedrock",
            "@ai-sdk/azure",
            "@ai-sdk/cohere",
        ] {
            let raw = format!(
                r#"{{"p":{{"name":"P","api":"https://p.example/v1","env":[],"npm":"{npm}","models":{{}}}}}}"#
            );
            let catalog =
                normalize_models_dev(raw.as_bytes(), NormalizationLimits::default()).unwrap();
            assert!(catalog.provider_str("p").is_none(), "npm: {npm}");
        }
    }

    #[test]
    fn dedupes_and_sorts_env_vars() {
        let raw = include_bytes!("../tests/fixtures/models-dev-small.json");
        let catalog = normalize_models_dev(raw, NormalizationLimits::default()).unwrap();
        let openai = catalog.provider_str("openai").unwrap();
        assert_eq!(openai.env_vars, ["OPENAI_API_KEY"]);
    }

    #[test]
    fn treats_zero_context_window_as_unknown() {
        let raw = br#"{"p":{"name":"P","api":"https://p.example/v1","env":[],"npm":"@ai-sdk/openai-compatible","models":{"m":{"name":"M","limit":{"context":0}}}}}"#;
        let catalog = normalize_models_dev(raw, NormalizationLimits::default()).unwrap();
        let model = catalog.provider_str("p").unwrap().model_str("m").unwrap();
        assert_eq!(model.context_window, None);
    }

    #[test]
    fn excludes_providers_with_invalid_ids() {
        let raw = br#"{"wafer.ai":{"name":"Wafer","api":"https://w.example/v1","env":[],"npm":"@ai-sdk/openai-compatible","models":{}},"ok":{"name":"Ok","api":"https://ok.example/v1","env":[],"npm":"@ai-sdk/openai-compatible","models":{}}}"#;
        let catalog = normalize_models_dev(raw, NormalizationLimits::default()).unwrap();
        assert_eq!(
            catalog
                .providers
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["ok"]
        );
    }

    #[test]
    fn rejects_oversize_documents() {
        let raw = include_bytes!("../tests/fixtures/models-dev-small.json");
        let err = normalize_models_dev(
            raw,
            NormalizationLimits {
                max_bytes: 8,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CatalogError::LimitExceeded { field: "bytes", .. }
        ));
    }
}
