//! Parsing of OpenAI-style `/models` discovery responses.
//!
//! Discovery responses are untrusted remote input: body size, model count,
//! and per-field lengths are all bounded by [`crate::limits`], and model IDs
//! are preserved verbatim (never rewritten) so gateway-prefixed IDs like
//! `openai/gpt-4o` round-trip exactly.

use crate::limits::{
    MAX_DISCOVERED_MODELS, MAX_DISCOVERY_BODY_BYTES, MAX_MODEL_ID_BYTES, MAX_MODEL_NAME_BYTES,
};
use crate::types::ModelId;

/// A model discovered from a provider's model-list endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredModel {
    /// Model identifier, preserved verbatim from the response.
    pub id: ModelId,
    /// Optional display name from the response.
    pub name: Option<String>,
}

impl DiscoveredModel {
    /// Validates and constructs a discovered model with no display name.
    pub fn new(id: impl Into<String>) -> Result<Self, DiscoveryError> {
        let id = id.into();
        if id.len() > MAX_MODEL_ID_BYTES {
            return Err(DiscoveryError::ModelIdTooLong { index: 0 });
        }
        let id = ModelId::new(id).map_err(|err| DiscoveryError::InvalidModelId {
            index: 0,
            reason: err.to_string(),
        })?;
        Ok(Self { id, name: None })
    }
}

/// Errors produced while parsing a model-discovery response.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    /// Response body exceeds [`MAX_DISCOVERY_BODY_BYTES`].
    #[error("model-list response exceeds 2097152 bytes")]
    BodyTooLarge,
    /// Response body is not valid JSON.
    #[error("model-list response is not valid JSON: {0}")]
    InvalidJson(String),
    /// Response lacks the required `data` array.
    #[error("model-list response must contain a data array")]
    MissingData,
    /// Response lists more than [`MAX_DISCOVERED_MODELS`] models.
    #[error("model-list response exceeds 2000 models")]
    TooManyModels,
    /// A model entry lacks a string `id`.
    #[error("model at index {index} is missing a string id")]
    MissingModelId {
        /// Index of the offending entry in `data`.
        index: usize,
    },
    /// A model ID exceeds [`MAX_MODEL_ID_BYTES`].
    #[error("model id at index {index} exceeds 512 bytes")]
    ModelIdTooLong {
        /// Index of the offending entry in `data`.
        index: usize,
    },
    /// A model name exceeds [`crate::limits::MAX_MODEL_NAME_BYTES`].
    #[error("model name at index {index} exceeds 512 bytes")]
    ModelNameTooLong {
        /// Index of the offending entry in `data`.
        index: usize,
    },
    /// A model ID failed [`ModelId`] validation.
    #[error("invalid model id at index {index}: {reason}")]
    InvalidModelId {
        /// Index of the offending entry in `data`.
        index: usize,
        /// Why it was rejected.
        reason: String,
    },
}

/// Parses an OpenAI-style model-list response body into discovered models.
///
/// Enforces body-size, model-count, and per-field length bounds; ignores
/// unknown fields; preserves model IDs verbatim; and collapses duplicate IDs
/// in first-seen order so a server cannot inflate counts past validation.
pub fn parse_model_list(body: &[u8]) -> Result<Vec<DiscoveredModel>, DiscoveryError> {
    if body.len() > MAX_DISCOVERY_BODY_BYTES {
        return Err(DiscoveryError::BodyTooLarge);
    }
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|err| DiscoveryError::InvalidJson(err.to_string()))?;
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or(DiscoveryError::MissingData)?;
    if data.len() > MAX_DISCOVERED_MODELS {
        return Err(DiscoveryError::TooManyModels);
    }

    let mut seen = std::collections::HashSet::with_capacity(data.len());
    let mut models = Vec::with_capacity(data.len());
    for (index, entry) in data.iter().enumerate() {
        let id = entry
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or(DiscoveryError::MissingModelId { index })?;
        if id.len() > MAX_MODEL_ID_BYTES {
            return Err(DiscoveryError::ModelIdTooLong { index });
        }
        let id = ModelId::new(id).map_err(|err| DiscoveryError::InvalidModelId {
            index,
            reason: err.to_string(),
        })?;
        let name = match entry.get("name").and_then(serde_json::Value::as_str) {
            Some(name) if name.len() > MAX_MODEL_NAME_BYTES => {
                return Err(DiscoveryError::ModelNameTooLong { index });
            }
            Some(name) => Some(name.to_string()),
            None => None,
        };
        if seen.insert(id.clone()) {
            models.push(DiscoveredModel { id, name });
        }
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_list_without_rewriting_ids() {
        let body = include_bytes!("../tests/fixtures/openai-models.json");
        let models = parse_model_list(body).expect("fixture parses");
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            ["openai/gpt-4o", "best-effort", "anthropic/claude-3-5-sonnet"]
        );
    }

    #[test]
    fn rejects_body_above_size_bound() {
        let body = vec![b' '; MAX_DISCOVERY_BODY_BYTES + 1];
        assert_eq!(
            parse_model_list(&body).unwrap_err(),
            DiscoveryError::BodyTooLarge
        );
    }

    #[test]
    fn rejects_missing_data_array() {
        assert_eq!(
            parse_model_list(br#"{"object":"list"}"#).unwrap_err(),
            DiscoveryError::MissingData
        );
    }

    #[test]
    fn rejects_model_count_above_bound() {
        let entries: Vec<String> = (0..=MAX_DISCOVERED_MODELS)
            .map(|i| format!(r#"{{"id":"model-{i}"}}"#))
            .collect();
        let body = format!(r#"{{"object":"list","data":[{}]}}"#, entries.join(","));
        assert_eq!(
            parse_model_list(body.as_bytes()).unwrap_err(),
            DiscoveryError::TooManyModels
        );
    }

    #[test]
    fn rejects_overlong_id_and_name() {
        let overlong_id = format!(r#"{{"data":[{{"id":"{}"}}]}}"#, "a".repeat(513));
        assert_eq!(
            parse_model_list(overlong_id.as_bytes()).unwrap_err(),
            DiscoveryError::ModelIdTooLong { index: 0 }
        );

        let overlong_name = format!(
            r#"{{"data":[{{"id":"ok","name":"{}"}}]}}"#,
            "n".repeat(513)
        );
        assert_eq!(
            parse_model_list(overlong_name.as_bytes()).unwrap_err(),
            DiscoveryError::ModelNameTooLong { index: 0 }
        );
    }

    #[test]
    fn rejects_missing_model_id() {
        assert_eq!(
            parse_model_list(br#"{"data":[{"id":"ok"},{"object":"model"}]}"#).unwrap_err(),
            DiscoveryError::MissingModelId { index: 1 }
        );
    }

    #[test]
    fn collapses_duplicate_ids_in_first_seen_order() {
        let body = br#"{"data":[
            {"id":"b","name":"first"},
            {"id":"a"},
            {"id":"b","name":"second"}
        ]}"#;
        let models = parse_model_list(body).expect("duplicates parse");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id.as_str(), "b");
        assert_eq!(models[0].name.as_deref(), Some("first"));
        assert_eq!(models[1].id.as_str(), "a");
    }
}
