//! Redacted credential resolution and provider availability.
//!
//! Secrets live only in [`SecretString`], which has no serde support, redacts
//! `Debug`/`Display`, and zeroes its bytes on drop. Status DTOs carry only
//! provider IDs and [`CredentialOrigin`], never key material.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::CatalogProvider;

/// A provider API key held in memory.
///
/// Deliberately implements neither `Serialize` nor `Deserialize`; the only
/// way to read the value is [`SecretString::expose`]. `Debug`/`Display` are
/// redacted, and the bytes are zeroed on drop.
pub struct SecretString(Vec<u8>);

impl SecretString {
    /// Wraps a secret value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().into_bytes())
    }

    /// Returns the secret value. The only accessor; call sites should be
    /// limited to constructing outbound authorization headers.
    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.0).expect("SecretString is constructed from a String")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // Volatile so the zeroing store is not elided as a dead write.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString([REDACTED])")
    }
}

/// Where a resolved credential came from. Safe to serialize; contains no
/// key material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialOrigin {
    /// Key stored in `auth.json`.
    Stored,
    /// Key read from a provider environment variable.
    Environment,
    /// Key supplied inline by model configuration (lowest precedence).
    Model,
}

/// Candidate credentials for one provider, one slot per origin.
#[derive(Debug, Default)]
pub struct CredentialSources {
    /// Key from stored auth.
    pub stored: Option<SecretString>,
    /// Key from the environment.
    pub environment: Option<SecretString>,
    /// Key from model configuration.
    pub model: Option<SecretString>,
}

/// The winning credential and its origin.
#[derive(Debug)]
pub struct ResolvedCredential {
    /// The secret key material.
    pub secret: SecretString,
    /// Which source won.
    pub origin: CredentialOrigin,
}

/// Resolves credential sources with precedence
/// stored > environment > model.
pub fn resolve_credential(sources: CredentialSources) -> Option<ResolvedCredential> {
    let CredentialSources {
        stored,
        environment,
        model,
    } = sources;
    let candidates = [
        (stored, CredentialOrigin::Stored),
        (environment, CredentialOrigin::Environment),
        (model, CredentialOrigin::Model),
    ];
    candidates
        .into_iter()
        .find_map(|(secret, origin)| secret.map(|secret| ResolvedCredential { secret, origin }))
}

/// Availability classification for a provider, derived from its catalog
/// entry and the resolved credential origin. Contains no secrets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAvailability {
    /// Usable: unauthenticated, or keyed via a stored/model credential.
    Configured,
    /// Usable via an environment-variable key.
    Environment,
    /// Supported but no credential is available.
    MissingKey,
    /// No supported models; cannot be used by this client.
    Unavailable,
}

/// Classifies a provider from its catalog entry and resolved credential
/// origin. Providers with no supported models are [`ProviderAvailability::Unavailable`];
/// unauthenticated providers are always [`ProviderAvailability::Configured`].
pub fn classify_provider(
    provider: &CatalogProvider,
    origin: Option<CredentialOrigin>,
) -> ProviderAvailability {
    if provider.models.is_empty() {
        return ProviderAvailability::Unavailable;
    }
    if provider.unauthenticated {
        return ProviderAvailability::Configured;
    }
    match origin {
        Some(CredentialOrigin::Environment) => ProviderAvailability::Environment,
        Some(_) => ProviderAvailability::Configured,
        None => ProviderAvailability::MissingKey,
    }
}

/// Secret-free provider status DTO for UI and ACP surfaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    /// Provider identifier.
    pub provider_id: String,
    /// Availability classification.
    pub availability: ProviderAvailability,
    /// Origin of the winning credential, when one exists.
    pub origin: Option<CredentialOrigin>,
}

impl ProviderStatus {
    /// Status for a provider configured via a stored credential.
    pub fn configured(provider_id: &str) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            availability: ProviderAvailability::Configured,
            origin: Some(CredentialOrigin::Stored),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CatalogModel, CatalogProvider, ModelId, Protocol, ProviderId};

    fn provider(unauthenticated: bool, models: Vec<CatalogModel>) -> CatalogProvider {
        CatalogProvider {
            id: ProviderId::new("openai").unwrap(),
            name: "OpenAI".to_string(),
            api_base_url: "https://api.openai.com/v1".to_string(),
            env_vars: vec!["OPENAI_API_KEY".to_string()],
            unauthenticated,
            models,
        }
    }

    fn model() -> CatalogModel {
        CatalogModel {
            id: ModelId::new("gpt-5").unwrap(),
            name: "GPT-5".to_string(),
            protocol: Protocol::ChatCompletions,
            context_window: None,
            reasoning: false,
            cost: None,
        }
    }

    fn supported() -> CatalogProvider {
        provider(false, vec![model()])
    }

    fn unsupported() -> CatalogProvider {
        provider(false, Vec::new())
    }

    #[test]
    fn credential_precedence_is_stored_environment_then_model() {
        let sources = CredentialSources {
            stored: Some(SecretString::new("stored")),
            environment: Some(SecretString::new("env")),
            model: Some(SecretString::new("model")),
        };
        assert_eq!(
            resolve_credential(sources).unwrap().origin,
            CredentialOrigin::Stored
        );
    }

    #[test]
    fn precedence_falls_through_in_order_when_higher_sources_are_absent() {
        let stored_only = CredentialSources {
            stored: Some(SecretString::new("stored")),
            environment: Some(SecretString::new("env")),
            model: Some(SecretString::new("model")),
            ..Default::default()
        };
        assert_eq!(
            resolve_credential(stored_only).unwrap().origin,
            CredentialOrigin::Stored
        );

        let env_only = CredentialSources {
            environment: Some(SecretString::new("env")),
            model: Some(SecretString::new("model")),
            ..Default::default()
        };
        assert_eq!(
            resolve_credential(env_only).unwrap().origin,
            CredentialOrigin::Environment
        );

        let model_only = CredentialSources {
            model: Some(SecretString::new("model")),
            ..Default::default()
        };
        assert_eq!(
            resolve_credential(model_only).unwrap().origin,
            CredentialOrigin::Model
        );

        assert!(resolve_credential(CredentialSources::default()).is_none());
    }

    #[test]
    fn resolved_credential_exposes_the_winning_secret() {
        let sources = CredentialSources {
            stored: Some(SecretString::new("stored-key")),
            ..Default::default()
        };
        let resolved = resolve_credential(sources).unwrap();
        assert_eq!(resolved.secret.expose(), "stored-key");
    }

    #[test]
    fn secret_display_and_debug_never_expose_value() {
        let secret = SecretString::new("sk-provider-secret");
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert_eq!(format!("{secret:?}"), "SecretString([REDACTED])");
        assert!(
            !serde_json::to_string(&ProviderStatus::configured("openai"))
                .unwrap()
                .contains("secret")
        );
    }

    #[test]
    fn availability_distinguishes_stored_environment_missing_and_unsupported() {
        assert_eq!(
            classify_provider(&supported(), Some(CredentialOrigin::Stored)),
            ProviderAvailability::Configured
        );
        assert_eq!(
            classify_provider(&supported(), Some(CredentialOrigin::Environment)),
            ProviderAvailability::Environment
        );
        assert_eq!(
            classify_provider(&supported(), None),
            ProviderAvailability::MissingKey
        );
        assert_eq!(
            classify_provider(&unsupported(), None),
            ProviderAvailability::Unavailable
        );
    }

    #[test]
    fn unauthenticated_providers_are_configured_without_a_key() {
        let free = provider(true, vec![model()]);
        assert_eq!(
            classify_provider(&free, None),
            ProviderAvailability::Configured
        );
    }
}
