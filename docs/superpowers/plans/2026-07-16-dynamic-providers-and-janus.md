# Dynamic Providers And Janus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add secure, bounded OpenAI-compatible model discovery and a first-class Janus setup flow that preserve exact model IDs, survive outages with last-known-good models, and drive end-to-end inference through the existing sampler.

**Architecture:** Extend Plan 2's `xai-grok-catalog` crate with provider configuration, URL policy, discovery parsing/merge, a manual-redirect HTTP client, dynamic cache persistence, and the Janus preset. Extend Plan 2's shell `ProviderCatalogAdapter` to orchestrate health/discovery and publish immutable snapshots; the pager owns only setup presentation and user input, while inference continues through existing `ModelEntry` conversion and `xai-grok-sampler`.

**Tech Stack:** Rust 2024, Tokio, reqwest 0.12 streaming, serde/serde_json/TOML, url 2, arc-swap, wiremock 0.6, axum-based `MockInferenceServer`, ratatui pager, Cargo nextest/test.

---

## File Responsibility Map

Plan 2 is a hard prerequisite. It must already provide `crates/codegen/xai-grok-catalog`, the provider/model catalog and auth-store types, immutable `CatalogSnapshot` publication, configured-provider filtering, `/providers`, `/login` aliasing, and the shell adapter at `crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs`. Keep those responsibilities intact; this plan extends them rather than rebuilding the catalog or provider picker.

| Path | Action | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify | Register `futures-util` and `toml` as workspace dependencies for catalog discovery/config tests, retain existing `wiremock`, and retain the Plan 2 `xai-grok-catalog` workspace dependency. |
| `crates/codegen/xai-grok-sampler/src/config.rs` | Modify | Make `SamplerConfig` debug output structurally redact credentials before provider keys can enter the sampler. |
| `crates/codegen/xai-grok-sampler/src/client.rs` | Modify | Remove authorization/key prefix tracing and retain boolean-only auth diagnostics. |
| `crates/codegen/xai-grok-sampler/tests/test_secret_redaction.rs` | Create | Regression tests proving sampler config/debug logs cannot contain raw or partial keys. |
| `crates/codegen/xai-grok-catalog/Cargo.toml` | Modify | Add `reqwest`, `futures-util`, `tokio`, `url`, and `wiremock` test support needed for discovery. |
| `crates/codegen/xai-grok-catalog/src/lib.rs` | Modify | Export the dynamic-provider, discovery, cache, and Janus APIs. |
| `crates/codegen/xai-grok-catalog/src/limits.rs` | Create | Single source of truth for body, model-count, field-length, redirect, timeout, and cache-age bounds. |
| `crates/codegen/xai-grok-catalog/src/dynamic.rs` | Create | Typed dynamic-provider configuration, endpoint derivation, static/discovered merge, metadata enrichment, and exact-ID preservation. |
| `crates/codegen/xai-grok-catalog/src/discovery.rs` | Create | Bounded OpenAI model-list parser and typed discovery/health errors. |
| `crates/codegen/xai-grok-catalog/src/http.rs` | Create | URL security policy, body-limited GET, timeout, and manual redirects with cross-origin credential stripping. |
| `crates/codegen/xai-grok-catalog/src/dynamic_cache.rs` | Create | Secret-free, per-provider last-known-good dynamic model cache with atomic persistence. |
| `crates/codegen/xai-grok-catalog/src/janus.rs` | Create | Janus preset constants/defaults, health interpretation, key hint, and actionable status text. |
| `crates/codegen/xai-grok-catalog/tests/discovery_http.rs` | Create | Wiremock integration coverage for auth, redirects, URL policy, body bounds, and stale fallback inputs. |
| `crates/codegen/xai-grok-catalog/tests/fixtures/openai-models.json` | Create | OpenAI-format discovery fixture containing slash and combo IDs. |
| `crates/codegen/xai-grok-shell/Cargo.toml` | Modify | Consume the Plan 2 catalog crate and add test support for dynamic provider orchestration. |
| `crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs` | Modify | Extend the Plan 2 adapter with dynamic refresh, cache fallback, Janus setup commands, status events, and immutable snapshot replacement. |
| `crates/codegen/xai-grok-shell/src/extensions/providers.rs` | Modify | Add typed `x.ai/providers/setup_janus` ACP request/response DTOs and route them to the adapter. |
| `crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs` | Modify | Route the Janus setup provider extension without exposing adapter internals to pager. |
| `crates/codegen/xai-grok-shell/src/agent/config.rs` | Modify | Resolve catalog-generated provider credentials into `ModelEntry`/`SamplerConfig` without serializing secrets into snapshots. |
| `crates/codegen/xai-grok-shell/src/agent/models.rs` | Modify | Trigger bounded dynamic refresh and adopt replacement catalog snapshots without changing the selected model. |
| `crates/codegen/xai-grok-shell/src/session/slash_commands.rs` | Modify | Advertise the Plan 2 `/providers` surface and route `/login` to it without altering unrelated login behavior. |
| `crates/codegen/xai-grok-shell/tests/test_dynamic_provider_e2e.rs` | Create | End-to-end Janus setup, discovery, exact model selection, auth header, and chat-completions inference. |
| `crates/codegen/xai-grok-test-support/src/mock_server.rs` | Modify | Add `/v1/health`, configurable health status/body, auth checks for GET endpoints, and full GET header capture. |
| `crates/codegen/xai-grok-test-support/src/lib.rs` | Modify | Re-export `HealthResponse` for shell and pager integration fixtures. |
| `crates/codegen/xai-grok-pager/src/slash/commands/providers.rs` | Modify | Extend Plan 2's provider command with Janus setup actions and `/login` alias behavior. |
| `crates/codegen/xai-grok-pager/src/providers.rs` | Modify | Add secret-free Janus setup ACP response DTOs beside Plan 2's provider DTOs. |
| `crates/codegen/xai-grok-pager/src/views/providers_modal.rs` | Modify | Extend Plan 2's provider modal with Janus base URL, optional-key, health, discovery, empty-list, cached/offline, and insecure-HTTP confirmation states. |
| `crates/codegen/xai-grok-pager/src/app/actions.rs` | Modify | Carry typed Janus setup inputs/results through the pager event loop. |
| `crates/codegen/xai-grok-pager/src/app/dispatch/auth.rs` | Modify | Advance the Janus setup state machine while retaining the existing xAI login dispatch. |
| `crates/codegen/xai-grok-pager/src/app/dispatch/router.rs` | Modify | Route the new provider actions/results. |
| `crates/codegen/xai-grok-pager/src/app/effects/mod.rs` | Modify | Execute the typed Janus ACP setup request off the render loop and redact the key-bearing effect. |
| `crates/codegen/xai-grok-pager/tests/pty_e2e_provider_setup.rs` | Create | PTY coverage of Janus defaults, optional key, actionable failures, and cached-offline UX. |
| `docs/configuration/providers.md` | Create | Document generic discovery and Janus configuration, security policy, limits, and exact-ID semantics. |

## Prerequisite API Contract From Plan 2

Use these exact Plan 2 names throughout this plan. If Plan 2 has not landed with this contract, finish Plan 2 before starting Task 1 rather than introducing duplicate types:

```rust
// xai-grok-catalog
pub struct CatalogSnapshot;
pub struct ProviderId(String);
pub struct ModelId(String);
pub struct NormalizedCatalog;
pub struct CatalogProvider;
pub struct CatalogModel;
pub struct ModelPatch;
pub enum Protocol { ChatCompletions, Responses, Messages, Unsupported }
pub struct SecretString;

impl CatalogSnapshot {
    pub fn catalog(&self) -> &NormalizedCatalog;
    pub fn model(&self, provider_id: &ProviderId, model_id: &ModelId) -> Option<&CatalogModel>;
    pub fn bundled_model_by_exact_id(&self, id: &ModelId) -> Option<&CatalogModel>;
}

// xai-grok-shell/src/agent/provider_catalog.rs
pub(crate) struct ProviderCatalogAdapter;
pub(crate) enum ProviderCatalogEvent;

impl ProviderCatalogAdapter {
    pub(crate) fn snapshot(&self) -> std::sync::Arc<CatalogSnapshot>;
    pub(crate) fn publish(&self, snapshot: std::sync::Arc<CatalogSnapshot>);
    pub(crate) fn credential_for(&self, provider_id: &ProviderId) -> Option<SecretString>;
}
```

`ProviderId` and `ModelId` are the validated Plan 2 newtypes. Catalog code uses `Protocol`; conversion to the repository's existing `xai_grok_sampling_types::ApiBackend` occurs only in `agent/provider_catalog.rs`. Dynamic static metadata uses Plan 2's field-aware `ModelPatch`; this plan does not introduce `Catalog`, `ProviderDefinition`, `ModelMetadata`, `ModelOverride`, `ProviderAuth`, or a second credential-resolver trait.

The implementation must not put `SecretString`, bearer text, `api_key`, or authorization headers in `CatalogSnapshot`, `CatalogModel`, dynamic cache JSON, status events, or debug formatting.

### Task 1: Remove Sampler Credential Debug And Trace Leakage

**Files:**
- Modify: `crates/codegen/xai-grok-sampler/src/config.rs:48-127`
- Modify: `crates/codegen/xai-grok-sampler/src/client.rs:401-430,549-597`
- Create: `crates/codegen/xai-grok-sampler/tests/test_secret_redaction.rs`

- [ ] **Step 1: Write failing structural-redaction tests**

Create `crates/codegen/xai-grok-sampler/tests/test_secret_redaction.rs`:

```rust
use xai_grok_sampler::{ApiBackend, SamplerConfig, SamplingClient};

const SECRET: &str = "sk-janus-super-secret-0123456789";

fn config() -> SamplerConfig {
    SamplerConfig {
        api_key: Some(SECRET.to_owned()),
        base_url: "http://127.0.0.1:20128/v1".to_owned(),
        model: "openai/gpt-4o".to_owned(),
        api_backend: ApiBackend::ChatCompletions,
        ..SamplerConfig::default()
    }
}

#[test]
fn sampler_config_debug_never_contains_key_or_prefix() {
    let rendered = format!("{:?}", config());
    assert!(!rendered.contains(SECRET));
    assert!(!rendered.contains("sk-janus"));
    assert!(rendered.contains("has_api_key: true"));
}

#[test]
fn sampling_client_debug_never_contains_key_or_prefix() {
    let rendered = format!("{:?}", SamplingClient::new(config()).unwrap());
    assert!(!rendered.contains(SECRET));
    assert!(!rendered.contains("sk-janus"));
}
```

- [ ] **Step 2: Run the tests and verify the raw config debug test fails**

Run: `cargo test -p xai-grok-sampler --test test_secret_redaction -- --nocapture`

Expected: `sampler_config_debug_never_contains_key_or_prefix` fails because derived `Debug` prints `api_key: Some("sk-janus-super-secret-0123456789")`.

- [ ] **Step 3: Replace derived `Debug` with a manual boolean-only implementation**

In `config.rs`, change `SamplerConfig` to `#[derive(Clone, Serialize, Deserialize)]` and add:

```rust
impl std::fmt::Debug for SamplerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamplerConfig")
            .field("has_api_key", &self.api_key.is_some())
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("max_completion_tokens", &self.max_completion_tokens)
            .field("temperature", &self.temperature)
            .field("top_p", &self.top_p)
            .field("api_backend", &self.api_backend)
            .field("auth_scheme", &self.auth_scheme)
            .field("extra_header_names", &self.extra_headers.keys().collect::<Vec<_>>())
            .field("context_window", &self.context_window)
            .field("force_http1", &self.force_http1)
            .field("max_retries", &self.max_retries)
            .field("stream_tool_calls", &self.stream_tool_calls)
            .field("idle_timeout_secs", &self.idle_timeout_secs)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("has_attribution_callback", &self.attribution_callback.is_some())
            .field("has_bearer_resolver", &self.bearer_resolver.is_some())
            .field("supports_backend_search", &self.supports_backend_search)
            .field("compactions_remaining", &self.compactions_remaining)
            .field("compaction_at_tokens", &self.compaction_at_tokens)
            .field("doom_loop_recovery", &self.doom_loop_recovery)
            .field("has_header_injector", &self.header_injector.is_some())
            .finish_non_exhaustive()
    }
}
```

Do not print extra-header values because those may contain provider credentials.

- [ ] **Step 4: Remove raw key and prefix fields from sampler tracing**

In `SamplingClient::new`, replace both invalid-header tracing calls with:

```rust
tracing::debug!(
    auth_scheme = ?config.auth_scheme,
    "Invalid configured credential: cannot be converted to an HTTP header"
);
```

In `SamplingClient::post`, delete the `auth_prefix` and `x_api_key_prefix` extraction block and keep only:

```rust
tracing::info!(
    target: crate::sampling_log::TARGET,
    event = "client_post",
    base_url = %self.base_url,
    model = %self.defaults.model,
    api_backend = ?self.defaults.api_backend,
    auth_scheme = ?self.defaults.auth_scheme,
    has_bearer_resolver = self.bearer_resolver.is_some(),
    has_authorization_header = headers.get(AUTHORIZATION).is_some(),
    has_x_api_key_header = headers.get(HeaderName::from_static("x-api-key")).is_some(),
);
```

- [ ] **Step 5: Run focused and crate tests**

Run: `cargo test -p xai-grok-sampler --test test_secret_redaction && cargo test -p xai-grok-sampler`

Expected: both commands exit 0; neither test output nor panic output contains `sk-janus-super-secret`.

- [ ] **Step 6: Commit sampler hardening**

```bash
git add crates/codegen/xai-grok-sampler/src/config.rs crates/codegen/xai-grok-sampler/src/client.rs crates/codegen/xai-grok-sampler/tests/test_secret_redaction.rs
git commit -m "fix(sampler): redact configured credentials"
```

### Task 2: Lock Dynamic Provider Configuration And Numeric Bounds

**Files:**
- Modify: `crates/codegen/xai-grok-catalog/Cargo.toml`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`
- Create: `crates/codegen/xai-grok-catalog/src/limits.rs`
- Create: `crates/codegen/xai-grok-catalog/src/dynamic.rs`

- [ ] **Step 1: Write failing tests for all central bounds and Janus-compatible configuration**

At the bottom of the new `dynamic.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::*;

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
        assert_eq!(DYNAMIC_CACHE_MAX_AGE, std::time::Duration::from_secs(24 * 60 * 60));
    }

    #[test]
    fn config_defaults_to_chat_completions_and_models_discovery() {
        let cfg: DynamicProviderConfig = toml::from_str(r#"
            id = "local"
            name = "Local Gateway"
            base_url = "http://127.0.0.1:9000/v1"
            unauthenticated = true
            discover = true
        "#).unwrap();
        assert_eq!(cfg.protocol, Protocol::ChatCompletions);
        assert_eq!(cfg.models_endpoint, None);
        assert_eq!(cfg.health_endpoint, None);
        assert!(!cfg.allow_insecure_http);
    }

    #[test]
    fn provider_and_static_model_ids_keep_slashes_and_hyphens() {
        let cfg: DynamicProviderConfig = toml::from_str(r#"
            id = "gateway"
            name = "Gateway"
            base_url = "https://gateway.example/v1"
            discover = true
            [models."openai/gpt-4o"]
            name = "GPT-4o"
            [models.best-effort]
            context_window = 32768
        "#).unwrap();
        assert!(cfg.models.contains_key(&ModelId::new("openai/gpt-4o").unwrap()));
        assert!(cfg.models.contains_key(&ModelId::new("best-effort").unwrap()));
    }

    #[test]
    fn invalid_lengths_are_rejected_at_config_boundary() {
        let err = DynamicProviderConfig::new(
            "x".repeat(MAX_PROVIDER_ID_BYTES + 1),
            "Gateway",
            "https://gateway.example/v1",
        ).unwrap_err();
        assert_eq!(err, DynamicConfigError::ProviderIdTooLong);
    }

    #[test]
    fn toml_cannot_bypass_id_or_endpoint_bounds() {
        let long_id = "x".repeat(MAX_PROVIDER_ID_BYTES + 1);
        let source = format!("id = \"{long_id}\"\nname = \"Gateway\"\nbase_url = \"https://gateway.example/v1\"");
        let error = toml::from_str::<DynamicProviderConfig>(&source).unwrap_err().to_string();
        assert!(error.contains("provider id exceeds 64 bytes"), "{error}");

        let long_endpoint = format!("/{}", "m".repeat(MAX_ENDPOINT_BYTES));
        let source = format!("id = \"gateway\"\nname = \"Gateway\"\nbase_url = \"https://gateway.example/v1\"\nmodels_endpoint = \"{long_endpoint}\"");
        let error = toml::from_str::<DynamicProviderConfig>(&source).unwrap_err().to_string();
        assert!(error.contains("endpoint exceeds 2048 bytes"), "{error}");
    }
}
```

- [ ] **Step 2: Run the focused test and verify missing modules fail compilation**

Run: `cargo test -p xai-grok-catalog dynamic::tests -- --nocapture`

Expected: compilation fails because `limits` and `DynamicProviderConfig` do not exist.

- [ ] **Step 3: Add central constants**

Create `limits.rs` with exactly:

```rust
use std::time::Duration;

pub const MAX_DISCOVERY_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_DISCOVERED_MODELS: usize = 2_000;
pub const MAX_PROVIDER_ID_BYTES: usize = 64;
pub const MAX_PROVIDER_NAME_BYTES: usize = 128;
pub const MAX_MODEL_ID_BYTES: usize = 512;
pub const MAX_MODEL_NAME_BYTES: usize = 512;
pub const MAX_ENDPOINT_BYTES: usize = 2_048;
pub const MAX_REDIRECTS: usize = 5;
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
pub const DYNAMIC_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
```

The 24-hour value is the product freshness policy shared with Plan 2's bundled catalog refresh. It controls stale refresh scheduling only; a failed refresh retains last-known-good dynamic models regardless of age.

- [ ] **Step 4: Add typed dynamic configuration**

Implement these exact public types in `dynamic.rs`:

```rust
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use crate::{ModelId, ModelPatch, Protocol, ProviderId};
use crate::limits::{MAX_ENDPOINT_BYTES, MAX_PROVIDER_ID_BYTES, MAX_PROVIDER_NAME_BYTES};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "RawDynamicProviderConfig")]
pub struct DynamicProviderConfig {
    pub id: ProviderId,
    pub name: String,
    pub base_url: String,
    #[serde(default, rename = "api_backend")]
    pub protocol: Protocol,
    #[serde(default)]
    pub unauthenticated: bool,
    #[serde(default)]
    pub discover: bool,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub health_endpoint: Option<String>,
    #[serde(default)]
    pub allow_insecure_http: bool,
    #[serde(default)]
    pub models: IndexMap<ModelId, ModelPatch>,
}

#[derive(Deserialize)]
struct RawModelPatch {
    name: Option<String>,
    #[serde(rename = "api_backend")]
    protocol: Option<Protocol>,
    context_window: Option<u64>,
    reasoning: Option<bool>,
    cost: Option<crate::ModelCost>,
    #[serde(default)]
    exclude: bool,
}

#[derive(Deserialize)]
struct RawDynamicProviderConfig {
    id: String,
    name: String,
    base_url: String,
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

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DynamicConfigError {
    #[error("provider id exceeds 64 bytes")]
    ProviderIdTooLong,
    #[error("provider name exceeds 128 bytes")]
    ProviderNameTooLong,
    #[error("endpoint exceeds 2048 bytes")]
    EndpointTooLong,
    #[error("provider id is invalid: {0}")]
    InvalidProviderId(String),
    #[error("model id is invalid: {0}")]
    InvalidModelId(String),
}

impl DynamicProviderConfig {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, DynamicConfigError> {
        let id = id.into();
        let name = name.into();
        let base_url = base_url.into();
        if id.len() > MAX_PROVIDER_ID_BYTES { return Err(DynamicConfigError::ProviderIdTooLong); }
        if name.len() > MAX_PROVIDER_NAME_BYTES { return Err(DynamicConfigError::ProviderNameTooLong); }
        if base_url.len() > MAX_ENDPOINT_BYTES { return Err(DynamicConfigError::EndpointTooLong); }
        let id = ProviderId::new(id)
            .map_err(|error| DynamicConfigError::InvalidProviderId(error.to_string()))?;
        Ok(Self {
            id,
            name,
            base_url,
            protocol: Protocol::ChatCompletions,
            unauthenticated: false,
            discover: false,
            models_endpoint: None,
            health_endpoint: None,
            allow_insecure_http: false,
            models: IndexMap::new(),
        })
    }
}

impl TryFrom<RawDynamicProviderConfig> for DynamicProviderConfig {
    type Error = DynamicConfigError;

    fn try_from(raw: RawDynamicProviderConfig) -> Result<Self, Self::Error> {
        let mut config = Self::new(raw.id, raw.name, raw.base_url)?;
        for endpoint in [&raw.models_endpoint, &raw.health_endpoint].into_iter().flatten() {
            if endpoint.len() > MAX_ENDPOINT_BYTES {
                return Err(DynamicConfigError::EndpointTooLong);
            }
        }
        config.protocol = raw.protocol;
        config.unauthenticated = raw.unauthenticated;
        config.discover = raw.discover;
        config.models_endpoint = raw.models_endpoint;
        config.health_endpoint = raw.health_endpoint;
        config.allow_insecure_http = raw.allow_insecure_http;
        config.models = raw.models.into_iter().map(|(id, patch)| {
            let id = ModelId::new(id)
                .map_err(|error| DynamicConfigError::InvalidModelId(error.to_string()))?;
            Ok((id.clone(), ModelPatch {
                id,
                name: patch.name,
                protocol: patch.protocol,
                context_window: patch.context_window,
                reasoning: patch.reasoning,
                cost: patch.cost,
                exclude: patch.exclude,
            }))
        }).collect::<Result<_, DynamicConfigError>>()?;
        Ok(config)
    }
}
```

Use Plan 2's `Protocol::default()` (`ChatCompletions`); ensure Plan 2's `ProviderId`/`ModelId` serde implementations validate on deserialize rather than accepting overlong IDs. Deserialize the raw `[models."<id>"]` table through `IndexMap<String, RawModelPatch>`, validate each table key into a `ModelId`, and construct the public `IndexMap<ModelId, ModelPatch>` by injecting that ID into its patch. The `toml_cannot_bypass_id_or_endpoint_bounds` test proves config files cannot bypass the constructor bounds. `unauthenticated = true` means a bearer key is optional; `false` means a resolved credential is required.

- [ ] **Step 5: Export modules and add dependencies**

In `lib.rs` add:

```rust
pub mod dynamic;
pub mod limits;

pub use dynamic::{DynamicConfigError, DynamicProviderConfig};
```

In the catalog manifest add workspace `indexmap`, `serde`, `thiserror`, `toml`, and `url` dependencies. Duplicate existing entries are not added; the resulting manifest contains one declaration for each dependency.

- [ ] **Step 6: Run tests**

Run: `cargo test -p xai-grok-catalog dynamic::tests`

Expected: 5 dynamic configuration tests pass, including exact slash/combo ID preservation.

- [ ] **Step 7: Commit configuration and bounds**

```bash
git add Cargo.toml crates/codegen/xai-grok-catalog/Cargo.toml crates/codegen/xai-grok-catalog/src/lib.rs crates/codegen/xai-grok-catalog/src/limits.rs crates/codegen/xai-grok-catalog/src/dynamic.rs
git commit -m "feat(catalog): define bounded dynamic providers"
```

### Task 3: Parse, Merge, And Enrich Discovered Models

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/discovery.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/dynamic.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`
- Create: `crates/codegen/xai-grok-catalog/tests/fixtures/openai-models.json`

- [ ] **Step 1: Add the exact fixture**

Create `tests/fixtures/openai-models.json`:

```json
{
  "object": "list",
  "data": [
    {"id": "openai/gpt-4o", "object": "model", "created": 1720000000, "owned_by": "openai"},
    {"id": "best-effort", "object": "model", "created": 1720000001, "owned_by": "janus"},
    {"id": "anthropic/claude-3-5-sonnet", "object": "model", "created": 1720000002, "owned_by": "anthropic", "future": true}
  ]
}
```

- [ ] **Step 2: Write failing parser, count, length, merge, and enrichment tests**

Add tests to `discovery.rs` and `dynamic.rs`:

```rust
#[test]
fn parses_openai_list_without_rewriting_ids() {
    let body = include_bytes!("../tests/fixtures/openai-models.json");
    let models = parse_model_list(body).unwrap();
    let ids = models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids, ["openai/gpt-4o", "best-effort", "anthropic/claude-3-5-sonnet"]);
}

#[test]
fn rejects_model_count_above_bound() {
    let data = (0..=crate::limits::MAX_DISCOVERED_MODELS)
        .map(|n| serde_json::json!({"id": format!("m-{n}")}))
        .collect::<Vec<_>>();
    let body = serde_json::to_vec(&serde_json::json!({"object":"list","data":data})).unwrap();
    assert_eq!(parse_model_list(&body).unwrap_err(), DiscoveryError::TooManyModels);
}

#[test]
fn rejects_overlong_id_and_name() {
    let id = "m".repeat(crate::limits::MAX_MODEL_ID_BYTES + 1);
    let body = serde_json::to_vec(&serde_json::json!({"data":[{"id":id}]})).unwrap();
    assert_eq!(parse_model_list(&body).unwrap_err(), DiscoveryError::ModelIdTooLong { index: 0 });
}

#[test]
fn explicit_static_model_wins_and_exact_catalog_match_enriches_missing_fields() {
    let discovered = vec![DiscoveredModel::new("openai/gpt-4o").unwrap(), DiscoveredModel::new("best-effort").unwrap()];
    let static_models = IndexMap::from([
        (ModelId::new("openai/gpt-4o").unwrap(), ModelPatch {
            id: ModelId::new("openai/gpt-4o").unwrap(),
            name: Some("Pinned GPT".into()),
            context_window: None,
            reasoning: None,
            cost: None,
            protocol: None,
            exclude: false,
        })
    ]);
    let bundled = CatalogModel {
        id: ModelId::new("openai/gpt-4o").unwrap(),
        name: "GPT-4o".into(),
        protocol: Protocol::ChatCompletions,
        context_window: Some(128_000),
        reasoning: true,
        cost: Some(ModelCost { input_per_million: 2.5, output_per_million: 10.0 }),
    };
    let merged = merge_dynamic_models(
        Protocol::ChatCompletions,
        &static_models,
        discovered,
        |id| (id.as_str() == "openai/gpt-4o").then_some(&bundled),
    );
    assert_eq!(merged[0].id.as_str(), "openai/gpt-4o");
    assert_eq!(merged[0].name, "Pinned GPT");
    assert_eq!(merged[0].context_window, Some(128_000));
    assert!(merged[0].reasoning);
    assert_eq!(merged[1].id.as_str(), "best-effort");
    assert_eq!(merged[1].context_window, None);
    assert_eq!(merged[1].protocol, Protocol::ChatCompletions);
}
```

- [ ] **Step 3: Run and verify failures**

Run: `cargo test -p xai-grok-catalog discovery -- --nocapture`

Expected: compilation fails because `parse_model_list`, `DiscoveredModel`, and `merge_dynamic_models` do not exist.

- [ ] **Step 4: Implement the bounded OpenAI response parser**

Create `discovery.rs` with these exact public APIs:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredModel {
    pub id: crate::ModelId,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    #[error("model-list response exceeds 2097152 bytes")]
    BodyTooLarge,
    #[error("model-list response is invalid JSON: {0}")]
    InvalidJson(String),
    #[error("model-list response must contain a data array")]
    MissingData,
    #[error("model-list response exceeds 2000 models")]
    TooManyModels,
    #[error("model {index} has no string id")]
    MissingModelId { index: usize },
    #[error("model {index} id exceeds 512 bytes")]
    ModelIdTooLong { index: usize },
    #[error("model {index} name exceeds 512 bytes")]
    ModelNameTooLong { index: usize },
    #[error("model {index} id is invalid: {reason}")]
    InvalidModelId { index: usize, reason: String },
}

impl DiscoveredModel {
    pub fn new(id: impl Into<String>) -> Result<Self, DiscoveryError> {
        let id = id.into();
        if id.len() > crate::limits::MAX_MODEL_ID_BYTES {
            return Err(DiscoveryError::ModelIdTooLong { index: 0 });
        }
        let id = crate::ModelId::new(id)
            .map_err(|error| DiscoveryError::InvalidModelId { index: 0, reason: error.to_string() })?;
        Ok(Self { id, name: None })
    }
}

pub fn parse_model_list(body: &[u8]) -> Result<Vec<DiscoveredModel>, DiscoveryError> {
    if body.len() > crate::limits::MAX_DISCOVERY_BODY_BYTES {
        return Err(DiscoveryError::BodyTooLarge);
    }
    let root: serde_json::Value = serde_json::from_slice(body)
        .map_err(|error| DiscoveryError::InvalidJson(error.to_string()))?;
    let data = root.get("data").and_then(serde_json::Value::as_array)
        .ok_or(DiscoveryError::MissingData)?;
    if data.len() > crate::limits::MAX_DISCOVERED_MODELS {
        return Err(DiscoveryError::TooManyModels);
    }
    data.iter().enumerate().map(|(index, value)| {
        let id = value.get("id").and_then(serde_json::Value::as_str)
            .ok_or(DiscoveryError::MissingModelId { index })?;
        if id.len() > crate::limits::MAX_MODEL_ID_BYTES {
            return Err(DiscoveryError::ModelIdTooLong { index });
        }
        let name = value.get("name").and_then(serde_json::Value::as_str);
        if name.is_some_and(|name| name.len() > crate::limits::MAX_MODEL_NAME_BYTES) {
            return Err(DiscoveryError::ModelNameTooLong { index });
        }
        let id = crate::ModelId::new(id.to_owned())
            .map_err(|error| DiscoveryError::InvalidModelId { index, reason: error.to_string() })?;
        Ok(DiscoveredModel { id, name: name.map(str::to_owned) })
    }).collect()
}
```

Unknown response and model fields remain ignored. Duplicate IDs are collapsed in first-seen order before merge so a server cannot use duplicates to bypass model-count semantics.

- [ ] **Step 5: Implement static precedence and exact-ID metadata enrichment**

In `dynamic.rs`, add:

```rust
pub fn merge_dynamic_models<'a>(
    default_protocol: Protocol,
    static_models: &IndexMap<ModelId, ModelPatch>,
    discovered: Vec<crate::DiscoveredModel>,
    mut bundled_exact: impl FnMut(&ModelId) -> Option<&'a CatalogModel>,
) -> Vec<CatalogModel> {
    let mut merged = IndexMap::<ModelId, CatalogModel>::new();
    for model in discovered {
        let bundled = bundled_exact(&model.id);
        let id = model.id;
        let name = model.name.unwrap_or_else(|| id.as_str().to_owned());
        merged.entry(id.clone()).or_insert_with(|| CatalogModel {
            id,
            name,
            protocol: default_protocol.clone(),
            context_window: bundled.and_then(|m| m.context_window),
            reasoning: bundled.is_some_and(|m| m.reasoning),
            cost: bundled.and_then(|m| m.cost.clone()),
        });
    }
    for (id, explicit) in static_models {
        let bundled = bundled_exact(id);
        let mut model = merged.shift_remove(id).unwrap_or_else(|| CatalogModel {
            id: id.clone(),
            name: id.as_str().to_owned(),
            protocol: default_protocol.clone(),
            context_window: bundled.and_then(|m| m.context_window),
            reasoning: bundled.is_some_and(|m| m.reasoning),
            cost: bundled.and_then(|m| m.cost.clone()),
        });
        if explicit.exclude { merged.shift_remove(id); continue; }
        if let Some(name) = &explicit.name { model.name.clone_from(name); }
        if let Some(protocol) = &explicit.protocol { model.protocol = protocol.clone(); }
        if let Some(context_window) = explicit.context_window { model.context_window = Some(context_window); }
        if let Some(reasoning) = explicit.reasoning { model.reasoning = reasoning; }
        if let Some(cost) = &explicit.cost { model.cost = Some(cost.clone()); }
        merged.insert(id.clone(), model);
    }
    merged.into_values().collect()
}
```

`ModelPatch` replaces only `Some` fields, exactly as in Plan 2's field-aware layer. Enrichment is by the complete `ModelId` string, never by basename: `openai/gpt-4o` may enrich only from a bundled model whose exact ID is `openai/gpt-4o`; it must not match `gpt-4o`.

- [ ] **Step 6: Export and test**

Export `DiscoveredModel`, `DiscoveryError`, `parse_model_list`, and `merge_dynamic_models` from `lib.rs`.

Run: `cargo test -p xai-grok-catalog discovery && cargo test -p xai-grok-catalog dynamic`

Expected: parser, bounds, merge precedence, exact-ID enrichment, slash IDs, and combo IDs all pass.

- [ ] **Step 7: Commit parser and merge**

```bash
git add crates/codegen/xai-grok-catalog/src/discovery.rs crates/codegen/xai-grok-catalog/src/dynamic.rs crates/codegen/xai-grok-catalog/src/lib.rs crates/codegen/xai-grok-catalog/tests/fixtures/openai-models.json
git commit -m "feat(catalog): parse and merge discovered models"
```

### Task 4: Enforce URL Policy And Credential-Safe Manual Redirects

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/http.rs`
- Create: `crates/codegen/xai-grok-catalog/tests/discovery_http.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`
- Modify: `crates/codegen/xai-grok-catalog/Cargo.toml`

- [ ] **Step 1: Write failing endpoint derivation and plain-HTTP policy tests**

In `http.rs` add tests:

```rust
#[test]
fn derives_models_relative_to_normalized_base() {
    assert_eq!(derive_endpoint("https://gateway.example/v1", None, "models").unwrap().as_str(), "https://gateway.example/v1/models");
    assert_eq!(derive_endpoint("https://gateway.example/v1/", None, "models").unwrap().as_str(), "https://gateway.example/v1/models");
    assert_eq!(derive_endpoint("https://gateway.example/api", Some("/custom/models"), "models").unwrap().as_str(), "https://gateway.example/custom/models");
}

#[test]
fn plain_http_is_loopback_only_without_opt_in() {
    assert!(validate_url(&url::Url::parse("http://127.0.0.1:20128/v1/models").unwrap(), false).is_ok());
    assert!(validate_url(&url::Url::parse("http://[::1]:20128/v1/models").unwrap(), false).is_ok());
    assert_eq!(validate_url(&url::Url::parse("http://192.168.1.20:20128/v1/models").unwrap(), false).unwrap_err(), HttpError::InsecureHttpDenied);
    assert!(validate_url(&url::Url::parse("http://192.168.1.20:20128/v1/models").unwrap(), true).is_ok());
    assert_eq!(validate_url(&url::Url::parse("file:///tmp/models").unwrap(), true).unwrap_err(), HttpError::UnsupportedScheme);
}
```

- [ ] **Step 2: Write failing wiremock redirect/security tests**

Create `tests/discovery_http.rs` using two `wiremock::MockServer` instances. Cover these exact cases:

```rust
#[tokio::test]
async fn same_origin_redirect_keeps_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/v1/models-final"))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/models-final"))
        .and(header("authorization", "Bearer sk-janus-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})))
        .mount(&server).await;
    let response = get_bounded(&client(), &format!("{}/v1/models", server.uri()), Some(&SecretString::new("sk-janus-test")), false, RequestKind::Discovery).await.unwrap();
    assert_eq!(response.body, br#"{"data":[]}"#);
}

#[tokio::test]
async fn cross_origin_redirect_strips_bearer() {
    let origin = MockServer::start().await;
    let target = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models"))
        .and(header("authorization", "Bearer sk-janus-test"))
        .respond_with(ResponseTemplate::new(307).insert_header("Location", format!("{}/models", target.uri())))
        .mount(&origin).await;
    Mock::given(method("GET")).and(path("/models"))
        .and(header_does_not_exist("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})))
        .mount(&target).await;
    get_bounded(&client(), &format!("{}/v1/models", origin.uri()), Some(&SecretString::new("sk-janus-test")), false, RequestKind::Discovery).await.unwrap();
}
```

Also add `redirect_loop_stops_at_five`, `redirect_to_disallowed_plain_http_is_rejected`, `content_length_above_limit_is_rejected_before_read`, and `chunked_body_above_limit_is_rejected_while_streaming`. The oversized cases assert `HttpError::BodyTooLarge`, not a serde error.

- [ ] **Step 3: Run and verify missing transport APIs fail compilation**

Run: `cargo test -p xai-grok-catalog --test discovery_http -- --nocapture`

Expected: compilation fails because `get_bounded`, `RequestKind`, and `HttpError` do not exist.

- [ ] **Step 4: Implement endpoint derivation and URL validation**

Create these exact APIs in `http.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind { Discovery, Health }

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("unsupported URL scheme; use https or loopback http")]
    UnsupportedScheme,
    #[error("plain HTTP is allowed only for loopback unless allow_insecure_http=true")]
    InsecureHttpDenied,
    #[error("endpoint has no host")]
    MissingHost,
    #[error("endpoint is invalid: {0}")]
    InvalidUrl(String),
    #[error("redirect limit exceeded")]
    TooManyRedirects,
    #[error("response exceeds the configured body limit")]
    BodyTooLarge,
    #[error("request timed out")]
    Timeout,
    #[error("request failed: {0}")]
    Transport(String),
    #[error("server returned HTTP {0}")]
    Status(u16),
}

pub struct BoundedResponse {
    pub final_url: url::Url,
    pub status: reqwest::StatusCode,
    pub body: Vec<u8>,
}

pub fn derive_endpoint(base: &str, override_path: Option<&str>, default_leaf: &str) -> Result<url::Url, HttpError>;
pub fn validate_url(url: &url::Url, allow_insecure_http: bool) -> Result<(), HttpError>;
pub fn client() -> reqwest::Client;
pub async fn get_bounded(
    client: &reqwest::Client,
    endpoint: &str,
    credential: Option<&crate::SecretString>,
    allow_insecure_http: bool,
    kind: RequestKind,
) -> Result<BoundedResponse, HttpError>;
```

Use `reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()`. `validate_url` accepts only `https`, or `http` whose parsed `IpAddr::is_loopback()` is true unless opt-in is set. Hostnames such as `localhost` are resolved by the OS but are accepted only when the literal host is exactly `localhost`; arbitrary DNS names are never treated as loopback.

- [ ] **Step 5: Implement manual redirects and streaming body bounds**

In `get_bounded`, use this loop shape:

```rust
let mut url = url::Url::parse(endpoint).map_err(|e| HttpError::InvalidUrl(e.to_string()))?;
let mut send_credential = true;
for redirect_count in 0..=crate::limits::MAX_REDIRECTS {
    validate_url(&url, allow_insecure_http)?;
    let mut request = client.get(url.clone());
    if send_credential {
        if let Some(secret) = credential {
            request = request.bearer_auth(secret.expose());
        }
    }
    let timeout = match kind {
        RequestKind::Discovery => crate::limits::DISCOVERY_TIMEOUT,
        RequestKind::Health => crate::limits::HEALTH_TIMEOUT,
    };
    let response = tokio::time::timeout(timeout, request.send()).await
        .map_err(|_| HttpError::Timeout)?
        .map_err(|e| HttpError::Transport(e.to_string()))?;
    if response.status().is_redirection() {
        if redirect_count == crate::limits::MAX_REDIRECTS { return Err(HttpError::TooManyRedirects); }
        let location = response.headers().get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| HttpError::Transport("redirect has no valid Location header".into()))?;
        let next = url.join(location).map_err(|e| HttpError::InvalidUrl(e.to_string()))?;
        validate_url(&next, allow_insecure_http)?;
        send_credential &= same_origin(&url, &next);
        url = next;
        continue;
    }
    if let Some(length) = response.content_length()
        && length > crate::limits::MAX_DISCOVERY_BODY_BYTES as u64 {
        return Err(HttpError::BodyTooLarge);
    }
    if !response.status().is_success() { return Err(HttpError::Status(response.status().as_u16())); }
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|e| HttpError::Transport(e.to_string()))?;
        if body.len().saturating_add(chunk.len()) > crate::limits::MAX_DISCOVERY_BODY_BYTES {
            return Err(HttpError::BodyTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    return Ok(BoundedResponse { final_url: url, status, body });
}
Err(HttpError::TooManyRedirects)
```

`same_origin` compares scheme, lowercase host, and `port_or_known_default`. Never include endpoint user-info in a request; reject URLs where `username()` is non-empty or `password()` is present.

- [ ] **Step 6: Run URL and transport tests**

Run: `cargo test -p xai-grok-catalog http::tests && cargo test -p xai-grok-catalog --test discovery_http`

Expected: all policy, redirect, auth stripping, and size-bound tests pass. Wiremock reports no unmatched authorization expectations.

- [ ] **Step 7: Commit secure transport**

```bash
git add crates/codegen/xai-grok-catalog/Cargo.toml crates/codegen/xai-grok-catalog/src/http.rs crates/codegen/xai-grok-catalog/src/lib.rs crates/codegen/xai-grok-catalog/tests/discovery_http.rs
git commit -m "feat(catalog): secure dynamic model discovery"
```

### Task 5: Add Secret-Free Last-Known-Good Dynamic Cache

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/dynamic_cache.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`
- Modify: `crates/codegen/xai-grok-catalog/tests/discovery_http.rs`

- [ ] **Step 1: Write failing cache round-trip and stale fallback tests**

Add tests in `dynamic_cache.rs`:

```rust
#[tokio::test]
async fn cache_round_trip_contains_no_secret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dynamic_models.json");
    let cache = DynamicCache::new(path.clone());
    let entry = CachedProviderModels {
        provider_id: ProviderId::new("janus").unwrap(),
        base_url: "http://127.0.0.1:20128/v1".into(),
        fetched_at_unix: 1_721_088_000,
        models: vec![CachedModel { id: ModelId::new("openai/gpt-4o").unwrap(), name: None }],
    };
    cache.store_provider(entry.clone()).await.unwrap();
    let bytes = tokio::fs::read(&path).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("api_key"));
    assert!(!text.contains("authorization"));
    assert_eq!(cache.load().await.unwrap().provider(&ProviderId::new("janus").unwrap()), Some(&entry));
}

#[tokio::test]
async fn failed_refresh_keeps_stale_last_known_good() {
    let cached = CachedProviderModels {
        provider_id: ProviderId::new("janus").unwrap(),
        base_url: "http://127.0.0.1:20128/v1".into(),
        fetched_at_unix: 1,
        models: vec![CachedModel { id: ModelId::new("best-effort").unwrap(), name: None }],
    };
    let outcome = DynamicRefreshOutcome::from_result(
        Err(DiscoveryRefreshError::Offline), Some(cached.clone()), 2_000_000,
    );
    assert_eq!(outcome.models(), cached.models.as_slice());
    assert_eq!(outcome.status(), DynamicRefreshStatus::CachedOffline);
}

#[tokio::test]
async fn valid_empty_response_replaces_nonempty_cache() {
    let outcome = DynamicRefreshOutcome::from_result(
        Ok(Vec::new()), Some(cached_provider("janus", &["best-effort"])), 2_000_000,
    );
    assert!(outcome.models().is_empty());
    assert_eq!(outcome.status(), DynamicRefreshStatus::Empty);
}
```

- [ ] **Step 2: Run and verify missing cache APIs fail compilation**

Run: `cargo test -p xai-grok-catalog dynamic_cache::tests -- --nocapture`

Expected: compilation fails because `DynamicCache` and refresh outcome types do not exist.

- [ ] **Step 3: Implement cache schema and atomic persistence**

Create `dynamic_cache.rs` with:

```rust
pub const DYNAMIC_CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedModel {
    pub id: crate::ModelId,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedProviderModels {
    pub provider_id: crate::ProviderId,
    pub base_url: String,
    pub fetched_at_unix: i64,
    pub models: Vec<CachedModel>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DynamicCacheFile {
    pub schema_version: u32,
    pub providers: indexmap::IndexMap<crate::ProviderId, CachedProviderModels>,
}

#[derive(Clone, Debug)]
pub struct DynamicCache { path: std::path::PathBuf }

impl DynamicCache {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self;
    pub async fn load(&self) -> Result<DynamicCacheFile, DynamicCacheError>;
    pub async fn store_provider(&self, provider: CachedProviderModels) -> Result<(), DynamicCacheError>;
}
```

`load` returns an empty version-1 cache for `NotFound`, rejects other schema versions, validates every ID and all Plan 2 field bounds, and never deletes the file on an error. `store_provider` loads the last valid file, updates one provider, writes JSON to a sibling `dynamic_models.json.tmp-<pid>-<nonce>`, calls `sync_all`, and atomically renames it to `~/.grok/dynamic_models.json`; on Unix create it with mode `0600` even though it contains no credentials.

- [ ] **Step 4: Implement refresh outcome semantics**

Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicRefreshStatus { Fresh, Empty, CachedOffline }

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryRefreshError {
    #[error("provider is offline")]
    Offline,
    #[error("authentication failed")]
    Authentication,
    #[error("discovery response is invalid")]
    InvalidResponse,
}

pub struct DynamicRefreshOutcome {
    models: Vec<CachedModel>,
    status: DynamicRefreshStatus,
    refresh_error: Option<DiscoveryRefreshError>,
}

impl DynamicRefreshOutcome {
    pub fn from_result(
        result: Result<Vec<CachedModel>, DiscoveryRefreshError>,
        cached: Option<CachedProviderModels>,
        _now_unix: i64,
    ) -> Self;
    pub fn models(&self) -> &[CachedModel];
    pub fn status(&self) -> DynamicRefreshStatus;
    pub fn refresh_error(&self) -> Option<&DiscoveryRefreshError>;
}
```

Any successful response, including an empty `data` list, becomes the new last-known-good value. Any transport, timeout, HTTP, or parse failure returns the existing cached models regardless of cache age; `DYNAMIC_CACHE_MAX_AGE` controls whether background refresh starts, not whether offline models disappear.

- [ ] **Step 5: Run cache tests**

Run: `cargo test -p xai-grok-catalog dynamic_cache::tests`

Expected: all cache tests pass; the cache remains readable after a simulated failed refresh and contains no secret-bearing field names.

- [ ] **Step 6: Commit dynamic cache**

```bash
git add crates/codegen/xai-grok-catalog/src/dynamic_cache.rs crates/codegen/xai-grok-catalog/src/lib.rs crates/codegen/xai-grok-catalog/tests/discovery_http.rs
git commit -m "feat(catalog): cache last known dynamic models"
```

### Task 6: Add Janus Preset, Health Semantics, And Actionable Errors

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/janus.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/http.rs`

- [ ] **Step 1: Write failing Janus defaults and error-message tests**

Add to `janus.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_matches_janus_contract_exactly() {
        let preset = janus_preset();
        assert_eq!(preset.id.as_str(), "janus");
        assert_eq!(preset.name, "Janus");
        assert_eq!(preset.base_url, "http://127.0.0.1:20128/v1");
        assert_eq!(preset.protocol, Protocol::ChatCompletions);
        assert!(preset.unauthenticated);
        assert_eq!(preset.health_endpoint.as_deref(), Some("/v1/health"));
        assert_eq!(preset.models_endpoint.as_deref(), Some("/v1/models"));
        assert_eq!(JANUS_KEY_HINT, "optional, starts with sk-janus-");
    }

    #[test]
    fn health_and_discovery_failures_are_actionable_and_secret_free() {
        assert_eq!(janus_failure(&JanusFailure::Connection { url: "http://127.0.0.1:20128/v1/health".into() }), "Could not connect to Janus at http://127.0.0.1:20128/v1/health. Check that the Janus service is running and the base URL is correct.");
        assert_eq!(janus_failure(&JanusFailure::Authentication), "Janus rejected the API key. Update the Janus key in /providers.");
        assert_eq!(janus_failure(&JanusFailure::EmptyModels), "Janus is healthy but returned no models.");
        assert!(!janus_failure(&JanusFailure::Authentication).contains("sk-"));
    }
}
```

- [ ] **Step 2: Run and verify missing preset APIs fail compilation**

Run: `cargo test -p xai-grok-catalog janus::tests -- --nocapture`

Expected: compilation fails because `janus_preset` and `JanusFailure` do not exist.

- [ ] **Step 3: Implement Janus preset and status mapping**

Create `janus.rs`:

```rust
use crate::{DynamicProviderConfig, Protocol};

pub const JANUS_PROVIDER_ID: &str = "janus";
pub const JANUS_DISPLAY_NAME: &str = "Janus";
pub const JANUS_DEFAULT_BASE_URL: &str = "http://127.0.0.1:20128/v1";
pub const JANUS_HEALTH_ENDPOINT: &str = "/v1/health";
pub const JANUS_MODELS_ENDPOINT: &str = "/v1/models";
pub const JANUS_KEY_HINT: &str = "optional, starts with sk-janus-";

pub fn janus_preset() -> DynamicProviderConfig {
    let mut config = DynamicProviderConfig::new(
        JANUS_PROVIDER_ID,
        JANUS_DISPLAY_NAME,
        JANUS_DEFAULT_BASE_URL,
    ).expect("Janus constants are valid");
    config.protocol = Protocol::ChatCompletions;
    config.unauthenticated = true;
    config.discover = true;
    config.health_endpoint = Some(JANUS_HEALTH_ENDPOINT.to_owned());
    config.models_endpoint = Some(JANUS_MODELS_ENDPOINT.to_owned());
    config
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JanusFailure {
    Connection { url: String },
    Authentication,
    InvalidResponse,
    EmptyModels,
}

pub fn janus_failure(failure: &JanusFailure) -> String {
    match failure {
        JanusFailure::Connection { url } => format!("Could not connect to Janus at {url}. Check that the Janus service is running and the base URL is correct."),
        JanusFailure::Authentication => "Janus rejected the API key. Update the Janus key in /providers.".to_owned(),
        JanusFailure::InvalidResponse => "Janus returned an invalid model list. Check that the base URL points to an OpenAI-compatible Janus API.".to_owned(),
        JanusFailure::EmptyModels => "Janus is healthy but returned no models.".to_owned(),
    }
}
```

Map HTTP 401 and 403 from either health or discovery to `Authentication`; map connection and timeout to `Connection` with the attempted URL; map successful `data: []` to `EmptyModels` while retaining it as a valid last-known-good result.

- [ ] **Step 4: Run Janus tests**

Run: `cargo test -p xai-grok-catalog janus::tests`

Expected: defaults, optional auth, endpoint paths, and exact user-facing messages pass.

- [ ] **Step 5: Commit Janus catalog support**

```bash
git add crates/codegen/xai-grok-catalog/src/janus.rs crates/codegen/xai-grok-catalog/src/lib.rs crates/codegen/xai-grok-catalog/src/http.rs
git commit -m "feat(catalog): add Janus dynamic provider preset"
```

### Task 7: Extend MockInferenceServer For Health And GET Header Capture

**Files:**
- Modify: `crates/codegen/xai-grok-test-support/src/mock_server.rs:33-85,293-410,729-1029`
- Modify: `crates/codegen/xai-grok-test-support/src/lib.rs`

- [ ] **Step 1: Write failing mock-server tests**

Add tests inside `mock_server.rs`:

```rust
#[tokio::test]
async fn health_and_models_capture_authorization_headers() {
    let server = MockInferenceServer::start_with_required_auth(
        vec![MockModelEntry::new("openai/gpt-4o")],
        "sk-janus-test",
    ).await.unwrap();
    let client = reqwest::Client::new();
    assert_eq!(client.get(format!("{}/health", server.url())).bearer_auth("sk-janus-test").send().await.unwrap().status(), StatusCode::OK);
    assert_eq!(client.get(format!("{}/models", server.url())).bearer_auth("sk-janus-test").send().await.unwrap().status(), StatusCode::OK);
    let requests = server.requests();
    assert_eq!(requests[0].path, "/v1/health");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-janus-test"));
    assert_eq!(requests[1].path, "/v1/models");
    assert_eq!(requests[1].header("authorization"), Some("Bearer sk-janus-test"));
}

#[tokio::test]
async fn health_status_is_configurable() {
    let server = MockInferenceServer::start().await.unwrap();
    server.set_health(StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({"status":"starting"}));
    let response = reqwest::get(format!("{}/health", server.url())).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
```

- [ ] **Step 2: Run and verify the tests fail**

Run: `cargo test -p xai-grok-test-support health_ -- --nocapture`

Expected: compilation fails because `set_health` does not exist; before the implementation, GET request entries do not expose headers.

- [ ] **Step 3: Add mutable health state and full GET logging**

Add:

```rust
#[derive(Clone)]
pub struct HealthResponse {
    status: StatusCode,
    body: Value,
}

impl Default for HealthResponse {
    fn default() -> Self {
        Self { status: StatusCode::OK, body: json!({"status":"ok"}) }
    }
}
```

Store `health: Arc<RwLock<HealthResponse>>` on `MockInferenceServer`, initialize and pass it to `build_router`, and add:

```rust
pub fn set_health(&self, status: StatusCode, body: Value) {
    *self.health.write().unwrap() = HealthResponse { status, body };
}
```

Add `GET /v1/health`. For both `/v1/health` and `/v1/models`, accept `HeaderMap`, record `Self::extract_auth(&headers)` plus `Self::headers_vec(&headers)`, and call `check_auth` using a clone of `required_token`. Keep inference POST behavior unchanged.

- [ ] **Step 4: Run mock support tests**

Run: `cargo test -p xai-grok-test-support`

Expected: all tests pass; GET entries contain lowercase header names and the auth gate returns 401 when the configured token is absent.

- [ ] **Step 5: Commit mock-server support**

```bash
git add crates/codegen/xai-grok-test-support/src/mock_server.rs crates/codegen/xai-grok-test-support/src/lib.rs
git commit -m "test: extend inference mock for provider setup"
```

### Task 8: Orchestrate Dynamic Refresh Through The Plan 2 Shell Adapter

**Files:**
- Modify: `crates/codegen/xai-grok-shell/Cargo.toml`
- Modify: `crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/models.rs:97-193,274-352`
- Modify: `crates/codegen/xai-grok-shell/src/agent/config.rs:3886-3931,4305-4717`

- [ ] **Step 1: Write failing adapter tests for refresh, cache fallback, and unchanged selection**

In Plan 2's `provider_catalog.rs` test module, add wiremock-backed tests:

```rust
#[tokio::test]
async fn refresh_publishes_discovered_models_without_changing_selection() {
    let fixture = AdapterFixture::new().await;
    fixture.configure_dynamic(DynamicProviderConfig {
        id: ProviderId::new("gateway").unwrap(),
        name: "Gateway".into(),
        base_url: fixture.server.uri() + "/v1",
        discover: true,
        unauthenticated: true,
        ..DynamicProviderConfig::new("gateway", "Gateway", fixture.server.uri() + "/v1").unwrap()
    });
    fixture.mount_models(serde_json::json!({"data":[{"id":"openai/gpt-4o"},{"id":"best-effort"}]})).await;
    let before = fixture.models.current_model_id();
    let event = fixture.adapter.refresh_dynamic(&ProviderId::new("gateway").unwrap()).await.unwrap();
    assert_eq!(event, ProviderCatalogEvent::DynamicRefreshComplete { provider_id: ProviderId::new("gateway").unwrap(), model_count: 2, cached: false });
    assert_eq!(fixture.models.current_model_id(), before);
    assert!(fixture.adapter.snapshot().model(&ProviderId::new("gateway").unwrap(), &ModelId::new("openai/gpt-4o").unwrap()).is_some());
}

#[tokio::test]
async fn offline_refresh_publishes_cached_models_and_warning() {
    let fixture = AdapterFixture::with_cached_models("janus", &["best-effort"]).await;
    fixture.configure_dynamic(janus_preset());
    let event = fixture.adapter.refresh_dynamic(&ProviderId::new("janus").unwrap()).await.unwrap();
    assert_eq!(event, ProviderCatalogEvent::DynamicRefreshComplete { provider_id: ProviderId::new("janus").unwrap(), model_count: 1, cached: true });
    assert!(fixture.adapter.snapshot().model(&ProviderId::new("janus").unwrap(), &ModelId::new("best-effort").unwrap()).is_some());
}

#[test]
fn non_loopback_http_is_rejected_before_model_becomes_sampleable() {
    let fixture = AdapterFixture::new_sync();
    let mut config = DynamicProviderConfig::new("lan", "LAN", "http://192.168.1.20:20128/v1").unwrap();
    config.discover = true;
    let error = fixture.adapter.configure_dynamic(config).unwrap_err();
    assert!(matches!(error, ProviderAdapterError::Http(HttpError::InsecureHttpDenied)));
    assert!(fixture.adapter.snapshot().model(&ProviderId::new("lan").unwrap(), &ModelId::new("best-effort").unwrap()).is_none());
}
```

Fixture helpers are test-only and must construct a real `Catalog`, temporary auth store, temporary `DynamicCache`, and `wiremock::MockServer`; they must not alter process-global `GROK_HOME`.

- [ ] **Step 2: Run and verify missing refresh APIs fail compilation**

Run: `cargo test -p xai-grok-shell agent::provider_catalog::tests::refresh_ -- --nocapture`

Expected: compilation fails because `refresh_dynamic` and dynamic refresh event variants do not exist.

- [ ] **Step 3: Add typed adapter requests and events**

Extend the Plan 2 adapter with:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderCatalogEvent {
    DynamicRefreshStarted { provider_id: ProviderId },
    DynamicRefreshComplete { provider_id: ProviderId, model_count: usize, cached: bool },
    DynamicRefreshFailed { provider_id: ProviderId, message: String },
    JanusHealthComplete { healthy: bool, message: String },
}

impl ProviderCatalogAdapter {
    pub(crate) async fn refresh_dynamic(
        &self,
        provider_id: &ProviderId,
    ) -> Result<ProviderCatalogEvent, ProviderAdapterError>;

    pub(crate) async fn check_health(
        &self,
        provider_id: &ProviderId,
    ) -> Result<ProviderCatalogEvent, ProviderAdapterError>;
}
```

`refresh_dynamic` resolves the credential only at request time, calls catalog `get_bounded` and `parse_model_list`, merges static models and exact metadata, stores successful results, replaces that provider's dynamic layer, and publishes the returned immutable snapshot. On failure, it merges cached models and returns `cached: true`; if neither static nor cached models exist, it returns `DynamicRefreshFailed` with a concise secret-free message.

Add `CatalogSnapshot::with_dynamic_models(&self, provider_id: &ProviderId, models: Vec<CatalogModel>) -> Arc<CatalogSnapshot>` in this task. It applies the dynamic layer between Plan 2's reviewed overrides and explicit user configuration, rebuilds indexes, and returns a new secret-free snapshot without mutating the prior one.

`configure_dynamic` must derive and validate the inference base, discovery endpoint, and optional health endpoint with the same `validate_url` policy before publishing any model. This prevents a provider from passing secure discovery and later sending prompts or credentials to a disallowed plain-HTTP inference URL.

- [ ] **Step 4: Add stale-trigger and bounded concurrency**

Store a `tokio::sync::Semaphore` with 4 permits in `ProviderCatalogAdapter` and an in-flight `HashSet<ProviderId>` under a short mutex. Add:

```rust
pub(crate) fn refresh_stale_dynamic_in_background(&self) {
    for provider_id in self.configured_stale_dynamic_provider_ids() {
        if !self.mark_refresh_started(&provider_id) { continue; }
        let adapter = self.clone();
        tokio::spawn(async move {
            let _permit = adapter.dynamic_refresh_limit.acquire().await;
            let _ = adapter.refresh_dynamic(&provider_id).await;
            adapter.mark_refresh_finished(&provider_id);
        });
    }
}
```

Invoke it when `/model` or `/providers` opens via Plan 2's adapter hooks. Rendering always uses the current snapshot first; never await network I/O before opening either picker.

- [ ] **Step 5: Extend `ModelsManager` snapshot adoption**

Add `ModelsManager::apply_provider_snapshot(snapshot: Arc<CatalogSnapshot>)`. Convert through Plan 2's shell adapter conversion API, atomically replace the catalog, keep `current_model_id` when it still exists, call `reselect_current_model_if_missing` only if it disappeared, and call `notify_models_updated`. Do not route dynamic providers through the legacy xAI `/v1/models` cache.

- [ ] **Step 6: Resolve credentials at sampler construction only**

Extend Plan 2's catalog-backed `ModelEntry` adapter so `ModelEntry` stores `provider_id: Option<ProviderId>` but no provider credential. Immediately before `sampling_config_for_model`, call `ProviderCatalogAdapter::credential_for`; place the resulting exposed string into the short-lived `ResolvedCredentials`, then into `SamplerConfig.api_key`. Assert in tests:

```rust
let snapshot_json = serde_json::to_string(adapter.snapshot().as_ref()).unwrap();
assert!(!snapshot_json.contains("sk-janus-test"));
let model_json = serde_json::to_string(&entry).unwrap();
assert!(!model_json.contains("sk-janus-test"));
let sampling = adapter.sampling_config_for_catalog_model(&entry).unwrap();
assert_eq!(sampling.api_key.as_deref(), Some("sk-janus-test"));
assert!(!format!("{sampling:?}").contains("sk-janus-test"));
```

- [ ] **Step 7: Run shell adapter and config tests**

Run: `cargo test -p xai-grok-shell agent::provider_catalog::tests && cargo test -p xai-grok-shell agent::config::tests::catalog_provider`

Expected: refresh, cache fallback, unchanged selection, snapshot redaction, and sampler handoff tests pass.

- [ ] **Step 8: Commit adapter orchestration**

```bash
git add crates/codegen/xai-grok-shell/Cargo.toml crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs crates/codegen/xai-grok-shell/src/agent/models.rs crates/codegen/xai-grok-shell/src/agent/config.rs
git commit -m "feat(shell): orchestrate dynamic provider refresh"
```

### Task 9: Implement Janus Setup Commands In The Shell Adapter

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs`
- Modify: `crates/codegen/xai-grok-shell/src/extensions/providers.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/slash_commands.rs`

- [ ] **Step 1: Write failing setup-state tests**

Add to `provider_catalog.rs` tests:

```rust
#[tokio::test]
async fn janus_setup_persists_base_and_optional_key_then_health_checks_and_discovers() {
    let server = MockInferenceServer::start_with_required_auth(
        vec![MockModelEntry::new("openai/gpt-4o"), MockModelEntry::new("best-effort")],
        "sk-janus-test",
    ).await.unwrap();
    let fixture = AdapterFixture::new_with_mock(&server).await;
    let result = fixture.adapter.setup_janus(JanusSetupRequest {
        base_url: server.url(),
        api_key: Some(SecretString::new("sk-janus-test")),
        allow_insecure_http: false,
    }).await.unwrap();
    assert_eq!(result, JanusSetupResult::Ready { model_count: 2 });
    assert_eq!(fixture.auth_store.resolve(&ProviderId::new("janus").unwrap()).unwrap().expose(), "sk-janus-test");
    assert!(fixture.adapter.snapshot().model(&ProviderId::new("janus").unwrap(), &ModelId::new("openai/gpt-4o").unwrap()).is_some());
}

#[tokio::test]
async fn janus_setup_reports_auth_and_empty_list_exactly() {
    let fixture = AdapterFixture::janus().await;
    fixture.mock.set_health(StatusCode::UNAUTHORIZED, serde_json::json!({"error":"bad key"}));
    assert_eq!(fixture.adapter.setup_janus(request(None)).await.unwrap(), JanusSetupResult::Failed { message: "Janus rejected the API key. Update the Janus key in /providers.".into(), cached_models: 0 });
    fixture.mock.set_health(StatusCode::OK, serde_json::json!({"status":"ok"}));
    fixture.mock.set_models(Vec::new());
    assert_eq!(fixture.adapter.setup_janus(request(None)).await.unwrap(), JanusSetupResult::Empty);
}
```

- [ ] **Step 2: Run and verify missing setup API fails compilation**

Run: `cargo test -p xai-grok-shell agent::provider_catalog::tests::janus_setup -- --nocapture`

Expected: compilation fails because `JanusSetupRequest`, `JanusSetupResult`, and `setup_janus` do not exist.

- [ ] **Step 3: Implement the setup transaction**

Add exact types:

```rust
pub(crate) struct JanusSetupRequest {
    pub(crate) base_url: String,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) allow_insecure_http: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum JanusSetupResult {
    Ready { model_count: usize },
    Empty,
    Failed { message: String, cached_models: usize },
}
```

Implement `ProviderCatalogAdapter::setup_janus` in this order:

1. Start from `janus_preset()` and replace `base_url` and `allow_insecure_http` from the request.
2. Validate the normalized health and model URLs before writing config or auth.
3. Persist the provider config through Plan 2's atomic global config writer.
4. If a key was supplied, persist it through Plan 2's `0600` auth store; if omitted, leave an existing key unchanged. A separate explicit `ClearProviderCredential` action removes it.
5. Perform `GET /v1/health` with the optional key.
6. On 2xx health, perform `GET /v1/models` and apply the same merge/cache path as generic discovery.
7. Return `Ready`, `Empty`, or `Failed`; failures include the number of still-visible cached models.

Never log the request with `Debug`; log only provider ID, normalized URL, status category, model count, and whether a credential was present.

- [ ] **Step 4: Expose setup through the existing provider ACP boundary**

Add this Plan 2 provider extension method:

```text
x.ai/providers/setup_janus
{ baseUrl: string, apiKey: string|null, allowInsecureHttp: bool }
-> { state: "ready"|"empty"|"failed", modelCount: number, cachedModels: number, message: string|null }
```

Define serde request/response DTOs in `extensions/providers.rs`. The route validates the request, moves `apiKey` directly into `SecretString`, invokes `ProviderCatalogAdapter::setup_janus`, drops the request secret, and returns only the secret-free response. Register the method in `acp_agent.rs` beside Plan 2's provider list/store/clear/refresh routes. Add an extension test that serializes the response and proves it contains neither the supplied key nor an `apiKey` field.

- [ ] **Step 5: Keep `/login` as an alias to Plan 2 provider management**

In `session/slash_commands.rs`, ensure both built-ins resolve to the same provider-management action:

```rust
BuiltinCommand {
    name: "providers",
    description: "Configure model providers",
    argument_hint: None,
    aliases: &["login"],
    gate: BuiltinGate::AlwaysOn,
    resolve: |_args| BuiltinAction::Providers,
},
```

Assert the command registry contains no standalone `LoginCommand`; Plan 2 deletes that command and registers `login` only as the `ProvidersCommand` alias. Existing xAI OAuth remains a selectable entry inside provider management.

- [ ] **Step 6: Run setup, ACP route, and command tests**

Run: `cargo test -p xai-grok-shell agent::provider_catalog::tests::janus_setup && cargo test -p xai-grok-shell extensions::providers::tests::setup_janus && cargo test -p xai-grok-shell session::slash_commands::tests::providers`

Expected: authenticated, unauthenticated, empty, 401/403, secret-free ACP response, and alias tests pass.

- [ ] **Step 7: Commit shell setup flow**

```bash
git add crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs crates/codegen/xai-grok-shell/src/extensions/providers.rs crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs crates/codegen/xai-grok-shell/src/session/slash_commands.rs
git commit -m "feat(shell): add Janus provider setup flow"
```

### Task 10: Add Janus Setup UX To The Plan 2 Provider Picker

**Files:**
- Modify: `crates/codegen/xai-grok-pager/src/providers.rs`
- Modify: `crates/codegen/xai-grok-pager/src/slash/commands/providers.rs`
- Modify: `crates/codegen/xai-grok-pager/src/views/providers_modal.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/actions.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/dispatch/auth.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/dispatch/router.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/effects/mod.rs`

- [ ] **Step 1: Write failing reducer/view tests for every Janus screen state**

In `app/dispatch/auth.rs` tests, add:

```rust
#[test]
fn selecting_janus_starts_with_exact_default_base() {
    let mut app = provider_app();
    dispatch(Action::SelectProvider("janus".into()), &mut app);
    assert_eq!(app.provider_setup, Some(ProviderSetupState::JanusBaseUrl {
        value: "http://127.0.0.1:20128/v1".into(),
        insecure_confirmation_required: false,
    }));
}

#[test]
fn janus_key_screen_is_optional_and_uses_hint() {
    let state = ProviderSetupState::JanusApiKey { value: String::new() };
    let rendered = render_provider_setup_text(&state, 80);
    assert!(rendered.contains("Optional API key"));
    assert!(rendered.contains("optional, starts with sk-janus-"));
    assert!(!rendered.contains("sk-janus-test"));
}

#[test]
fn offline_result_keeps_cached_models_visible() {
    let mut app = provider_app();
    dispatch(Action::TaskComplete(TaskResult::JanusSetupComplete(
        JanusSetupResponse {
            state: JanusSetupState::Failed,
            model_count: 0,
            cached_models: 2,
            message: Some("Could not connect to Janus at http://127.0.0.1:20128/v1/health. Check that the Janus service is running and the base URL is correct.".into()),
        }
    )), &mut app);
    assert!(matches!(app.provider_setup, Some(ProviderSetupState::Result { cached_models: 2, .. })));
}
```

Add view tests asserting an empty list says `Janus is healthy but returned no models.`, 401/403 directs to `/providers`, and LAN HTTP displays a confirmation before an effect is emitted.

- [ ] **Step 2: Run and verify missing state/action variants fail compilation**

Run: `cargo test -p xai-grok-pager app::dispatch::auth::tests::janus -- --nocapture`

Expected: compilation fails because Janus setup state and task-result variants do not exist.

- [ ] **Step 3: Add typed pager ACP response, states, and effects**

Add to `providers.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JanusSetupState { Ready, Empty, Failed }

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JanusSetupResponse {
    pub state: JanusSetupState,
    pub model_count: usize,
    pub cached_models: usize,
    pub message: Option<String>,
}
```

Add to `actions.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderSetupState {
    JanusBaseUrl { value: String, insecure_confirmation_required: bool },
    JanusApiKey { value: String },
    JanusChecking,
    Result { message: String, cached_models: usize },
}

pub enum Action {
    SelectProvider(String),
    SetJanusBaseUrl(String),
    ConfirmInsecureProviderUrl,
    SetJanusApiKey(String),
    SubmitJanusSetup,
}

pub enum Effect {
    SetupJanus { base_url: String, api_key: Option<String>, allow_insecure_http: bool },
}

pub enum TaskResult {
    JanusSetupComplete(JanusSetupResponse),
}
```

Use Plan 2's pager-safe provider DTOs at this crate boundary; do not import catalog `ProviderId` or `SecretString` into pager state. The plain key exists only in the focused input widget and the `SetupJanus` effect, is moved into ACP JSON at execution, and is cleared from the widget immediately. Implement manual `Debug` for `Effect` so `SetupJanus` prints only `has_api_key`.

- [ ] **Step 4: Implement reducer sequence and background effect**

The sequence is exactly: provider selection -> editable base URL -> optional masked key -> setup spinner -> result. For `http://` non-loopback, stop after base URL entry and show `This URL sends prompts and credentials over plain HTTP. Continue only if you trust this network.`; emit `SetupJanus` only after `ConfirmInsecureProviderUrl`.

In `app/effects/mod.rs`, execute `Effect::SetupJanus` through `x.ai/providers/setup_janus` and return `TaskResult::JanusSetupComplete`; do not block rendering. On completion, request the updated provider/model state through Plan 2's ACP methods and retain the current model selection.

- [ ] **Step 5: Render concise status with cached model count**

Use these exact strings:

```text
Checking Janus health at {url}
Janus is ready. {count} models available.
Janus is healthy but returned no models.
{error} {count} cached models remain available.
```

Do not display or retain the key after submit. Keep slash/combo IDs unmodified in the provider detail and `/model` rows.

- [ ] **Step 6: Run pager provider tests**

Run: `cargo test -p xai-grok-pager providers -- --nocapture`

Expected: default URL, optional masked key, insecure confirmation, spinner progression, empty list, auth guidance, and cached-offline tests pass.

- [ ] **Step 7: Commit setup UX**

```bash
git add crates/codegen/xai-grok-pager/src/providers.rs crates/codegen/xai-grok-pager/src/slash/commands/providers.rs crates/codegen/xai-grok-pager/src/views/providers_modal.rs crates/codegen/xai-grok-pager/src/app/actions.rs crates/codegen/xai-grok-pager/src/app/dispatch/auth.rs crates/codegen/xai-grok-pager/src/app/dispatch/router.rs crates/codegen/xai-grok-pager/src/app/effects/mod.rs
git commit -m "feat(pager): guide Janus provider setup"
```

### Task 11: Prove Exact IDs And End-To-End Janus Inference

**Files:**
- Create: `crates/codegen/xai-grok-shell/tests/test_dynamic_provider_e2e.rs`
- Create: `crates/codegen/xai-grok-pager/tests/pty_e2e_provider_setup.rs`
- Modify: `crates/codegen/xai-grok-test-support/src/headless.rs`
- Test: `crates/codegen/xai-grok-pager/Cargo.toml` (retain the existing `xai-grok-pager-pty-harness` and `xai-grok-test-support` dev-dependencies unchanged)

- [ ] **Step 1: Write the failing shell end-to-end test**

Create `test_dynamic_provider_e2e.rs` using an isolated temp home and `MockInferenceServer`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn janus_discovers_slash_id_and_infers_with_same_id_and_bearer() {
    let server = MockInferenceServer::start_with_required_auth(
        vec![MockModelEntry::new("openai/gpt-4o"), MockModelEntry::new("best-effort")],
        "sk-janus-e2e",
    ).await.unwrap();
    server.set_response("Janus answer");
    let home = tempfile::tempdir().unwrap();
    let harness = HeadlessHarness::builder()
        .home(home.path())
        .provider_config(serde_json::json!({
            "id": "janus",
            "base_url": server.url(),
            "api_backend": "chat_completions",
            "discover": true
        }))
        .provider_key("janus", "sk-janus-e2e")
        .build().await.unwrap();
    harness.setup_janus(server.url(), Some("sk-janus-e2e")).await.unwrap();
    harness.switch_model("janus/openai/gpt-4o").await.unwrap();
    let response = harness.prompt("hello Janus").await.unwrap();
    assert!(response.contains("Janus answer"));
    let requests = server.requests();
    assert!(requests.iter().any(|r| r.method == "GET" && r.path == "/v1/health" && r.header("authorization") == Some("Bearer sk-janus-e2e")));
    assert!(requests.iter().any(|r| r.method == "GET" && r.path == "/v1/models" && r.header("authorization") == Some("Bearer sk-janus-e2e")));
    let inference = requests.iter().find(|r| r.path == "/v1/chat/completions").unwrap();
    assert_eq!(inference.header("authorization"), Some("Bearer sk-janus-e2e"));
    assert_eq!(inference.body.as_ref().unwrap()["model"], "openai/gpt-4o");
}
```

Add the exact helper methods shown (`provider_config`, `provider_key`, `setup_janus`, `switch_model`) to `xai-grok-test-support::HeadlessHarnessBuilder`/`HeadlessHarness`; each helper must call the shell adapter, and none may bypass it with a handcrafted `SamplerConfig`.

- [ ] **Step 2: Run and verify the E2E test fails before wiring is complete**

Run: `cargo test -p xai-grok-shell --test test_dynamic_provider_e2e -- --nocapture`

Expected: the test fails at provider refresh or model switch until adapter-to-model-to-sampler wiring is complete.

- [ ] **Step 3: Finish only the wiring exposed by the test**

Ensure catalog keys use `provider/model` for picker uniqueness while `CatalogModel.id` remains the exact upstream ID. Selecting `janus/openai/gpt-4o` must produce sampler `model = "openai/gpt-4o"`, `base_url = server.url()`, and `ApiBackend::ChatCompletions`. Selecting `janus/best-effort` must send `model = "best-effort"`.

- [ ] **Step 4: Add PTY setup smoke test**

In `pty_e2e_provider_setup.rs`, script:

1. Start `MockInferenceServer` with `openai/gpt-4o` and `best-effort`.
2. Open `/providers`, choose `Janus`, accept the prefilled mock base URL, skip the optional key for an unauthenticated server, and wait for `Janus is ready. 2 models available.`
3. Open `/model`, type `openai/gpt-4o`, select it, send `hello`, and assert the fixed response renders.
4. Drop the server, reopen `/providers`, trigger refresh, and assert `2 cached models remain available.` while `/model` still lists both IDs.

Use the existing `xai-grok-pager-pty-harness` scripted-event API and a 10-second per-screen deadline; include `server.request_log_summary()` in assertion failures.

- [ ] **Step 5: Run both E2E tests**

Run: `cargo test -p xai-grok-shell --test test_dynamic_provider_e2e -- --nocapture`

Expected: pass; health, model-list, and inference requests all carry the expected bearer, and inference carries exact model ID `openai/gpt-4o`.

Run: `cargo test -p xai-grok-pager --test pty_e2e_provider_setup -- --nocapture`

Expected: pass; setup is responsive and cached models remain visible after the mock server stops.

- [ ] **Step 6: Commit end-to-end coverage**

```bash
git add crates/codegen/xai-grok-shell/tests/test_dynamic_provider_e2e.rs crates/codegen/xai-grok-test-support/src/headless.rs crates/codegen/xai-grok-pager/tests/pty_e2e_provider_setup.rs
git commit -m "test: cover Janus discovery and inference end to end"
```

### Task 12: Document Configuration And Run Final Security Verification

**Files:**
- Create: `docs/configuration/providers.md`
- Test: `docs/` (the repository has no `docs/README.md`; do not create an index solely for this plan)

- [ ] **Step 1: Write the provider configuration documentation**

Create `docs/configuration/providers.md` with these complete examples:

```toml
# Generic authenticated OpenAI-compatible provider.
[provider.gateway]
name = "Team Gateway"
base_url = "https://gateway.example/v1"
api_backend = "chat_completions"
discover = true
models_endpoint = "/v1/models"

# Generic unauthenticated loopback provider with a static metadata override.
[provider.local]
name = "Local Gateway"
base_url = "http://127.0.0.1:9000/v1"
unauthenticated = true
discover = true

[provider.local.models."openai/gpt-4o"]
name = "Local GPT-4o"
context_window = 128000
reasoning = false

# Janus preset; every field shown below is the preset default.
[provider.janus]
preset = "janus"
base_url = "http://127.0.0.1:20128/v1"
api_backend = "chat_completions"
health_endpoint = "/v1/health"
models_endpoint = "/v1/models"
discover = true
```

State that credentials are entered through `/providers` and stored by Plan 2's auth store, not placed in TOML. Explain that HTTPS is required except loopback HTTP, non-loopback HTTP needs `allow_insecure_http = true`, redirects strip auth cross-origin, static fields override discovered fields, exact IDs including `openai/gpt-4o` and `best-effort` are preserved, successful empty lists replace cache, and failed refreshes retain last-known-good models.

- [ ] **Step 2: Run formatting and focused checks**

Run: `cargo fmt --all -- --check`

Expected: exit 0 with no diff.

Run: `cargo clippy -p xai-grok-catalog -p xai-grok-sampler -p xai-grok-test-support -p xai-grok-shell -p xai-grok-pager --all-targets -- -D warnings`

Expected: exit 0 with no warnings.

- [ ] **Step 3: Run all affected tests**

Run: `cargo test -p xai-grok-catalog -p xai-grok-sampler -p xai-grok-test-support -p xai-grok-shell -p xai-grok-pager`

Expected: exit 0; all unit, wiremock integration, mock-server, shell E2E, and pager tests pass.

- [ ] **Step 4: Run explicit secret and unsafe-transport scans**

Run: `rg -n 'api_key\s*=\s*%|auth_header_prefix|x_api_key_prefix|Authorization.*\{.*key|Bearer \{.*key' crates/codegen/xai-grok-sampler crates/codegen/xai-grok-catalog crates/codegen/xai-grok-shell`

Expected: no output.

Run: `rg -n 'redirect\(reqwest::redirect::Policy::(limited|default)|\.json::<.*OpenAi|response\.bytes\(\)' crates/codegen/xai-grok-catalog/src`

Expected: no output; discovery uses `Policy::none` and bounded streaming.

Run: `rg -n 'SecretString|api_key|authorization' crates/codegen/xai-grok-catalog/src/dynamic_cache.rs`

Expected: only negative-test assertions or explanatory comments; no serialized cache field or cache value contains credentials.

- [ ] **Step 5: Inspect the final diff for scope**

Run: `git status --short && git diff --stat && git diff --check`

Expected: only files in this responsibility map are changed, `git diff --check` exits 0, and there are no telemetry changes or foundational provider-picker/catalog rewrites.

- [ ] **Step 6: Commit documentation and verification fixes**

```bash
git add docs/configuration/providers.md
git commit -m "docs: explain dynamic providers and Janus"
```
