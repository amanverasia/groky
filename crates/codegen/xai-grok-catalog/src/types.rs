//! Validated, secret-free provider/model catalog DTOs.

use serde::{Deserialize, Deserializer, Serialize};

/// Current catalog schema version.
pub const CATALOG_SCHEMA_VERSION: u32 = 1;

/// Maximum byte length of a provider ID.
pub const MAX_PROVIDER_ID_BYTES: usize = 64;

/// Maximum byte length of a model ID.
pub const MAX_MODEL_ID_BYTES: usize = 512;

/// Errors produced by catalog validation, normalization, and layering.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// A provider ID failed validation.
    #[error("invalid provider id {value:?}: {reason}")]
    InvalidProviderId {
        /// The offending value.
        value: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A model ID failed validation.
    #[error("invalid model id {value:?}: {reason}")]
    InvalidModelId {
        /// The offending value.
        value: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A normalization limit was exceeded; the whole document is rejected.
    #[error("limit exceeded for {field}: {actual} exceeds {limit}")]
    LimitExceeded {
        /// The limited quantity.
        field: &'static str,
        /// The configured limit.
        limit: usize,
        /// The observed value.
        actual: usize,
    },
    /// The raw document could not be parsed.
    #[error("invalid catalog document: {0}")]
    InvalidDocument(String),
    /// A network refresh failed; the message is concise status text and
    /// never embeds response bodies.
    #[error("catalog fetch failed: {0}")]
    Fetch(String),
    /// The on-disk catalog cache could not be read, validated, or written.
    #[error("catalog cache error: {0}")]
    Cache(String),
    /// A field value failed validation.
    #[error("invalid {field} in {context}: {reason}")]
    InvalidField {
        /// Field name.
        field: &'static str,
        /// Provider/model context for the failure.
        context: String,
        /// Why it was rejected.
        reason: &'static str,
    },
}

/// Validated provider identifier: non-empty, `[a-z0-9_-]`, at most 64 bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    /// Validates and constructs a provider ID.
    pub fn new(value: impl Into<String>) -> Result<Self, CatalogError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CatalogError::InvalidProviderId {
                value,
                reason: "must not be empty",
            });
        }
        if value.len() > MAX_PROVIDER_ID_BYTES {
            return Err(CatalogError::InvalidProviderId {
                value,
                reason: "must be at most 64 bytes",
            });
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(CatalogError::InvalidProviderId {
                value,
                reason: "must contain only [a-z0-9_-]",
            });
        }
        Ok(Self(value))
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Validated model identifier: non-empty, no control characters, at most 512 bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    /// Validates and constructs a model ID.
    pub fn new(value: impl Into<String>) -> Result<Self, CatalogError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CatalogError::InvalidModelId {
                value,
                reason: "must not be empty",
            });
        }
        if value.len() > MAX_MODEL_ID_BYTES {
            return Err(CatalogError::InvalidModelId {
                value,
                reason: "must be at most 512 bytes",
            });
        }
        if value.chars().any(char::is_control) {
            return Err(CatalogError::InvalidModelId {
                value,
                reason: "must not contain control characters",
            });
        }
        Ok(Self(value))
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ModelId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Wire protocol used to talk to a provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// OpenAI-compatible chat completions.
    #[default]
    ChatCompletions,
    /// OpenAI Responses API.
    Responses,
    /// Anthropic Messages API.
    Messages,
    /// Protocol not supported by this client; excluded during normalization.
    Unsupported,
}

/// Per-million-token model pricing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    /// Input cost per million tokens (USD).
    pub input_per_million: f64,
    /// Output cost per million tokens (USD).
    pub output_per_million: f64,
}

/// A normalized, secret-free model entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogModel {
    /// Model identifier.
    pub id: ModelId,
    /// Human-readable display name.
    pub name: String,
    /// Wire protocol for this model.
    pub protocol: Protocol,
    /// Context window in tokens, when known.
    pub context_window: Option<u64>,
    /// Whether the model supports reasoning.
    pub reasoning: bool,
    /// Token pricing, when known.
    pub cost: Option<ModelCost>,
}

/// A normalized, secret-free provider entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogProvider {
    /// Provider identifier.
    pub id: ProviderId,
    /// Human-readable display name.
    pub name: String,
    /// API base URL.
    pub api_base_url: String,
    /// API-key environment variable names, sorted and deduplicated.
    pub env_vars: Vec<String>,
    /// Whether the provider allows unauthenticated use.
    pub unauthenticated: bool,
    /// Models offered by this provider, sorted by ID.
    pub models: Vec<CatalogModel>,
}

impl CatalogProvider {
    /// Looks up a model by validated ID.
    pub fn model(&self, id: &ModelId) -> Option<&CatalogModel> {
        self.models.iter().find(|m| &m.id == id)
    }

    /// Looks up a model by raw string ID.
    pub fn model_str(&self, id: &str) -> Option<&CatalogModel> {
        self.models.iter().find(|m| m.id.as_str() == id)
    }
}

/// A complete normalized catalog, sorted deterministically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedCatalog {
    /// Schema version of this catalog document.
    pub schema_version: u32,
    /// Providers sorted lexicographically by ID.
    pub providers: Vec<CatalogProvider>,
}

impl NormalizedCatalog {
    /// Looks up a provider by validated ID.
    pub fn provider(&self, id: &ProviderId) -> Option<&CatalogProvider> {
        self.providers.iter().find(|p| &p.id == id)
    }

    /// Looks up a provider by raw string ID.
    pub fn provider_str(&self, id: &str) -> Option<&CatalogProvider> {
        self.providers.iter().find(|p| p.id.as_str() == id)
    }
}

/// Bounds applied while normalizing a raw models.dev document.
///
/// Exceeding any bound rejects the whole document.
#[derive(Clone, Copy, Debug)]
pub struct NormalizationLimits {
    /// Maximum raw document size in bytes.
    pub max_bytes: usize,
    /// Maximum number of providers.
    pub max_providers: usize,
    /// Maximum number of models per provider.
    pub max_models_per_provider: usize,
    /// Maximum byte length of any normalized string field.
    pub max_string_bytes: usize,
}

impl Default for NormalizationLimits {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024,
            max_providers: 512,
            max_models_per_provider: 4096,
            max_string_bytes: 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_rejects_invalid_characters_and_lengths() {
        assert!(ProviderId::new("openai").is_ok());
        assert!(ProviderId::new("open_ai-2").is_ok());
        assert!(ProviderId::new("").is_err());
        assert!(ProviderId::new("OpenAI").is_err());
        assert!(ProviderId::new("open ai").is_err());
        assert!(ProviderId::new("a".repeat(65)).is_err());
        assert!(ProviderId::new("a".repeat(64)).is_ok());
    }

    #[test]
    fn model_id_rejects_control_characters_and_over_length() {
        assert!(ModelId::new("gpt-5").is_ok());
        assert!(ModelId::new("Claude Sonnet 4").is_ok());
        assert!(ModelId::new("").is_err());
        assert!(ModelId::new("bad\nid").is_err());
        assert!(ModelId::new("a".repeat(513)).is_err());
        assert!(ModelId::new("a".repeat(512)).is_ok());
    }

    #[test]
    fn id_deserialization_goes_through_validation() {
        assert!(serde_json::from_str::<ProviderId>(r#""UPPER""#).is_err());
        assert!(serde_json::from_str::<ProviderId>(r#""openai""#).is_ok());
        assert!(serde_json::from_str::<ModelId>(r#""bad\u0000id""#).is_err());
        assert!(serde_json::from_str::<ModelId>(r#""gpt-5""#).is_ok());
    }
}
