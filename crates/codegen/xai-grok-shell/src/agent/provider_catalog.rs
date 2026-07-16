//! Adapter between the dependency-neutral provider catalog
//! (`xai-grok-catalog`) and the shell's `ModelEntry` catalog.
//!
//! Conversion is secret-free: catalog snapshots never carry key material and
//! the produced [`ModelEntry`] has no `api_key`/`env_key`. Credentials are
//! resolved at call time by the credential seam in `agent::config`.

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use xai_grok_catalog::{
    CatalogError, CatalogModel, CatalogProvider, CatalogSnapshot, CredentialSources, Protocol,
    ProviderId, SecretString, resolve_credential,
};

use crate::agent::config::{ConfigProviderOverride, CredentialPolicy, ModelEntry, ModelInfo};
use crate::sampling::ApiBackend;

/// Converts a catalog provider/model pair into a shell [`ModelEntry`].
///
/// The entry uses the qualified ID `provider/model`, the catalog model ID as
/// the routing slug, the provider's base URL, and carries no secrets.
/// [`Protocol::Unsupported`] models are rejected.
pub fn model_entry_from_catalog(
    provider: &CatalogProvider,
    model: &CatalogModel,
) -> Result<ModelEntry, CatalogError> {
    let api_backend = match model.protocol {
        Protocol::ChatCompletions => ApiBackend::ChatCompletions,
        Protocol::Responses => ApiBackend::Responses,
        Protocol::Messages => ApiBackend::Messages,
        Protocol::Unsupported => {
            return Err(CatalogError::InvalidField {
                field: "protocol",
                context: format!("{}/{}", provider.id.as_str(), model.id.as_str()),
                reason: "protocol is not supported by this client",
            });
        }
    };
    let mut info = ModelInfo::fallback(model.id.as_str());
    info.id = Some(format!("{}/{}", provider.id.as_str(), model.id.as_str()));
    info.model = model.id.as_str().to_owned();
    info.base_url = provider.api_base_url.clone();
    info.name = Some(model.name.clone());
    info.api_backend = api_backend;
    if let Some(cw) = model.context_window.and_then(NonZeroU64::new) {
        info.context_window = cw;
    }
    info.supports_reasoning_effort = model.reasoning;
    Ok(ModelEntry {
        info,
        api_key: None,
        env_key: None,
        api_base_url: None,
        provider_id: Some(provider.id.clone()),
        credential_policy: if provider.unauthenticated {
            CredentialPolicy::None
        } else {
            CredentialPolicy::ProviderApiKey
        },
    })
}

/// Applies a user `[provider.<id>]` override onto a catalog provider.
///
/// `base_url` maps onto the neutral `CatalogProvider.api_base_url`. Applied
/// after embedded/runtime/override layering; explicit `[model.<qualified-id>]`
/// overrides still apply last via `ConfigModelOverride::apply`.
pub fn apply_config_provider_override(
    provider: &mut CatalogProvider,
    config: &ConfigProviderOverride,
) {
    if let Some(ref name) = config.name {
        provider.name.clone_from(name);
    }
    if let Some(ref base_url) = config.base_url {
        provider.api_base_url.clone_from(base_url);
    }
    if let Some(ref env_key) = config.env_key {
        provider.env_vars = env_key.names().iter().map(|s| (*s).to_owned()).collect();
    }
    if let Some(unauthenticated) = config.unauthenticated {
        provider.unauthenticated = unauthenticated;
    }
}

/// Shell-side handle to the catalog manager plus call-time credential
/// resolution. Holds no secrets: session keys live only for lookup and the
/// resolved secret is returned to the caller, never stored in snapshots.
pub struct ProviderCatalogAdapter {
    manager: xai_grok_catalog::CatalogManager,
    grok_home: PathBuf,
    session_keys: parking_lot::RwLock<std::collections::HashMap<String, String>>,
}

impl ProviderCatalogAdapter {
    /// Builds an adapter over a catalog manager rooted at `grok_home`.
    pub fn new(manager: xai_grok_catalog::CatalogManager, grok_home: PathBuf) -> Self {
        Self {
            manager,
            grok_home,
            session_keys: parking_lot::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Current immutable catalog snapshot.
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        self.manager.snapshot()
    }

    /// Records a session-scoped provider API key (highest precedence).
    pub fn set_session_key(&self, provider_id: &ProviderId, api_key: String) {
        self.session_keys
            .write()
            .insert(provider_id.as_str().to_owned(), api_key);
    }

    /// Resolves the credential for `provider_id` at call time with precedence
    /// session > stored > environment. Model-inline credentials are resolved
    /// by the per-entry credential seam, not here. The returned secret is
    /// never retained by the adapter.
    pub fn credential_for(&self, provider_id: &ProviderId) -> Option<SecretString> {
        let snapshot = self.manager.snapshot();
        let provider = snapshot.catalog().provider(provider_id);
        let session = self
            .session_keys
            .read()
            .get(provider_id.as_str())
            .map(SecretString::new);
        let stored = crate::auth::read_provider_api_key(&self.grok_home, provider_id.as_str())
            .map(SecretString::new);
        let environment = provider.and_then(|p| {
            crate::agent::config::EnvKeys::new(p.env_vars.iter().cloned())
                .resolve_value()
                .map(SecretString::new)
        });
        resolve_credential(CredentialSources {
            session,
            stored,
            environment,
            model: None,
        })
        .map(|resolved| resolved.secret)
    }
}
