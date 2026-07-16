//! `x.ai/providers/*` extension handlers: provider catalog listing, API key
//! management, and coalesced background catalog refresh.
//!
//! Responses are secret-free: keys are written through
//! [`crate::auth::store_provider_api_key`] and never logged or echoed.
//! Errors are concise and typed; detailed causes go only to local tracing.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use xai_grok_catalog::{ProviderAvailability, RefreshOutcome, RefreshStatus, classify_provider};

use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use crate::agent::models::ModelsManager;
use crate::agent::provider_catalog::{ProviderCatalogAdapter, apply_config_provider_override};

/// Broadcast method for provider availability changes. Forwarded machine-wide
/// by the leader, like `x.ai/models/update`.
pub const PROVIDERS_UPDATE_METHOD: &str = "x.ai/providers/update";

/// Minimal dependency surface for the provider handlers, so they can be
/// driven in integration tests without constructing a full `MvpAgent`.
#[derive(Clone)]
pub struct ProviderSurface {
    /// Owns the composed model catalog and the attached provider adapter.
    pub models_manager: ModelsManager,
    /// Root for `auth.json` provider key scopes.
    pub grok_home: PathBuf,
    /// Whether xAI credentials (OAuth session or API key) exist; feeds the
    /// synthetic `xai` row only, never touches xAI auth flows.
    pub xai_configured: bool,
}

// ── Wire types (all secret-free) ────────────────────────────────────

/// One provider row in the `x.ai/providers/list` response.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRow {
    pub provider_id: String,
    pub provider_name: String,
    pub status: ProviderAvailability,
}

/// Response for `x.ai/providers/list`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderRow>,
    /// Catalog freshness: `fresh`, `stale`, `refreshing`, `cachedAfterError`.
    pub refresh_status: String,
    /// Whether this call started a background catalog refresh.
    pub refresh_started: bool,
}

/// Params for `x.ai/providers/store_key`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreProviderKeyRequest {
    pub provider_id: String,
    pub api_key: String,
}

/// Response for `x.ai/providers/store_key` and `x.ai/providers/clear_key`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKeyResponse {
    pub provider_id: String,
    pub status: ProviderAvailability,
}

/// Params for `x.ai/providers/clear_key`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearProviderKeyRequest {
    pub provider_id: String,
}

/// Response for `x.ai/providers/refresh`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResponse {
    pub started: bool,
}

// ── Dispatch ────────────────────────────────────────────────────────

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let grok_home = crate::util::grok_home::grok_home();
    let xai_configured = agent.auth_manager.current().is_some()
        || crate::agent::auth_method::read_xai_api_key_env().is_ok()
        || crate::auth::read_api_key(&grok_home).is_some();
    let surface = ProviderSurface {
        models_manager: agent.models_manager.clone(),
        grok_home,
        xai_configured,
    };
    match args.method.as_ref() {
        "x.ai/providers/list" => to_raw_response(&list_providers(&surface)?),
        "x.ai/providers/store_key" => {
            let req: StoreProviderKeyRequest = parse_params(args)?;
            to_raw_response(&store_provider_key(&surface, req)?)
        }
        "x.ai/providers/clear_key" => {
            let req: ClearProviderKeyRequest = parse_params(args)?;
            to_raw_response(&clear_provider_key(&surface, req)?)
        }
        "x.ai/providers/refresh" => to_raw_response(&refresh_providers(&surface)),
        _ => Err(acp::Error::method_not_found()),
    }
}

// ── Handlers ────────────────────────────────────────────────────────

/// `x.ai/providers/list`: current snapshot state, returned immediately.
/// Spawns a single coalesced background refresh when the catalog is stale.
pub fn list_providers(surface: &ProviderSurface) -> Result<ProviderListResponse, acp::Error> {
    let adapter = require_adapter(surface)?;
    let refresh_started = !matches!(*adapter.snapshot().status(), RefreshStatus::Fresh)
        && adapter.try_begin_refresh();
    if refresh_started {
        spawn_catalog_refresh(surface, Arc::clone(&adapter));
    }
    Ok(ProviderListResponse {
        providers: provider_rows(surface, &adapter),
        refresh_status: refresh_status_str(&adapter),
        refresh_started,
    })
}

/// `x.ai/providers/store_key`: validate, write the key scope, rebuild model
/// availability, then broadcast `x.ai/providers/update` and
/// `x.ai/models/update` (in that order).
pub fn store_provider_key(
    surface: &ProviderSurface,
    req: StoreProviderKeyRequest,
) -> Result<ProviderKeyResponse, acp::Error> {
    let adapter = require_adapter(surface)?;
    if req.api_key.trim().is_empty() {
        return Err(acp::Error::invalid_params().data("apiKey must not be blank"));
    }
    if effective_status(surface, &adapter, &req.provider_id)? == ProviderAvailability::Unavailable {
        return Err(
            acp::Error::invalid_params().data("provider has no models supported by this client")
        );
    }
    crate::auth::store_provider_api_key(&surface.grok_home, &req.provider_id, &req.api_key)
        .map_err(|e| {
            tracing::warn!(provider = %req.provider_id, error = %e, "provider key store failed");
            acp::Error::internal_error().data("failed to store provider key")
        })?;
    broadcast_providers_update(surface, &adapter);
    surface.models_manager.rebuild_provider_models();
    Ok(ProviderKeyResponse {
        status: effective_status(surface, &adapter, &req.provider_id)?,
        provider_id: req.provider_id,
    })
}

/// `x.ai/providers/clear_key`: remove only that provider's key scope, rebuild
/// availability, and broadcast provider then model updates.
pub fn clear_provider_key(
    surface: &ProviderSurface,
    req: ClearProviderKeyRequest,
) -> Result<ProviderKeyResponse, acp::Error> {
    let adapter = require_adapter(surface)?;
    crate::auth::clear_provider_api_key(&surface.grok_home, &req.provider_id).map_err(|e| {
        tracing::warn!(provider = %req.provider_id, error = %e, "provider key clear failed");
        acp::Error::internal_error().data("failed to clear provider key")
    })?;
    broadcast_providers_update(surface, &adapter);
    surface.models_manager.rebuild_provider_models();
    Ok(ProviderKeyResponse {
        status: effective_status(surface, &adapter, &req.provider_id)?,
        provider_id: req.provider_id,
    })
}

/// `x.ai/providers/refresh`: start one coalesced background catalog refresh.
/// No model discovery is performed.
pub fn refresh_providers(surface: &ProviderSurface) -> RefreshResponse {
    let Ok(adapter) = require_adapter(surface) else {
        return RefreshResponse { started: false };
    };
    let started = adapter.try_begin_refresh();
    if started {
        spawn_catalog_refresh(surface, adapter);
    }
    RefreshResponse { started }
}

// ── Internals ───────────────────────────────────────────────────────

fn require_adapter(surface: &ProviderSurface) -> Result<Arc<ProviderCatalogAdapter>, acp::Error> {
    surface
        .models_manager
        .provider_catalog()
        .ok_or_else(|| acp::Error::internal_error().data("provider catalog not attached"))
}

/// Snapshot rows: a pinned synthetic `xai` row (driven by xAI auth state,
/// which this surface never manages), then every catalog provider with
/// config overrides applied before classification.
fn provider_rows(surface: &ProviderSurface, adapter: &ProviderCatalogAdapter) -> Vec<ProviderRow> {
    let cfg = surface.models_manager.config_snapshot();
    let snapshot = adapter.snapshot();
    let mut rows = vec![ProviderRow {
        provider_id: "xai".to_string(),
        provider_name: "xAI".to_string(),
        status: if surface.xai_configured {
            ProviderAvailability::Configured
        } else {
            ProviderAvailability::MissingKey
        },
    }];
    for provider in &snapshot.catalog().providers {
        if provider.id.as_str() == "xai" {
            continue; // xAI is managed by the dedicated auth flow above.
        }
        let mut provider = provider.clone();
        if let Some(overrides) = cfg.config_providers.get(provider.id.as_str()) {
            apply_config_provider_override(&mut provider, overrides);
        }
        let status = classify_provider(&provider, adapter.credential_origin(&provider));
        rows.push(ProviderRow {
            provider_id: provider.id.as_str().to_string(),
            provider_name: provider.name.clone(),
            status,
        });
    }
    rows
}

/// Availability for one provider after config overrides. Unknown provider →
/// concise `invalid_params`.
fn effective_status(
    surface: &ProviderSurface,
    adapter: &ProviderCatalogAdapter,
    provider_id: &str,
) -> Result<ProviderAvailability, acp::Error> {
    let cfg = surface.models_manager.config_snapshot();
    let snapshot = adapter.snapshot();
    let provider = snapshot
        .catalog()
        .provider_str(provider_id)
        .ok_or_else(|| acp::Error::invalid_params().data("unknown provider"))?;
    let mut provider = provider.clone();
    if let Some(overrides) = cfg.config_providers.get(provider.id.as_str()) {
        apply_config_provider_override(&mut provider, overrides);
    }
    Ok(classify_provider(
        &provider,
        adapter.credential_origin(&provider),
    ))
}

fn refresh_status_str(adapter: &ProviderCatalogAdapter) -> String {
    if adapter.refresh_in_flight() {
        return "refreshing".to_string();
    }
    match adapter.snapshot().status() {
        RefreshStatus::Fresh => "fresh".to_string(),
        RefreshStatus::Stale => "stale".to_string(),
        RefreshStatus::Refreshing => "refreshing".to_string(),
        RefreshStatus::CachedAfterError(_) => "cachedAfterError".to_string(),
    }
}

/// Broadcast the current secret-free provider snapshot to connected clients.
fn broadcast_providers_update(surface: &ProviderSurface, adapter: &ProviderCatalogAdapter) {
    let payload = serde_json::json!({ "providers": provider_rows(surface, adapter) });
    surface
        .models_manager
        .broadcast_ext_notification(PROVIDERS_UPDATE_METHOD, &payload);
}

/// Background catalog refresh: bounded HTTP fetch, no model discovery. On a
/// changed catalog, recomposes model availability (which broadcasts
/// `x.ai/models/update`) and pushes a replacement provider snapshot. The
/// caller must have won `try_begin_refresh`.
fn spawn_catalog_refresh(surface: &ProviderSurface, adapter: Arc<ProviderCatalogAdapter>) {
    let surface = surface.clone();
    tokio::spawn(async move {
        let outcome = adapter.refresh().await;
        adapter.finish_refresh();
        match outcome {
            Ok(RefreshOutcome::Updated) => {
                broadcast_providers_update(&surface, &adapter);
                surface.models_manager.rebuild_provider_models();
            }
            Ok(_) => {
                // Fresh/NotModified: availability unchanged; still push the
                // (now non-refreshing) snapshot so pickers can settle.
                broadcast_providers_update(&surface, &adapter);
            }
            Err(err) => {
                tracing::warn!(error = %err, "provider catalog refresh failed");
                broadcast_providers_update(&surface, &adapter);
            }
        }
    });
}
