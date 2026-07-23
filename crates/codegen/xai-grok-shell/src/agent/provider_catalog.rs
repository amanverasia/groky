//! Adapter between the dependency-neutral provider catalog
//! (`xai-grok-catalog`) and the shell's `ModelEntry` catalog.
//!
//! Conversion is secret-free: catalog snapshots never carry key material and
//! the produced [`ModelEntry`] has no `api_key`/`env_key`. Credentials are
//! resolved at call time by the credential seam in `agent::config`.

use std::collections::HashSet;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;
use xai_grok_catalog::{
    CachedModel, CachedProviderModels, CatalogError, CatalogModel, CatalogProvider,
    CatalogSnapshot, CredentialOrigin, CredentialSources, DiscoveredModel, DynamicCache,
    DynamicProviderConfig, HttpError, JANUS_PROVIDER_ID, NormalizedCatalog, Protocol,
    ProviderAvailability, ProviderId, RequestKind, SecretString, classify_provider,
    derive_endpoint, get_bounded, janus_failure, janus_failure_from_http, janus_preset,
    merge_dynamic_models, parse_model_list, redact_userinfo, resolve_credential, validate_url,
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

/// File name of the dynamic-discovery last-known-good cache under
/// `$GROK_HOME`. Secret-free; schema owned by `xai_grok_catalog::dynamic_cache`.
pub const DYNAMIC_MODELS_CACHE_FILE: &str = "dynamic_models_cache.json";

/// File name of the persisted dynamic-provider registrations under
/// `$GROK_HOME`. Deliberately secret-free — it stores only `id`,
/// `base_url`, and `allow_insecure_http` — so the Janus setup flow can be
/// re-registered on startup without touching config.toml (which has no
/// shell-side writer) or the auth store (which owns the key).
pub const DYNAMIC_PROVIDERS_FILE: &str = "dynamic_providers.json";

/// Schema version of [`DYNAMIC_PROVIDERS_FILE`].
const DYNAMIC_PROVIDERS_SCHEMA_VERSION: u32 = 1;

/// Maximum number of dynamic provider refreshes in flight at once.
pub const MAX_CONCURRENT_DYNAMIC_REFRESHES: usize = 4;

/// Outcome of a dynamic-provider orchestration step, for surfaces (pickers,
/// notifications). All variants are secret-free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderCatalogEvent {
    /// A background dynamic refresh began for `provider_id`.
    DynamicRefreshStarted {
        /// The dynamic provider being refreshed.
        provider_id: ProviderId,
    },
    /// A dynamic refresh finished and a model list was published.
    DynamicRefreshComplete {
        /// The dynamic provider that was refreshed.
        provider_id: ProviderId,
        /// Number of models now published for the provider.
        model_count: usize,
        /// `true` when discovery failed and the published models came from
        /// the last-known-good cache and/or static config instead.
        cached: bool,
    },
    /// A dynamic refresh failed and nothing could be published.
    DynamicRefreshFailed {
        /// The dynamic provider whose refresh failed.
        provider_id: ProviderId,
        /// Concise, secret-free failure description.
        message: String,
    },
    /// A Janus health probe finished (wired up by the Janus setup flow;
    /// unused until then).
    JanusHealthComplete {
        /// Whether the service answered its health endpoint.
        healthy: bool,
        /// Concise, secret-free status text.
        message: String,
    },
}

/// Errors from dynamic-provider configuration and refresh orchestration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProviderAdapterError {
    /// The provider id collides with a provider owned by a dedicated flow.
    #[error("provider id {0:?} is reserved")]
    ReservedProviderId(String),
    /// The provider id collides with the bundled or refreshed catalog.
    #[error("provider id {0:?} already belongs to the catalog")]
    CatalogProviderIdCollision(String),
    /// More than one replacement declaration used the same provider id.
    #[error("dynamic provider id {0:?} is duplicated")]
    DuplicateDynamicProviderId(String),
    /// The provider was never registered via
    /// [`ProviderCatalogAdapter::configure_dynamic`].
    #[error("unknown dynamic provider {0:?}")]
    UnknownDynamicProvider(String),
    /// A base URL violates the dynamic configuration policy.
    #[error("invalid dynamic provider configuration")]
    InvalidDynamicConfig,
    /// A derived endpoint violates the dynamic URL policy.
    #[error("invalid dynamic provider endpoint: {0}")]
    Endpoint(#[from] HttpError),
    /// Persisting the provider config or key failed (I/O detail is local
    /// only; the string carries no secrets).
    #[error("failed to persist provider setup: {0}")]
    Storage(String),
}

/// Input to [`ProviderCatalogAdapter::setup_janus`]. Deliberately does not
/// implement `Debug`/`Serialize`: the key must never be logged or echoed.
pub struct JanusSetupRequest {
    /// Janus base URL (e.g. `http://127.0.0.1:20128/v1`).
    pub base_url: String,
    /// Optional API key; `None` leaves any stored key unchanged.
    pub api_key: Option<SecretString>,
    /// Whether non-loopback plain HTTP is permitted for this provider.
    pub allow_insecure_http: bool,
}

/// Outcome of [`ProviderCatalogAdapter::setup_janus`]. Secret-free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JanusSetupResult {
    /// Janus is healthy and discovery published `model_count` models.
    Ready {
        /// Number of models published for `janus`.
        model_count: usize,
    },
    /// Janus is healthy but its model list is empty.
    Empty,
    /// Setup failed; `message` is concise, actionable, and secret-free.
    Failed {
        /// User-facing failure description.
        message: String,
        /// Models still available from the last-known-good cache.
        cached_models: usize,
    },
}

/// On-disk shape of [`DYNAMIC_PROVIDERS_FILE`]. Secret-free by design.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PersistedDynamicProviders {
    schema_version: u32,
    providers: Vec<PersistedDynamicProvider>,
}

/// One persisted dynamic-provider registration (no credentials, no models;
/// models come from the dynamic cache and keys from the auth store).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedDynamicProvider {
    id: String,
    base_url: String,
    allow_insecure_http: bool,
}

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
        // Keep the effective catalog provider's ordered env-var names on the
        // entry. This is both the configured credential fallback and the
        // value consumed later by `ModelsManager::sampling_config`; never
        // fall back to a separately loaded bundled catalog after overrides.
        env_key: Some(EnvKeys::new(provider.env_vars.iter().cloned())),
        api_base_url: None,
        provider_id: Some(provider.id.clone()),
        // `unauthenticated` means a key is *optional*, not forbidden (the
        // same contract discovery/health follow): the ProviderApiKey seam
        // resolves session > stored > environment and yields no header when
        // nothing exists, so keyless providers still work while a stored
        // key (e.g. Janus setup) authenticates inference too.
        credential_policy: CredentialPolicy::ProviderApiKey,
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
        if provider.id.as_str() == "xai" {
            // xAI models come from the dedicated entitlement pipeline; the
            // catalog's `xai` row must never enter the generic pipeline.
            continue;
        }
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
        for model in &provider.models {
            match model_entry_from_catalog(&provider, model) {
                Ok(entry) => {
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
    /// Registered dynamic providers and their published model layers.
    dynamic: parking_lot::Mutex<DynamicState>,
    /// Memoized composition of the manager snapshot plus dynamic layers,
    /// keyed by base-snapshot identity and dynamic generation.
    composed: parking_lot::Mutex<Option<ComposedSnapshot>>,
    /// Last-known-good discovery cache. All writes serialize behind this
    /// async mutex because `DynamicCache::store_provider` is
    /// read-modify-write and concurrent stores would lose updates.
    dynamic_cache: tokio::sync::Mutex<DynamicCache>,
    /// Caps concurrent dynamic refreshes at
    /// [`MAX_CONCURRENT_DYNAMIC_REFRESHES`].
    dynamic_semaphore: Arc<tokio::sync::Semaphore>,
    /// Providers with a background refresh in flight (coalescing).
    dynamic_in_flight: Arc<parking_lot::Mutex<HashSet<String>>>,
    /// Policy-enforcing discovery client (manual redirects, no auto-auth).
    discovery_client: reqwest::Client,
}

/// Registered dynamic providers plus their currently published model lists.
#[derive(Default)]
struct DynamicState {
    configs: IndexMap<String, DynamicProviderConfig>,
    models: IndexMap<String, Vec<CatalogModel>>,
    /// IDs owned by a dedicated setup flow. Generic mutation APIs neither
    /// accept nor replace these registrations (currently only Janus).
    managed_ids: HashSet<String>,
    /// Bumped once for each committed state transition; invalidates the
    /// composed-snapshot memo.
    generation: u64,
}

struct ComposedSnapshot {
    base: Arc<CatalogSnapshot>,
    generation: u64,
    composed: Arc<CatalogSnapshot>,
}

#[derive(Clone, Copy)]
enum GenericDynamicMutation {
    Upsert,
    Replace,
}

/// The provider entry a dynamic config contributes to the catalog layer.
/// Secret-free: dynamic providers carry only ordered environment-variable
/// names; credential values come from session/stored/environment scopes.
fn dynamic_catalog_provider(
    config: &DynamicProviderConfig,
    models: Vec<CatalogModel>,
) -> CatalogProvider {
    CatalogProvider {
        id: config.id.clone(),
        name: config.name.clone(),
        api_base_url: config.base_url.clone(),
        env_vars: config.env_vars.clone(),
        unauthenticated: config.unauthenticated,
        models,
    }
}

fn now_unix() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

impl ProviderCatalogAdapter {
    /// Builds an adapter over a catalog manager rooted at `grok_home`.
    /// Re-registers any dynamic providers persisted by earlier setup flows
    /// (e.g. Janus) from the secret-free [`DYNAMIC_PROVIDERS_FILE`]; no
    /// network I/O happens here.
    pub fn new(manager: xai_grok_catalog::CatalogManager, grok_home: PathBuf) -> Self {
        let dynamic_cache = DynamicCache::new(grok_home.join(DYNAMIC_MODELS_CACHE_FILE));
        let adapter = Self {
            manager,
            grok_home,
            session_keys: parking_lot::RwLock::new(std::collections::HashMap::new()),
            refresh_in_flight: std::sync::atomic::AtomicBool::new(false),
            dynamic: parking_lot::Mutex::new(DynamicState::default()),
            composed: parking_lot::Mutex::new(None),
            dynamic_cache: tokio::sync::Mutex::new(dynamic_cache),
            dynamic_semaphore: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_DYNAMIC_REFRESHES,
            )),
            dynamic_in_flight: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            discovery_client: xai_grok_catalog::http::client(),
        };
        adapter.register_persisted_dynamic_providers();
        adapter
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

    /// Current immutable catalog snapshot: the manager's snapshot with each
    /// registered dynamic provider's published model list layered on top
    /// (between bundled+cache and explicit `[provider.*]`/`[model.*]` config
    /// patches, which apply during composition). Memoized per (base,
    /// generation); with no dynamic layers this is the manager snapshot.
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        let base = self.manager.snapshot();
        let (layers, generation) = {
            let dynamic = self.dynamic.lock();
            if dynamic.models.is_empty() {
                return base;
            }
            let layers: Vec<CatalogProvider> = dynamic
                .models
                .iter()
                .filter_map(|(id, models)| {
                    // A catalog refresh may add an ID after dynamic registration.
                    // Never let a dynamic layer replace a catalog-owned provider.
                    if base.catalog().provider_str(id).is_some() {
                        return None;
                    }
                    dynamic
                        .configs
                        .get(id)
                        .map(|config| dynamic_catalog_provider(config, models.clone()))
                })
                .collect();
            (layers, dynamic.generation)
        };
        {
            let memo = self.composed.lock();
            if let Some(memo) = memo.as_ref()
                && Arc::ptr_eq(&memo.base, &base)
                && memo.generation == generation
            {
                return Arc::clone(&memo.composed);
            }
        }
        let mut composed = Arc::clone(&base);
        for provider in layers {
            composed = composed.with_dynamic_models(provider);
        }
        *self.composed.lock() = Some(ComposedSnapshot {
            base,
            generation,
            composed: Arc::clone(&composed),
        });
        composed
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

    /// Refreshes only when the cache is missing or older than the manager's
    /// refresh interval (24h); a fresh cache returns without network I/O.
    /// Callers coordinate coalescing via [`Self::try_begin_refresh`].
    pub async fn refresh_if_stale(&self) -> Result<xai_grok_catalog::RefreshOutcome, CatalogError> {
        self.manager.refresh_if_stale().await
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
        let snapshot = self.snapshot();
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
        let snapshot = self.snapshot();
        provider_model_entries(snapshot.catalog(), cfg, |provider| {
            self.credential_origin(provider)
        })
    }

    // ── Dynamic providers ───────────────────────────────────────────

    /// Insert or replace one generic dynamic provider atomically.
    pub fn upsert_dynamic(
        &self,
        config: DynamicProviderConfig,
    ) -> Result<(), ProviderAdapterError> {
        self.commit_generic_dynamic(std::iter::once(config), GenericDynamicMutation::Upsert)
    }

    /// Atomically replace every generic dynamic registration. The complete
    /// input is validated before the live state or its generation changes.
    /// Dedicated registrations (Janus) stay intact.
    pub fn replace_dynamic(
        &self,
        configs: impl IntoIterator<Item = DynamicProviderConfig>,
    ) -> Result<(), ProviderAdapterError> {
        self.commit_generic_dynamic(configs, GenericDynamicMutation::Replace)
    }

    /// Remove one generic dynamic provider. Dedicated registrations (Janus)
    /// and catalog-owned IDs cannot be removed through this public path.
    pub fn remove_dynamic(&self, provider_id: &ProviderId) -> Result<(), ProviderAdapterError> {
        let id = provider_id.as_str();
        self.validate_generic_id(id)?;
        let mut dynamic = self.dynamic.lock();
        if dynamic.managed_ids.contains(id) {
            return Err(ProviderAdapterError::ReservedProviderId(id.to_owned()));
        }
        if !dynamic.configs.contains_key(id) {
            return Err(ProviderAdapterError::UnknownDynamicProvider(id.to_owned()));
        }
        dynamic.configs.shift_remove(id);
        dynamic.models.shift_remove(id);
        dynamic.generation += 1;
        Ok(())
    }

    /// Compatibility wrapper for callers predating the explicit API. Generic
    /// calls have the same collision rules as `upsert_dynamic`; only the
    /// private Janus path may register the reserved Janus ID.
    pub fn configure_dynamic(
        &self,
        config: DynamicProviderConfig,
    ) -> Result<(), ProviderAdapterError> {
        self.upsert_dynamic(config)
    }

    fn commit_generic_dynamic(
        &self,
        configs: impl IntoIterator<Item = DynamicProviderConfig>,
        mutation: GenericDynamicMutation,
    ) -> Result<(), ProviderAdapterError> {
        let configs: Vec<_> = configs.into_iter().collect();
        let mut seen = HashSet::with_capacity(configs.len());
        let mut prepared = Vec::with_capacity(configs.len());
        for config in configs {
            let id = config.id.as_str().to_owned();
            if !seen.insert(id.clone()) {
                return Err(ProviderAdapterError::DuplicateDynamicProviderId(id));
            }
            self.validate_generic_id(&id)?;
            let statics = self.validate_and_static_models(&config)?;
            prepared.push((id, config, statics));
        }

        let mut dynamic = self.dynamic.lock();
        for (id, _, _) in &prepared {
            if dynamic.managed_ids.contains(id) {
                return Err(ProviderAdapterError::ReservedProviderId(id.clone()));
            }
        }
        if matches!(mutation, GenericDynamicMutation::Replace) {
            let managed_ids = dynamic.managed_ids.clone();
            dynamic.configs.retain(|id, _| managed_ids.contains(id));
            dynamic.models.retain(|id, _| managed_ids.contains(id));
        }
        for (id, config, statics) in prepared {
            dynamic.configs.insert(id.clone(), config);
            dynamic.models.insert(id, statics);
        }
        dynamic.generation += 1;
        Ok(())
    }

    fn validate_generic_id(&self, id: &str) -> Result<(), ProviderAdapterError> {
        if id == "xai" || id == JANUS_PROVIDER_ID {
            return Err(ProviderAdapterError::ReservedProviderId(id.to_owned()));
        }
        if self.manager.snapshot().catalog().provider_str(id).is_some() {
            return Err(ProviderAdapterError::CatalogProviderIdCollision(
                id.to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_and_static_models(
        &self,
        config: &DynamicProviderConfig,
    ) -> Result<Vec<CatalogModel>, ProviderAdapterError> {
        let canonical_base_url = config
            .canonical_base_url()
            .map_err(|_| ProviderAdapterError::InvalidDynamicConfig)?;
        let models_endpoint = derive_endpoint(
            &canonical_base_url,
            config.models_endpoint.as_deref(),
            "models",
        )?;
        validate_url(&models_endpoint, config.allow_insecure_http)?;
        let health_endpoint = derive_endpoint(
            &canonical_base_url,
            config.health_endpoint.as_deref(),
            "health",
        )?;
        validate_url(&health_endpoint, config.allow_insecure_http)?;
        let snapshot = self.manager.snapshot();
        Ok(merge_dynamic_models(
            config.protocol,
            &config.models,
            Vec::new(),
            |id| snapshot.bundled_model_by_exact_id(id),
        ))
    }

    /// The only registration path permitted to own the Janus ID. All input is
    /// validated before atomically replacing its config/models and advancing
    /// generation exactly once.
    fn configure_managed_janus(
        &self,
        config: DynamicProviderConfig,
    ) -> Result<(), ProviderAdapterError> {
        debug_assert_eq!(config.id.as_str(), JANUS_PROVIDER_ID);
        let statics = self.validate_and_static_models(&config)?;
        let mut dynamic = self.dynamic.lock();
        dynamic.managed_ids.insert(JANUS_PROVIDER_ID.to_owned());
        dynamic.configs.insert(JANUS_PROVIDER_ID.to_owned(), config);
        dynamic.models.insert(JANUS_PROVIDER_ID.to_owned(), statics);
        dynamic.generation += 1;
        Ok(())
    }

    /// Performs one bounded model discovery for a registered dynamic
    /// provider and publishes the result into the snapshot layer.
    ///
    /// The credential is resolved at request time (session > stored) and
    /// sent whenever one exists — `unauthenticated` means a key is
    /// *optional*, not forbidden, so a stored key still authenticates
    /// discovery. On success the discovered list is persisted to the
    /// last-known-good cache and merged with static config. On failure the
    /// cached and/or static models are published instead (`cached: true`);
    /// with neither, a [`ProviderCatalogEvent::DynamicRefreshFailed`]
    /// carries a concise, secret-free message (401/403 map to an auth hint;
    /// attempted URLs are userinfo-redacted; bearer text never appears).
    pub async fn refresh_dynamic(
        &self,
        provider_id: &ProviderId,
    ) -> Result<ProviderCatalogEvent, ProviderAdapterError> {
        let config = self
            .dynamic
            .lock()
            .configs
            .get(provider_id.as_str())
            .cloned()
            .ok_or_else(|| {
                ProviderAdapterError::UnknownDynamicProvider(provider_id.as_str().to_owned())
            })?;
        let endpoint = derive_endpoint(
            &config.base_url,
            config.models_endpoint.as_deref(),
            "models",
        )?;
        let credential = self.dynamic_credential(&config);

        let discovered = match get_bounded(
            &self.discovery_client,
            endpoint.as_str(),
            credential.as_ref(),
            config.allow_insecure_http,
            RequestKind::Discovery,
        )
        .await
        {
            Ok(response) => parse_model_list(&response.body).map_err(|err| {
                format!(
                    "invalid model list from {}: {err}",
                    redact_userinfo(&endpoint)
                )
            }),
            Err(HttpError::Status(code @ (401 | 403))) => Err(format!(
                "authentication failed (HTTP {code}); update the provider API key"
            )),
            Err(err) => Err(format!(
                "model discovery failed for {}: {err}",
                redact_userinfo(&endpoint)
            )),
        };

        match discovered {
            Ok(models) => {
                let entry = CachedProviderModels::new(
                    &config,
                    now_unix(),
                    models
                        .iter()
                        .map(|model| CachedModel {
                            id: model.id.clone(),
                            name: model.name.clone(),
                        })
                        .collect(),
                )
                .map_err(|_| ProviderAdapterError::InvalidDynamicConfig)?;
                // All cache writes hold this async mutex for the whole
                // read-modify-write, so concurrent refreshes cannot lose
                // each other's provider entries. Re-check the live registration
                // after acquiring it so a replacement/removal cannot race between
                // validation and persistence.
                let stored = {
                    let cache = self.dynamic_cache.lock().await;
                    let still_current =
                        self.dynamic.lock().configs.get(config.id.as_str()) == Some(&config);
                    if !still_current {
                        return Ok(ProviderCatalogEvent::DynamicRefreshComplete {
                            provider_id: provider_id.clone(),
                            model_count: 0,
                            cached: false,
                        });
                    }
                    cache.store_provider(entry).await
                };
                if let Err(err) = stored {
                    tracing::warn!(
                        provider = provider_id.as_str(),
                        error = %err,
                        "failed to persist dynamic model cache"
                    );
                }
                let model_count = self.publish_dynamic_models(&config, models);
                Ok(ProviderCatalogEvent::DynamicRefreshComplete {
                    provider_id: provider_id.clone(),
                    model_count,
                    cached: false,
                })
            }
            Err(message) => {
                let cached = {
                    let cache = self.dynamic_cache.lock().await;
                    cache
                        .load()
                        .await
                        .ok()
                        .and_then(|file| file.applicable_provider(&config).cloned())
                };
                if cached.is_none() && config.models.is_empty() {
                    return Ok(ProviderCatalogEvent::DynamicRefreshFailed {
                        provider_id: provider_id.clone(),
                        message,
                    });
                }
                tracing::warn!(
                    provider = provider_id.as_str(),
                    %message,
                    "dynamic discovery failed; serving cached/static models"
                );
                let discovered = cached
                    .map(|entry| {
                        entry
                            .models
                            .into_iter()
                            .map(|model| DiscoveredModel {
                                id: model.id,
                                name: model.name,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let model_count = self.publish_dynamic_models(&config, discovered);
                Ok(ProviderCatalogEvent::DynamicRefreshComplete {
                    provider_id: provider_id.clone(),
                    model_count,
                    cached: true,
                })
            }
        }
    }

    /// Merges discovered/cached models with static config (bundled-catalog
    /// enrichment on exact full-ID match) and publishes the layer only when
    /// the live registration still matches the configuration that was refreshed.
    fn publish_dynamic_models(
        &self,
        config: &DynamicProviderConfig,
        discovered: Vec<DiscoveredModel>,
    ) -> usize {
        let snapshot = self.manager.snapshot();
        let merged = merge_dynamic_models(config.protocol, &config.models, discovered, |id| {
            snapshot.bundled_model_by_exact_id(id)
        });
        let count = merged.len();
        let mut dynamic = self.dynamic.lock();
        if dynamic
            .configs
            .get(config.id.as_str())
            .is_none_or(|current| current != config)
        {
            return 0;
        }
        dynamic.models.insert(config.id.as_str().to_owned(), merged);
        dynamic.generation += 1;
        count
    }

    /// Spawns background refreshes for every registered discovery-enabled
    /// dynamic provider whose cache entry is missing or older than
    /// `DYNAMIC_CACHE_MAX_AGE`. At most
    /// [`MAX_CONCURRENT_DYNAMIC_REFRESHES`] run concurrently; per-provider
    /// refreshes coalesce via an in-flight set. Never performs network I/O
    /// before returning; `on_event` receives every start/completion/failure
    /// (secret-free) and is where callers recompose the model catalog.
    pub fn refresh_stale_dynamic_in_background(
        self: &Arc<Self>,
        on_event: impl Fn(ProviderCatalogEvent) + Send + Sync + 'static,
    ) {
        self.spawn_dynamic_refreshes(false, on_event);
    }

    /// Like [`Self::refresh_stale_dynamic_in_background`], but skips the
    /// cache-staleness gate: every registered discovery-enabled provider is
    /// re-discovered (still semaphore-capped and per-provider coalesced).
    /// Used by explicit user refreshes (`x.ai/providers/refresh` with
    /// `force: true`).
    pub fn refresh_all_dynamic_in_background(
        self: &Arc<Self>,
        on_event: impl Fn(ProviderCatalogEvent) + Send + Sync + 'static,
    ) {
        self.spawn_dynamic_refreshes(true, on_event);
    }

    fn spawn_dynamic_refreshes(
        self: &Arc<Self>,
        force: bool,
        on_event: impl Fn(ProviderCatalogEvent) + Send + Sync + 'static,
    ) {
        let adapter = Arc::clone(self);
        let on_event: Arc<dyn Fn(ProviderCatalogEvent) + Send + Sync> = Arc::new(on_event);
        tokio::spawn(async move {
            let candidates: Vec<DynamicProviderConfig> = adapter
                .dynamic
                .lock()
                .configs
                .values()
                .filter(|config| config.discover)
                .cloned()
                .collect();
            if candidates.is_empty() {
                return;
            }
            let cache_file = if force {
                None
            } else {
                let cache = adapter.dynamic_cache.lock().await;
                cache.load().await.ok()
            };
            let max_age = i64::try_from(xai_grok_catalog::limits::DYNAMIC_CACHE_MAX_AGE.as_secs())
                .unwrap_or(i64::MAX);
            let now = now_unix();
            for config in candidates {
                let provider_id = config.id.clone();
                let stale = force
                    || cache_file
                        .as_ref()
                        .and_then(|file| file.applicable_provider(&config))
                        .is_none_or(|entry| now.saturating_sub(entry.fetched_at_unix) >= max_age);
                if !stale {
                    continue;
                }
                if !adapter
                    .dynamic_in_flight
                    .lock()
                    .insert(provider_id.as_str().to_owned())
                {
                    continue;
                }
                let adapter = Arc::clone(&adapter);
                let on_event = Arc::clone(&on_event);
                tokio::spawn(async move {
                    /// Removes the provider from the in-flight set on drop,
                    /// so a panic inside `refresh_dynamic` (or the event
                    /// callback) cannot block future background refreshes
                    /// for this provider forever.
                    struct InFlightGuard {
                        in_flight: Arc<parking_lot::Mutex<HashSet<String>>>,
                        provider_id: String,
                    }
                    impl Drop for InFlightGuard {
                        fn drop(&mut self) {
                            self.in_flight.lock().remove(&self.provider_id);
                        }
                    }
                    let _in_flight_guard = InFlightGuard {
                        in_flight: Arc::clone(&adapter.dynamic_in_flight),
                        provider_id: provider_id.as_str().to_owned(),
                    };
                    let _permit = Arc::clone(&adapter.dynamic_semaphore)
                        .acquire_owned()
                        .await
                        .expect("dynamic refresh semaphore is never closed");
                    on_event(ProviderCatalogEvent::DynamicRefreshStarted {
                        provider_id: provider_id.clone(),
                    });
                    let result = adapter.refresh_dynamic(&provider_id).await;
                    match result {
                        Ok(event) => on_event(event),
                        Err(err) => on_event(ProviderCatalogEvent::DynamicRefreshFailed {
                            provider_id,
                            message: err.to_string(),
                        }),
                    }
                });
            }
        });
    }

    // ── Janus setup ─────────────────────────────────────────────────

    /// Runs the full Janus setup flow: validate + register the preset with
    /// the requested base URL, persist the secret-free config, store the
    /// optional key, health-check the service, then discover and publish
    /// its models.
    ///
    /// Failures that the user can act on (bad URL, unreachable service,
    /// rejected key, empty/invalid model list) come back as
    /// [`JanusSetupResult::Failed`]/[`JanusSetupResult::Empty`] with
    /// secret-free messages; `Err` is reserved for local persistence
    /// failures. The request is never logged.
    pub async fn setup_janus(
        &self,
        request: JanusSetupRequest,
    ) -> Result<JanusSetupResult, ProviderAdapterError> {
        let provider_id =
            ProviderId::new(JANUS_PROVIDER_ID).expect("the Janus provider id constant is valid");
        let has_credential_in_request = request.api_key.is_some();

        // 1–2. Preset with overrides, validated + registered (URL policy is
        // enforced by configure_dynamic before anything is published).
        let mut config = janus_preset();
        config.base_url = request.base_url.clone();
        config.allow_insecure_http = request.allow_insecure_http;
        if let Err(err) = self.configure_managed_janus(config.clone()) {
            // The raw base URL may embed userinfo credentials (that is one
            // of the ways validation fails); redact before it can reach a
            // user-facing message, and drop it entirely if unparseable.
            let attempted = url::Url::parse(&request.base_url)
                .map(|url| redact_userinfo(&url))
                .unwrap_or_else(|_| "the configured base URL".to_string());
            let message = match &err {
                ProviderAdapterError::Endpoint(http) => {
                    janus_failure(&janus_failure_from_http(http, &attempted))
                }
                other => other.to_string(),
            };
            let cached_models = self.cached_dynamic_model_count(&config).await;
            tracing::warn!(
                provider = provider_id.as_str(),
                cached_models,
                "janus setup rejected: invalid endpoint configuration"
            );
            return Ok(JanusSetupResult::Failed {
                message,
                cached_models,
            });
        }

        // 3. Persist the secret-free registration for re-registration on
        // startup (config.toml has no shell-side writer; see
        // [`DYNAMIC_PROVIDERS_FILE`]).
        self.persist_dynamic_provider(&config)
            .map_err(|err| ProviderAdapterError::Storage(err.to_string()))?;

        // 4. Store the key if one was supplied; `None` leaves any existing
        // stored key unchanged (clear_key is the explicit removal path).
        if let Some(key) = request.api_key.as_ref() {
            crate::auth::store_provider_api_key(
                &self.grok_home,
                provider_id.as_str(),
                key.expose(),
            )
            .map_err(|err| ProviderAdapterError::Storage(err.to_string()))?;
        }

        // 5. Health probe with the resolved credential (stored or request).
        let health_endpoint = derive_endpoint(
            &config.base_url,
            config.health_endpoint.as_deref(),
            "health",
        )?;
        let credential = self.dynamic_credential(&config);
        let has_credential = credential.is_some() || has_credential_in_request;
        if let Err(err) = get_bounded(
            &self.discovery_client,
            health_endpoint.as_str(),
            credential.as_ref(),
            config.allow_insecure_http,
            RequestKind::Health,
        )
        .await
        {
            let redacted = redact_userinfo(&health_endpoint);
            let message = janus_failure(&janus_failure_from_http(&err, &redacted));
            let cached_models = self.cached_dynamic_model_count(&config).await;
            tracing::warn!(
                provider = provider_id.as_str(),
                url = %redacted,
                status = "unhealthy",
                cached_models,
                has_credential,
                "janus health probe failed"
            );
            return Ok(JanusSetupResult::Failed {
                message,
                cached_models,
            });
        }

        // 6. Healthy: same discovery/merge/cache path as a normal refresh.
        let event = self.refresh_dynamic(&provider_id).await?;
        let result = match event {
            ProviderCatalogEvent::DynamicRefreshComplete {
                model_count,
                cached: false,
                ..
            } => {
                if model_count == 0 {
                    JanusSetupResult::Empty
                } else {
                    JanusSetupResult::Ready { model_count }
                }
            }
            ProviderCatalogEvent::DynamicRefreshComplete {
                model_count,
                cached: true,
                ..
            } => JanusSetupResult::Failed {
                message: "Janus is healthy but model discovery failed; showing previously \
                          discovered models."
                    .to_string(),
                cached_models: model_count,
            },
            ProviderCatalogEvent::DynamicRefreshFailed { message, .. } => {
                JanusSetupResult::Failed {
                    message,
                    cached_models: 0,
                }
            }
            other => {
                tracing::warn!(?other, "unexpected event from janus discovery");
                JanusSetupResult::Failed {
                    message: janus_failure(&xai_grok_catalog::JanusFailure::InvalidResponse),
                    cached_models: 0,
                }
            }
        };
        let (status, model_count) = match &result {
            JanusSetupResult::Ready { model_count } => ("ready", *model_count),
            JanusSetupResult::Empty => ("empty", 0),
            JanusSetupResult::Failed { cached_models, .. } => ("failed", *cached_models),
        };
        tracing::info!(
            provider = provider_id.as_str(),
            url = %redact_userinfo(&health_endpoint),
            status,
            model_count,
            has_credential,
            "janus setup finished"
        );
        Ok(result)
    }

    /// Resolves the outbound credential for a dynamic provider (session >
    /// stored). A credential is sent whenever one exists, even for
    /// `unauthenticated` providers, where a key is optional.
    fn dynamic_credential(&self, config: &DynamicProviderConfig) -> Option<SecretString> {
        resolve_credential(self.credential_sources(&dynamic_catalog_provider(config, Vec::new())))
            .map(|resolved| resolved.secret)
    }

    /// Number of models in the applicable last-known-good cache entry.
    async fn cached_dynamic_model_count(&self, config: &DynamicProviderConfig) -> usize {
        let cache = self.dynamic_cache.lock().await;
        cache
            .load()
            .await
            .ok()
            .and_then(|file| {
                file.applicable_provider(config)
                    .map(|entry| entry.models.len())
            })
            .unwrap_or(0)
    }

    /// Upserts one provider's secret-free registration into
    /// [`DYNAMIC_PROVIDERS_FILE`] atomically (temp file + rename, mirroring
    /// the dynamic cache's write pattern).
    fn persist_dynamic_provider(&self, config: &DynamicProviderConfig) -> std::io::Result<()> {
        let path = self.grok_home.join(DYNAMIC_PROVIDERS_FILE);
        let mut file = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PersistedDynamicProviders>(&bytes).ok())
            .unwrap_or_default();
        file.schema_version = DYNAMIC_PROVIDERS_SCHEMA_VERSION;
        let entry = PersistedDynamicProvider {
            id: config.id.as_str().to_owned(),
            base_url: config.base_url.clone(),
            allow_insecure_http: config.allow_insecure_http,
        };
        match file
            .providers
            .iter_mut()
            .find(|p| p.id == config.id.as_str())
        {
            Some(existing) => *existing = entry,
            None => file.providers.push(entry),
        }
        std::fs::create_dir_all(&self.grok_home)?;
        let tmp = path.with_file_name(format!(
            "{DYNAMIC_PROVIDERS_FILE}.tmp-{}",
            std::process::id()
        ));
        std::fs::write(&tmp, serde_json::to_vec_pretty(&file)?)?;
        std::fs::rename(&tmp, &path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Re-registers dynamic providers persisted by earlier setup flows.
    /// Today only the Janus preset is recognized; unknown ids are skipped.
    /// Failures are logged, never fatal — a corrupt file must not block
    /// startup.
    fn register_persisted_dynamic_providers(&self) {
        let path = self.grok_home.join(DYNAMIC_PROVIDERS_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "failed to read persisted dynamic providers"
                );
                return;
            }
        };
        let file = match serde_json::from_slice::<PersistedDynamicProviders>(&bytes) {
            Ok(file) => file,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "failed to parse persisted dynamic providers"
                );
                return;
            }
        };
        if file.schema_version != DYNAMIC_PROVIDERS_SCHEMA_VERSION {
            tracing::warn!(
                path = %path.display(),
                schema_version = file.schema_version,
                expected = DYNAMIC_PROVIDERS_SCHEMA_VERSION,
                "ignoring persisted dynamic providers with unknown schema version"
            );
            return;
        }
        for persisted in file.providers {
            if persisted.id != JANUS_PROVIDER_ID {
                tracing::debug!(
                    provider = %persisted.id,
                    "skipping persisted dynamic provider with no known preset"
                );
                continue;
            }
            let mut config = janus_preset();
            config.base_url = persisted.base_url;
            config.allow_insecure_http = persisted.allow_insecure_http;
            if let Err(err) = self.configure_managed_janus(config) {
                tracing::warn!(
                    provider = %persisted.id,
                    error = %err,
                    "failed to re-register persisted dynamic provider"
                );
            }
        }
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
