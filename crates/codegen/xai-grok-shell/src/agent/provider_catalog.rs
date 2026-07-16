//! Adapter between the dependency-neutral provider catalog
//! (`xai-grok-catalog`) and the shell's `ModelEntry` catalog.
//!
//! Conversion is secret-free: catalog snapshots never carry key material and
//! the produced [`ModelEntry`] has no `api_key`/`env_key`. Credentials are
//! resolved at call time by the credential seam in `agent::config`.

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;
use xai_grok_catalog::{
    CatalogError, CatalogModel, CatalogProvider, CatalogSnapshot, CredentialOrigin,
    CredentialSources, NormalizedCatalog, Protocol, ProviderAvailability, ProviderId, SecretString,
    classify_provider, resolve_credential,
};

use crate::agent::config::{
    Config, ConfigProviderOverride, CredentialPolicy, EnvKeys, ModelEntry, ModelInfo,
    ProviderModelMeta,
};
use crate::sampling::ApiBackend;

/// File name of the provider catalog cache under `$GROK_HOME`. Distinct from
/// the xAI entitlement cache (`models_cache.json`, five-minute TTL).
pub const PROVIDER_CATALOG_CACHE_FILE: &str = "provider_catalog.json";

/// Upstream source of the provider catalog (24-hour freshness).
pub const PROVIDER_CATALOG_SOURCE_URL: &str = "https://models.dev/api.json";

/// Converts a catalog provider/model pair into a shell [`ModelEntry`].
///
/// The entry uses the qualified ID `provider/model`, the catalog model ID as
/// the routing slug, the provider's base URL, and carries no secrets — no
/// `api_key`/`env_key` (a `[provider.<id>] env_key` override is stamped on by
/// [`provider_model_entries`], not here). [`Protocol::Unsupported`] models
/// are rejected.
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
        provider_meta: Some(ProviderModelMeta {
            provider_name: provider.name.clone(),
            input_cost_per_million: model.cost.as_ref().map(|c| c.input_per_million),
            output_cost_per_million: model.cost.as_ref().map(|c| c.output_per_million),
        }),
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

/// Builds the qualified `provider/model` [`ModelEntry`] map for every catalog
/// provider whose availability is [`ProviderAvailability::Configured`] or
/// [`ProviderAvailability::Environment`].
///
/// `[provider.<id>]` config overrides are applied to the catalog provider
/// before classification and conversion, so `base_url`/`env_key`/
/// `unauthenticated` overrides shape both availability and the produced
/// entries. `credential_origin` supplies the winning credential origin per
/// (already-overridden) provider and must never expose secret values.
pub fn provider_model_entries(
    catalog: &NormalizedCatalog,
    cfg: &Config,
    credential_origin: impl Fn(&CatalogProvider) -> Option<CredentialOrigin>,
) -> IndexMap<String, ModelEntry> {
    let mut entries = IndexMap::new();
    for provider in &catalog.providers {
        let mut provider = provider.clone();
        let overrides = cfg.config_providers.get(provider.id.as_str());
        if let Some(overrides) = overrides {
            apply_config_provider_override(&mut provider, overrides);
        }
        let availability = classify_provider(&provider, credential_origin(&provider));
        if !matches!(
            availability,
            ProviderAvailability::Configured | ProviderAvailability::Environment
        ) {
            continue;
        }
        let override_env_key = overrides.and_then(|o| o.env_key.clone());
        for model in &provider.models {
            match model_entry_from_catalog(&provider, model) {
                Ok(mut entry) => {
                    // Route the `[provider.<id>] env_key` override to the
                    // credential seam: entry-level env names (names only,
                    // never values) replace the embedded-catalog fallback.
                    entry.env_key.clone_from(&override_env_key);
                    let key = format!("{}/{}", provider.id.as_str(), model.id.as_str());
                    entries.insert(key, entry);
                }
                Err(err) => {
                    tracing::debug!(
                        provider = provider.id.as_str(),
                        model = model.id.as_str(),
                        error = %err,
                        "skipping unsupported catalog model"
                    );
                }
            }
        }
    }
    entries
}

/// Shell-side handle to the catalog manager plus call-time credential
/// resolution. Holds no secrets: session keys live only for lookup and the
/// resolved secret is returned to the caller, never stored in snapshots.
pub struct ProviderCatalogAdapter {
    manager: xai_grok_catalog::CatalogManager,
    grok_home: PathBuf,
    session_keys: parking_lot::RwLock<std::collections::HashMap<String, String>>,
    /// Coalesces background catalog refreshes: only one may be in flight.
    refresh_in_flight: std::sync::atomic::AtomicBool,
}

impl ProviderCatalogAdapter {
    /// Builds an adapter over a catalog manager rooted at `grok_home`.
    pub fn new(manager: xai_grok_catalog::CatalogManager, grok_home: PathBuf) -> Self {
        Self {
            manager,
            grok_home,
            session_keys: parking_lot::RwLock::new(std::collections::HashMap::new()),
            refresh_in_flight: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Builds the production adapter: cache at
    /// `$GROK_HOME/provider_catalog.json`, sourced from
    /// [`PROVIDER_CATALOG_SOURCE_URL`] with 24-hour freshness. Construction
    /// loads the embedded catalog plus any on-disk cache without touching the
    /// network.
    pub fn from_grok_home(grok_home: PathBuf) -> Self {
        let manager = xai_grok_catalog::CatalogManager::new(
            grok_home.join(PROVIDER_CATALOG_CACHE_FILE),
            PROVIDER_CATALOG_SOURCE_URL.to_owned(),
        );
        Self::new(manager, grok_home)
    }

    /// Current immutable catalog snapshot.
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        self.manager.snapshot()
    }

    /// Attempts to claim the single background-refresh slot. Returns `true`
    /// when the caller won and must eventually call [`Self::finish_refresh`].
    pub fn try_begin_refresh(&self) -> bool {
        self.refresh_in_flight
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
    }

    /// Releases the background-refresh slot claimed by
    /// [`Self::try_begin_refresh`].
    pub fn finish_refresh(&self) {
        self.refresh_in_flight
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Whether a background catalog refresh is currently in flight.
    pub fn refresh_in_flight(&self) -> bool {
        self.refresh_in_flight
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Unconditionally performs one bounded, conditional HTTP catalog
    /// refresh. Callers coordinate coalescing via [`Self::try_begin_refresh`].
    pub async fn refresh(&self) -> Result<xai_grok_catalog::RefreshOutcome, CatalogError> {
        self.manager.refresh().await
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
        let provider = snapshot.catalog().provider(provider_id)?;
        resolve_credential(self.credential_sources(provider)).map(|resolved| resolved.secret)
    }

    /// The winning credential *origin* for `provider` (no secret exposed).
    /// Takes the provider by value so callers can classify an
    /// already-config-overridden provider (e.g. custom `env_key` names).
    pub fn credential_origin(&self, provider: &CatalogProvider) -> Option<CredentialOrigin> {
        resolve_credential(self.credential_sources(provider)).map(|resolved| resolved.origin)
    }

    /// Qualified `provider/model` entries for every provider that is
    /// configured (or keyed via environment) under `cfg`'s overrides.
    pub fn configured_model_entries(&self, cfg: &Config) -> IndexMap<String, ModelEntry> {
        let snapshot = self.manager.snapshot();
        provider_model_entries(snapshot.catalog(), cfg, |provider| {
            self.credential_origin(provider)
        })
    }

    fn credential_sources(&self, provider: &CatalogProvider) -> CredentialSources {
        let session = self
            .session_keys
            .read()
            .get(provider.id.as_str())
            .map(SecretString::new);
        let stored = crate::auth::read_provider_api_key(&self.grok_home, provider.id.as_str())
            .map(SecretString::new);
        let environment = EnvKeys::new(provider.env_vars.iter().cloned())
            .resolve_value()
            .map(SecretString::new);
        CredentialSources {
            session,
            stored,
            environment,
            model: None,
        }
    }
}
