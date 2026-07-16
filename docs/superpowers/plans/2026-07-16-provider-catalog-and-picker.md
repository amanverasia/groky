# Provider Catalog And Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an offline-first, refreshable provider catalog, secure provider API-key management, configured-provider model availability, and provider-aware model picking while preserving existing xAI OAuth, API-key, and default-model behavior.

**Architecture:** A dependency-neutral `xai-grok-catalog` crate owns normalized provider/model DTOs, deterministic models.dev ingestion, field-aware layering, immutable snapshots, refresh/cache mechanics, and credential precedence over caller-supplied secret sources. `xai-grok-shell` is the only adapter to existing `ModelEntry`/`ModelInfo`, auth storage, `ModelsManager`, and ACP; `xai-grok-pager` consumes secret-free ACP metadata and provider extension methods. The existing xAI entitlement `/models` pipeline remains separate, and provider entries carry an explicit credential policy so neither an xAI OAuth token nor `XAI_API_KEY` can fall through to a non-xAI origin.

**Tech Stack:** Rust 2024, Cargo workspace, serde/serde_json, indexmap, reqwest 0.12, tokio, arc-swap, chrono, url, tempfile, wiremock, agent-client-protocol 0.10.4, ratatui picker/ArgPicker, shell `AuthStore` and secure file writer.

---

## File Responsibility Map

**Create:**
- `crates/codegen/xai-grok-catalog/Cargo.toml` - dependency-neutral catalog crate manifest; it must not depend on shell, pager, sampler, auth, ACP, or sampling-types.
- `crates/codegen/xai-grok-catalog/src/lib.rs` - public exports and embedded snapshot loader.
- `crates/codegen/xai-grok-catalog/src/types.rs` - validated `ProviderId`/`ModelId` newtypes, normalized secret-free provider/model DTOs, patch DTOs, protocol enum, status/error types, and immutable `CatalogSnapshot`.
- `crates/codegen/xai-grok-catalog/src/normalize.rs` - bounded models.dev parsing, stable filtering, protocol inference, canonical ordering, and whole-document validation.
- `crates/codegen/xai-grok-catalog/src/layer.rs` - field-aware provider/model patch merge by provider ID and model ID.
- `crates/codegen/xai-grok-catalog/src/credentials.rs` - redacted secret wrapper, caller-injected credential sources, precedence, and provider availability classification.
- `crates/codegen/xai-grok-catalog/src/cache.rs` - schema-versioned, secret-free cache envelope and atomic cache I/O.
- `crates/codegen/xai-grok-catalog/src/refresh.rs` - 24-hour stale check, bounded conditional models.dev refresh, last-known-good semantics, and refresh status.
- `crates/codegen/xai-grok-catalog/src/bin/generate_catalog.rs` - deterministic generator CLI supporting fetched input and checked local input.
- `crates/codegen/xai-grok-catalog/data/models-dev.json` - committed normalized embedded snapshot.
- `crates/codegen/xai-grok-catalog/data/overrides.json` - committed reviewed provider/model patches and exclusions.
- `crates/codegen/xai-grok-catalog/tests/fixtures/models-dev-small.json` - representative raw models.dev fixture including unknown fields and unsupported protocols.
- `crates/codegen/xai-grok-catalog/tests/generator.rs` - fixture normalization and byte-for-byte determinism tests.
- `crates/codegen/xai-grok-catalog/tests/refresh.rs` - conditional request, timeout, size bound, cache fallback, and secret scan tests.
- `scripts/update-provider-catalog.sh` - reproducible maintainer entry point for fetching and regenerating the committed snapshot.
- `scripts/check-provider-catalog.sh` - deterministic committed snapshot check used locally and in CI.
- `.github/workflows/provider-catalog.yml` - catalog formatting, tests, and deterministic snapshot check.
- `crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs` - shell-only adapter from neutral catalog DTOs to `ModelEntry`, provider configuration overlay, provider credential sources, and composed catalog IDs.
- `crates/codegen/xai-grok-shell/src/extensions/providers.rs` - typed `x.ai/providers/list`, `x.ai/providers/store_key`, `x.ai/providers/clear_key`, and `x.ai/providers/refresh` ACP methods.
- `crates/codegen/xai-grok-pager/src/providers.rs` - provider ACP request/response DTOs and pager provider state.
- `crates/codegen/xai-grok-pager/src/slash/commands/providers.rs` - `/providers` command with `/login` alias and provider argument rows.
- `crates/codegen/xai-grok-pager/src/views/providers_modal.rs` - provider list/status and masked API-key entry modal.

**Modify:**
- `Cargo.toml` - register `xai-grok-catalog` as a workspace member/dependency after Plan 1 has removed telemetry members/dependencies.
- `Cargo.lock` - record the new workspace crate and dependency graph.
- `crates/codegen/xai-grok-shell/Cargo.toml` - depend on `xai-grok-catalog`; add only test dependencies needed by adapter integration tests.
- `crates/codegen/xai-grok-shell/src/agent/config.rs` - parse `[provider.<id>]`, retain `resolve_model_list`, add provider identity/credential policy to `ModelEntry`, make credential resolution provider-safe, and emit provider ACP metadata.
- `crates/codegen/xai-grok-shell/src/agent/config_model_override_parse.rs` - accept and test provider association on explicit model overrides.
- `crates/codegen/xai-grok-shell/src/agent/models.rs` - compose existing xAI models with configured provider models, own `CatalogManager`, trigger non-blocking refresh, and publish replacement snapshots without changing a valid current selection.
- `crates/codegen/xai-grok-shell/src/auth/model.rs` - define stable `provider::<provider_id>` scope construction without changing `API_KEY_SCOPE`.
- `crates/codegen/xai-grok-shell/src/auth/storage.rs` - read/store/clear provider keys through existing atomic `0600` writer while preserving sibling OAuth/xAI scopes.
- `crates/codegen/xai-grok-shell/src/auth/mod.rs` - export provider-key storage APIs to the shell adapter.
- `crates/codegen/xai-grok-shell/src/extensions/mod.rs` - register provider extension module.
- `crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs` - route provider ACP extension methods.
- `crates/codegen/xai-grok-shell/src/session/slash_commands.rs` - advertise `/providers` and `/login` alias on the shell ACP surface.
- `crates/codegen/xai-grok-shell/src/leader/server.rs` - forward `x.ai/providers/update` alongside `x.ai/models/update` to all clients.
- `crates/codegen/xai-grok-pager/Cargo.toml` - add `fuzzy-matcher` for explicit weighted model matching.
- `crates/codegen/xai-grok-pager/src/slash/commands/mod.rs` - register `ProvidersCommand` and remove the old pager-local OAuth-only `LoginCommand` registration.
- `crates/codegen/xai-grok-pager/src/slash/commands/model.rs` - provider-aware rows, indexed match text, deterministic no-query order, and current-first ordering.
- `crates/codegen/xai-grok-pager/src/slash/command.rs` - add optional explicit match score to `ArgItem`.
- `crates/codegen/xai-grok-pager/src/views/picker.rs` - honor explicit item scores before the existing stable tie-break.
- `crates/codegen/xai-grok-pager/src/views/modal.rs` - add provider modal state.
- `crates/codegen/xai-grok-pager/src/acp/model_state.rs` - parse provider/context/reasoning metadata and preserve selection during updates.
- `crates/codegen/xai-grok-pager/src/app/actions.rs` - provider list/key/refresh actions, effects, and task results.
- `crates/codegen/xai-grok-pager/src/app/dispatch/router.rs` - route provider actions.
- `crates/codegen/xai-grok-pager/src/app/dispatch/auth.rs` - retain existing `dispatch_login` for selecting xAI inside the provider modal; do not alter OAuth mechanics.
- `crates/codegen/xai-grok-pager/src/app/effects/mod.rs` - execute typed provider ACP calls and return secret-free task results.
- `crates/codegen/xai-grok-pager/src/app/acp_handler/mod.rs` - handle `x.ai/providers/update` and refresh model/provider state.
- `crates/codegen/xai-grok-pager/src/app/acp_handler/settings.rs` - preserve current selection on background model replacement and show cached-data status.

**Delete:**
- `crates/codegen/xai-grok-pager/src/slash/commands/login.rs` - its OAuth action becomes the xAI row inside `/providers`; `/login` is an alias of the provider command.

### Task 1: Create Neutral Catalog Types And Bounded Normalization

**Files:**
- Create: `crates/codegen/xai-grok-catalog/Cargo.toml`
- Create: `crates/codegen/xai-grok-catalog/src/lib.rs`
- Create: `crates/codegen/xai-grok-catalog/src/types.rs`
- Create: `crates/codegen/xai-grok-catalog/src/normalize.rs`
- Create: `crates/codegen/xai-grok-catalog/tests/fixtures/models-dev-small.json`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the raw fixture and failing normalization test**

Use a two-provider fixture: `openai` with `api`, `env`, `npm: "@ai-sdk/openai"`, `gpt-5` limits/reasoning/cost plus an unknown field; `anthropic` with `npm: "@ai-sdk/anthropic"` and `claude-sonnet-4` limits/cost. Add a third `unsupported` provider with `npm: "@ai-sdk/google"` so filtering is explicit. In `src/normalize.rs` add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_fields_and_ignores_unknown_fields() {
        let raw = include_bytes!("../tests/fixtures/models-dev-small.json");
        let catalog = normalize_models_dev(raw, NormalizationLimits::default()).unwrap();
        assert_eq!(catalog.providers.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["anthropic", "openai"]);
        let gpt = catalog.provider_str("openai").unwrap().model_str("gpt-5").unwrap();
        assert_eq!(gpt.protocol, Protocol::Responses);
        assert_eq!(gpt.context_window, Some(400_000));
        assert_eq!(gpt.cost.as_ref().unwrap().input_per_million, 1.25);
        assert!(gpt.reasoning);
    }

    #[test]
    fn rejects_the_whole_document_when_limits_are_exceeded() {
        let err = normalize_models_dev(br#"{"p":{"id":"p","name":"p","models":{}}}"#, NormalizationLimits { max_providers: 0, ..Default::default() }).unwrap_err();
        assert!(matches!(err, CatalogError::LimitExceeded { field: "providers", .. }));
    }
}
```

- [ ] **Step 2: Run the focused test and verify the missing crate/API failure**

Run: `cargo test -p xai-grok-catalog normalize::tests -- --nocapture`

Expected: FAIL because workspace package `xai-grok-catalog` and `normalize_models_dev` do not exist.

- [ ] **Step 3: Add the crate and normalized DTOs**

Register the member and workspace dependency, then define the core DTOs exactly as follows (derive `Serialize`, `Deserialize`, `Clone`, `Debug`, `PartialEq`; add `Eq` where floats are absent):

```rust
pub const CATALOG_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, CatalogError>;
    pub fn as_str(&self) -> &str;
}

impl ModelId {
    pub fn new(value: impl Into<String>) -> Result<Self, CatalogError>;
    pub fn as_str(&self) -> &str;
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol { #[default] ChatCompletions, Responses, Messages, Unsupported }

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelCost {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CatalogModel {
    pub id: ModelId,
    pub name: String,
    pub protocol: Protocol,
    pub context_window: Option<u64>,
    pub reasoning: bool,
    pub cost: Option<ModelCost>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CatalogProvider {
    pub id: ProviderId,
    pub name: String,
    pub api_base_url: String,
    pub env_vars: Vec<String>,
    pub unauthenticated: bool,
    pub models: Vec<CatalogModel>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalizedCatalog {
    pub schema_version: u32,
    pub providers: Vec<CatalogProvider>,
}

#[derive(Clone, Copy, Debug)]
pub struct NormalizationLimits {
    pub max_bytes: usize,
    pub max_providers: usize,
    pub max_models_per_provider: usize,
    pub max_string_bytes: usize,
}

impl Default for NormalizationLimits {
    fn default() -> Self { Self { max_bytes: 8 * 1024 * 1024, max_providers: 512, max_models_per_provider: 4096, max_string_bytes: 1024 } }
}
```

Implement raw serde structs with `#[serde(default)]`, reject empty IDs, provider IDs outside `[a-z0-9_-]`, model IDs containing control characters, zero context windows, non-finite/negative costs, and over-limit counts/strings. Implement `Deserialize` for both ID newtypes through `new` so JSON/TOML cannot bypass validation; `ProviderId` is limited to 64 bytes and `ModelId` to 512 bytes. Map `@ai-sdk/anthropic` to `Messages`, `@ai-sdk/openai` to `Responses`, and `@ai-sdk/openai-compatible` to `ChatCompletions`; retain `Unsupported` only long enough to exclude it with a typed local diagnostic. Sort providers and models lexicographically by ID and sort/deduplicate environment names. Add `provider(&ProviderId)` and `model(&ModelId)` accessors, plus string convenience lookups `provider_str` and `model_str` for fixture assertions.

- [ ] **Step 4: Run normalization tests**

Run: `cargo test -p xai-grok-catalog normalize::tests -- --nocapture`

Expected: PASS; output reports 2 passed and no network access.

- [ ] **Step 5: Commit the neutral catalog foundation**

```bash
git add Cargo.toml Cargo.lock crates/codegen/xai-grok-catalog
git commit -m "feat: add normalized provider catalog types"
```

### Task 2: Implement Field-Aware Layering And Reviewed Overrides

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/layer.rs`
- Create: `crates/codegen/xai-grok-catalog/data/overrides.json`
- Modify: `crates/codegen/xai-grok-catalog/src/types.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`

- [ ] **Step 1: Write failing partial-field and exclusion tests**

```rust
#[test]
fn higher_layer_replaces_only_fields_it_supplies() {
    let base = catalog_with_openai("OpenAI", "https://api.openai.com/v1", "GPT 5", 400_000);
    let patch = CatalogPatch { providers: vec![ProviderPatch {
        id: ProviderId::new("openai").unwrap(), name: None, api_base_url: None, env_vars: None,
        unauthenticated: None, exclude: false,
        models: vec![ModelPatch { id: ModelId::new("gpt-5").unwrap(), name: Some("GPT-5".into()), context_window: None, protocol: None, reasoning: None, cost: None, exclude: false }],
    }] };
    let merged = apply_patch(base, patch).unwrap();
    let provider = merged.provider_str("openai").unwrap();
    assert_eq!(provider.api_base_url, "https://api.openai.com/v1");
    assert_eq!(provider.model_str("gpt-5").unwrap().name, "GPT-5");
    assert_eq!(provider.model_str("gpt-5").unwrap().context_window, Some(400_000));
}

#[test]
fn exclusions_remove_known_incompatible_entries() {
    let merged = apply_patch(catalog_with_two_models(), patch_excluding("openai", "legacy")).unwrap();
    assert!(merged.provider_str("openai").unwrap().model_str("legacy").is_none());
}
```

- [ ] **Step 2: Run the tests and verify missing patch types**

Run: `cargo test -p xai-grok-catalog layer::tests -- --nocapture`

Expected: FAIL with unresolved `CatalogPatch`, `ProviderPatch`, `ModelPatch`, and `apply_patch`.

- [ ] **Step 3: Implement patch DTOs and deterministic merge**

Add patch fields as `Option<T>` so absence differs from false/empty. Merge providers/models through `BTreeMap`, apply all supplied scalar/collection fields, process `exclude` before additions, revalidate the complete result, and return lexicographically sorted vectors. `overrides.json` must be a valid explicit object:

```json
{
  "providers": [
    {
      "id": "xai",
      "api_base_url": "https://api.x.ai/v1",
      "env_vars": ["XAI_API_KEY"],
      "models": []
    }
  ]
}
```

Expose `load_overrides()` using `include_bytes!("../data/overrides.json")`; a malformed committed override must fail tests and panic at embedded startup with a path-specific message because it is a release artifact, not remote input.

- [ ] **Step 4: Run all crate tests**

Run: `cargo test -p xai-grok-catalog --lib -- --nocapture`

Expected: PASS, including preservation of base fields and deterministic exclusions.

- [ ] **Step 5: Commit field-aware layering**

```bash
git add crates/codegen/xai-grok-catalog
git commit -m "feat: add field-aware catalog layering"
```

### Task 3: Add Deterministic Generation, Embedded Snapshot, And CI Check

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/bin/generate_catalog.rs`
- Create: `crates/codegen/xai-grok-catalog/data/models-dev.json`
- Create: `crates/codegen/xai-grok-catalog/tests/generator.rs`
- Create: `scripts/update-provider-catalog.sh`
- Create: `scripts/check-provider-catalog.sh`
- Create: `.github/workflows/provider-catalog.yml`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`

- [ ] **Step 1: Write the failing byte-determinism integration test**

```rust
#[test]
fn fixture_generation_is_byte_for_byte_deterministic() {
    let input = fixture("models-dev-small.json");
    let one = tempfile::NamedTempFile::new().unwrap();
    let two = tempfile::NamedTempFile::new().unwrap();
    run_generator(&input, one.path());
    run_generator(&input, two.path());
    assert_eq!(std::fs::read(one.path()).unwrap(), std::fs::read(two.path()).unwrap());
    let text = std::fs::read_to_string(one.path()).unwrap();
    assert!(text.ends_with('\n'));
}
```

- [ ] **Step 2: Verify the generator binary is absent**

Run: `cargo test -p xai-grok-catalog --test generator -- --nocapture`

Expected: FAIL because `CARGO_BIN_EXE_generate_catalog` is undefined.

- [ ] **Step 3: Implement the deterministic generator and scripts**

The binary accepts `--input <path>|--fetch <url> --output <path> [--check]`. It reads/fetches bytes, calls only `normalize_models_dev` then `apply_patch(load_overrides())`, serializes with `serde_json::to_string_pretty`, appends exactly one newline, and in `--check` compares bytes without writing. Use this script content:

```bash
#!/usr/bin/env bash
set -euo pipefail
cargo run -p xai-grok-catalog --bin generate_catalog -- \
  --fetch https://models.dev/api.json \
  --output crates/codegen/xai-grok-catalog/data/models-dev.json "$@"
```

`check-provider-catalog.sh` runs the same command with `--check`. Generate and commit the complete current snapshot. In `lib.rs`, parse `include_bytes!("../data/models-dev.json")`, assert schema version 1, and expose `embedded_catalog() -> NormalizedCatalog`.

- [ ] **Step 4: Add CI that checks deterministic output**

Create a workflow triggered for changes under the crate/scripts/workflow, install the pinned `rust-toolchain.toml`, and run exactly:

```yaml
- run: cargo fmt --all -- --check
- run: cargo test -p xai-grok-catalog
- run: scripts/check-provider-catalog.sh
```

- [ ] **Step 5: Verify generation and embedded loading**

Run: `cargo test -p xai-grok-catalog --test generator && scripts/check-provider-catalog.sh`

Expected: PASS and `Catalog snapshot is current`; `git diff --exit-code -- crates/codegen/xai-grok-catalog/data/models-dev.json` exits 0.

- [ ] **Step 6: Commit deterministic generation**

```bash
git add .github/workflows/provider-catalog.yml scripts crates/codegen/xai-grok-catalog Cargo.lock
git commit -m "build: generate deterministic provider catalog"
```

### Task 4: Add Redacted Credential Resolution And Provider Availability

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/credentials.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/types.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`

- [ ] **Step 1: Write failing precedence, availability, and redaction tests**

```rust
#[test]
fn credential_precedence_is_session_stored_environment_then_model() {
    let sources = CredentialSources {
        session: Some(SecretString::new("session")), stored: Some(SecretString::new("stored")),
        environment: Some(SecretString::new("env")), model: Some(SecretString::new("model")),
    };
    assert_eq!(resolve_credential(sources).unwrap().origin, CredentialOrigin::Session);
}

#[test]
fn secret_display_and_debug_never_expose_value() {
    let secret = SecretString::new("sk-provider-secret");
    assert_eq!(format!("{secret}"), "[REDACTED]");
    assert_eq!(format!("{secret:?}"), "SecretString([REDACTED])");
    assert!(!serde_json::to_string(&ProviderStatus::configured("openai")).unwrap().contains("secret"));
}

#[test]
fn availability_distinguishes_stored_environment_missing_and_unsupported() {
    assert_eq!(classify_provider(&supported(), Some(CredentialOrigin::Stored)), ProviderAvailability::Configured);
    assert_eq!(classify_provider(&supported(), Some(CredentialOrigin::Environment)), ProviderAvailability::Environment);
    assert_eq!(classify_provider(&supported(), None), ProviderAvailability::MissingKey);
    assert_eq!(classify_provider(&unsupported(), None), ProviderAvailability::Unavailable);
}
```

- [ ] **Step 2: Run and verify the credential API is absent**

Run: `cargo test -p xai-grok-catalog credentials::tests -- --nocapture`

Expected: FAIL with unresolved credential/status types.

- [ ] **Step 3: Implement secret-safe credential policy**

Define `SecretString(Vec<u8>)` without serde traits, expose only `expose(&self) -> &str`, redact `Debug`/`Display`, and zero bytes in `Drop`. Define `CredentialSources`, `ResolvedCredential { secret, origin }`, and precedence in the tested order. Define `ProviderAvailability::{Configured, Environment, MissingKey, Unavailable}`; unauthenticated providers are `Configured`, supported keyed providers use the resolved origin, and providers with no supported models are `Unavailable`. Ensure all status/snapshot DTOs contain only provider ID and `CredentialOrigin`, never `SecretString`.

- [ ] **Step 4: Run credential and serialization tests**

Run: `cargo test -p xai-grok-catalog credentials::tests -- --nocapture`

Expected: PASS; a source scan `rg 'derive\([^)]*(Serialize|Deserialize)[^)]*\)' crates/codegen/xai-grok-catalog/src/credentials.rs` has no match on `SecretString`.

- [ ] **Step 5: Commit credential resolution**

```bash
git add crates/codegen/xai-grok-catalog
git commit -m "feat: add redacted provider credential resolution"
```

### Task 5: Implement 24-Hour Conditional Refresh And Secret-Free Atomic Cache

**Files:**
- Create: `crates/codegen/xai-grok-catalog/src/cache.rs`
- Create: `crates/codegen/xai-grok-catalog/src/refresh.rs`
- Create: `crates/codegen/xai-grok-catalog/tests/refresh.rs`
- Modify: `crates/codegen/xai-grok-catalog/src/lib.rs`
- Modify: `crates/codegen/xai-grok-catalog/Cargo.toml`

- [ ] **Step 1: Write failing cache fallback and conditional refresh tests**

```rust
#[tokio::test]
async fn stale_cache_sends_validators_and_304_renews_timestamp() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api.json"))
        .and(header("if-none-match", "catalog-v1"))
        .respond_with(ResponseTemplate::new(304)).mount(&server).await;
    let manager = manager_with_cache(&server, stale_cache("catalog-v1"));
    let before = manager.snapshot();
    assert_eq!(manager.refresh_if_stale().await.unwrap(), RefreshOutcome::NotModified);
    assert_eq!(manager.snapshot(), before);
    assert!(manager.cache_metadata().unwrap().fetched_at > stale_time());
}

#[tokio::test]
async fn invalid_response_keeps_last_known_good_snapshot_and_cache() {
    let manager = manager_with_valid_cache_and_response(br#"{"broken":true}"#);
    let before_snapshot = manager.snapshot();
    let before_bytes = std::fs::read(manager.cache_path()).unwrap();
    assert!(matches!(manager.refresh().await, Err(CatalogError::InvalidDocument(_))));
    assert_eq!(manager.snapshot(), before_snapshot);
    assert_eq!(std::fs::read(manager.cache_path()).unwrap(), before_bytes);
}

#[test]
fn cache_json_has_schema_source_timestamp_validators_and_no_secrets() {
    let bytes = encode_cache(&sample_cache()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for key in ["schema_version", "source_url", "fetched_at", "etag", "last_modified", "catalog"] { assert!(text.contains(key)); }
    assert!(!text.contains("api_key"));
    assert!(!text.contains("authorization"));
}
```

- [ ] **Step 2: Run the integration test and verify missing refresh/cache APIs**

Run: `cargo test -p xai-grok-catalog --test refresh -- --nocapture`

Expected: FAIL because `CatalogManager`, `CatalogCache`, and `RefreshOutcome` do not exist.

- [ ] **Step 3: Implement schema-versioned cache loading and immutable snapshots**

Define `CatalogCache { schema_version, source_url, fetched_at, etag, last_modified, catalog }`; reject unsupported schema/source and malformed catalogs. `CatalogSnapshot` wraps `Arc<NormalizedCatalog>` plus `RefreshStatus::{Fresh, Stale, Refreshing, CachedAfterError(String)}` and exposes these exact read APIs:

```rust
impl CatalogSnapshot {
    pub fn catalog(&self) -> &NormalizedCatalog;
    pub fn model(&self, provider_id: &ProviderId, model_id: &ModelId) -> Option<&CatalogModel>;
    pub fn bundled_model_by_exact_id(&self, id: &ModelId) -> Option<&CatalogModel>;
}
```

`bundled_model_by_exact_id` searches the embedded/runtime/override layers by complete model ID and never strips a slash prefix. `CatalogManager` stores `Arc<CatalogSnapshot>` in `ArcSwap`, exposes `snapshot() -> Arc<CatalogSnapshot>`, loads embedded first, overlays a valid cache regardless of age, and uses age only to decide refresh. Set `DEFAULT_REFRESH_INTERVAL` to exactly `Duration::from_secs(24 * 60 * 60)`.

- [ ] **Step 4: Implement bounded conditional HTTP refresh and atomic replacement**

Build a reqwest client with 2-second connect and 8-second total timeout, no credential headers, `redirect::Policy::limited(3)`, and streamed size enforcement at 8 MiB. Send `If-None-Match` and `If-Modified-Since`, accept only 200/304, normalize and validate a 200 completely before `ArcSwap::store`, then write `<cache>.tmp-<pid>` with `create_new(true)`, `sync_all`, rename, and parent directory sync on Unix. Delete a failed temp file. Keep prior memory/cache on every error and return concise status text (`"Using cached provider catalog; refresh failed"`) without embedding response bodies.

- [ ] **Step 5: Run refresh tests and crate tests**

Run: `cargo test -p xai-grok-catalog --test refresh -- --nocapture && cargo test -p xai-grok-catalog`

Expected: PASS, including 304 renewal, invalid response fallback, timeout fallback, oversized response rejection, and no-secret cache scan.

- [ ] **Step 6: Commit refresh/cache support**

```bash
git add crates/codegen/xai-grok-catalog Cargo.lock
git commit -m "feat: add provider catalog refresh cache"
```

### Task 6: Extend Auth Storage With Secure Provider Scopes

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/auth/model.rs`
- Modify: `crates/codegen/xai-grok-shell/src/auth/storage.rs`
- Modify: `crates/codegen/xai-grok-shell/src/auth/mod.rs`

- [ ] **Step 1: Write failing provider-key roundtrip and permission tests**

Add to `auth/storage.rs` tests:

```rust
#[test]
fn provider_key_roundtrip_preserves_xai_and_oauth_scopes() {
    let dir = tempfile::tempdir().unwrap();
    store_api_key(dir.path(), "xai-key").unwrap();
    store_provider_api_key(dir.path(), "openai", "openai-key").unwrap();
    assert_eq!(read_api_key(dir.path()).as_deref(), Some("xai-key"));
    assert_eq!(read_provider_api_key(dir.path(), "openai").as_deref(), Some("openai-key"));
    clear_provider_api_key(dir.path(), "openai").unwrap();
    assert_eq!(read_api_key(dir.path()).as_deref(), Some("xai-key"));
}

#[cfg(unix)]
#[test]
fn provider_key_file_is_created_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    store_provider_api_key(dir.path(), "anthropic", "secret").unwrap();
    assert_eq!(std::fs::metadata(dir.path().join("auth.json")).unwrap().permissions().mode() & 0o777, 0o600);
}
```

- [ ] **Step 2: Run focused auth tests and verify missing APIs**

Run: `cargo test -p xai-grok-shell auth::storage::provider_key -- --nocapture`

Expected: FAIL with unresolved `store_provider_api_key`, `read_provider_api_key`, and `clear_provider_api_key`.

- [ ] **Step 3: Implement validated provider scopes through the existing writer**

Add:

```rust
pub fn provider_api_key_scope(provider_id: &str) -> Result<String, &'static str> {
    if provider_id.is_empty() || !provider_id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_')) {
        return Err("provider id must contain only lowercase ASCII letters, digits, '-' or '_'");
    }
    Ok(format!("provider::{provider_id}"))
}
```

Implement read/store/clear using `read_auth_json`, `read_auth_json_or_empty_recovering_corrupt`, and the existing private `write_auth_json`; store `GrokAuth { auth_mode: AuthMode::ApiKey, key, ..Default::default() }`. Reject blank keys. Never log a key or serialize provider keys outside `auth.json`. Export these functions from `auth/mod.rs`.

- [ ] **Step 4: Run auth storage tests**

Run: `cargo test -p xai-grok-shell auth::storage -- --nocapture`

Expected: PASS; existing xAI storage, corrupt recovery, disk-full fallback, and provider-scope tests all pass.

- [ ] **Step 5: Commit provider auth scopes**

```bash
git add crates/codegen/xai-grok-shell/src/auth
git commit -m "feat: store provider API keys securely"
```

### Task 7: Add Provider Configuration And The Shell Catalog Adapter

**Files:**
- Create: `crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs`
- Modify: `crates/codegen/xai-grok-shell/Cargo.toml`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mod.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/config.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/config_model_override_parse.rs`

- [ ] **Step 1: Write failing config and conversion tests**

```rust
#[test]
fn parses_static_provider_override_without_discovery_fields() {
    let cfg = Config::new_from_toml_cfg(&toml::from_str(r#"
        [provider.openai]
        name = "OpenAI Team"
        base_url = "https://gateway.example/v1"
        env_key = ["OPENAI_API_KEY"]
    "#).unwrap()).unwrap();
    assert_eq!(cfg.config_providers["openai"].base_url.as_deref(), Some("https://gateway.example/v1"));
}

#[test]
fn adapter_uses_qualified_ids_and_never_copies_credentials() {
    let entry = model_entry_from_catalog(sample_provider(), sample_model()).unwrap();
    assert_eq!(entry.info.id.as_deref(), Some("openai/gpt-5"));
    assert_eq!(entry.provider_id.as_ref().map(ProviderId::as_str), Some("openai"));
    assert_eq!(entry.credential_policy, CredentialPolicy::ProviderApiKey);
    assert!(entry.api_key.is_none());
    assert!(entry.env_key.is_none());
}
```

- [ ] **Step 2: Run focused tests and verify missing config/adapter types**

Run: `cargo test -p xai-grok-shell provider_catalog -- --nocapture`

Expected: FAIL with missing `config_providers`, `CredentialPolicy`, and adapter functions.

- [ ] **Step 3: Add static provider configuration and neutral-to-shell conversion**

Define `ConfigProviderOverride { name, base_url, env_key: Option<EnvKeys>, unauthenticated: Option<bool> }` under `[provider.<id>]`. The shell adapter maps `base_url` onto neutral `CatalogProvider.api_base_url`; Plan 3 extends this same `base_url` field rather than introducing a second URL key. Do not add `discovery`, model-list endpoint, health endpoint, insecure HTTP opt-in, or Janus fields; those belong to Plan 3. Add to `ModelEntry`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub provider_id: Option<xai_grok_catalog::ProviderId>,
#[serde(default)]
pub credential_policy: CredentialPolicy,
```

with `provider_id` typed as `Option<xai_grok_catalog::ProviderId>`, and `CredentialPolicy::{XaiLegacy, ProviderApiKey, ExplicitModel, None}` with default `XaiLegacy` to preserve all current entries. Convert catalog `Protocol` to current shell `ApiBackend::{ChatCompletions, Responses, Messages}`, reject `Protocol::Unsupported`, use qualified `ModelInfo.id` `provider/model`, route slug `CatalogModel.id`, provider base URL, catalog name/context/reasoning, and no secret fields. Apply user provider fields after embedded/runtime/overrides and explicit `[model.<qualified-id>]` via existing `ConfigModelOverride::apply` last.

Define the shell adapter contract that Plan 3 extends:

```rust
pub(crate) struct ProviderCatalogAdapter {
    manager: xai_grok_catalog::CatalogManager,
    // Auth-store/config handles contain no secrets in catalog snapshots.
}

impl ProviderCatalogAdapter {
    pub(crate) fn snapshot(&self) -> std::sync::Arc<xai_grok_catalog::CatalogSnapshot>;
    pub(crate) fn publish(&self, snapshot: std::sync::Arc<xai_grok_catalog::CatalogSnapshot>);
    pub(crate) fn credential_for(
        &self,
        provider_id: &xai_grok_catalog::ProviderId,
    ) -> Option<xai_grok_catalog::SecretString>;
}
```

`snapshot` and `publish` use the `CatalogManager`'s `ArcSwap`; `credential_for` performs Plan 2's session, stored, environment, then model resolution at call time and never stores the returned secret in the adapter or snapshot.

- [ ] **Step 4: Add a dependency-cycle guard**

Run: `cargo tree -p xai-grok-catalog --edges normal | rg 'xai-grok-(shell|pager|sampler|auth|sampling-types)|agent-client-protocol'`

Expected: no output. Run `cargo tree -p xai-grok-shell --invert xai-grok-catalog`; expected output contains only consumers above the catalog crate, never a path back to shell.

- [ ] **Step 5: Run shell config/adapter tests**

Run: `cargo test -p xai-grok-shell agent::config_model_override_parse -- --nocapture && cargo test -p xai-grok-shell provider_catalog -- --nocapture`

Expected: PASS; existing `[model.*]` parsing remains green and catalog conversion emits qualified IDs.

- [ ] **Step 6: Commit the shell adapter**

```bash
git add Cargo.toml Cargo.lock crates/codegen/xai-grok-shell/Cargo.toml crates/codegen/xai-grok-shell/src/agent
git commit -m "feat: adapt provider catalog into shell models"
```

### Task 8: Enforce Credential Precedence And Block Cross-Provider xAI Leakage

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/agent/provider_catalog.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/config.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/models.rs`

- [ ] **Step 1: Write failing credential matrix and leakage regression tests**

```rust
#[test]
fn provider_credential_precedence_matches_design() {
    let sources = test_sources("cli", "stored", "env", "model");
    assert_origin(resolve_provider_credentials(&provider_entry(), sources), CredentialOrigin::Session);
    assert_origin(resolve_provider_credentials(&provider_entry(), test_sources("", "stored", "env", "model")), CredentialOrigin::Stored);
    assert_origin(resolve_provider_credentials(&provider_entry(), test_sources("", "", "env", "model")), CredentialOrigin::Environment);
    assert_origin(resolve_provider_credentials(&provider_entry(), test_sources("", "", "", "model")), CredentialOrigin::Model);
}

#[test]
fn xai_session_and_global_key_never_leak_to_openai() {
    let entry = provider_entry_for("openai", "https://api.openai.com/v1");
    let resolved = resolve_credentials_with(&entry, Some("xai-oauth-token"), || Some("xai-global-key"), |_| None, |_| None);
    assert!(resolved.api_key.is_none());
    assert_eq!(resolved.base_url, "https://api.openai.com/v1");
}

#[test]
fn xai_legacy_resolution_is_unchanged() {
    let entry = ModelEntry::fallback("grok-4", &EndpointsConfig::default());
    let resolved = resolve_credentials_with(&entry, Some("xai-oauth-token"), || Some("xai-global-key"), |_| None, |_| None);
    assert_eq!(resolved.api_key.as_deref(), Some("xai-oauth-token"));
    assert_eq!(resolved.auth_type, xai_chat_state::AuthType::SessionToken);
}
```

- [ ] **Step 2: Run the regression tests and observe leakage**

Run: `cargo test -p xai-grok-shell xai_session_and_global_key_never_leak_to_openai -- --nocapture`

Expected: FAIL because current `resolve_credentials` falls through from missing model credentials to the session token or `XAI_API_KEY` for every `ModelEntry`.

- [ ] **Step 3: Implement policy-specific credential resolution**

Extract injectable `resolve_credentials_with`. For `XaiLegacy`, retain byte-for-byte current order and endpoint behavior. For `ProviderApiKey`, resolve current-session provider key, `read_provider_api_key(grok_home, provider_id)`, catalog/provider env vars, then existing model `api_key`/`env_key`; never inspect `session_key` or `XAI_API_KEY`. For `ExplicitModel`, retain model `api_key`/`env_key` only and do not use xAI fallbacks when `provider_id` is non-xAI. For `None`, return no key. Keep only credential origin in debug logs. Call this seam from `ModelsManager::sampling_config`.

- [ ] **Step 4: Run the full credential suite**

Run: `cargo test -p xai-grok-shell resolve_credentials -- --nocapture && cargo test -p xai-grok-shell provider_catalog -- --nocapture`

Expected: PASS, including all pre-existing xAI precedence tests and the non-xAI leakage regression.

- [ ] **Step 5: Commit the credential firewall**

```bash
git add crates/codegen/xai-grok-shell/src/agent
git commit -m "fix: isolate provider credentials from xAI tokens"
```

### Task 9: Compose Configured Provider Models Into ModelsManager And ACP Metadata

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/agent/models.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/config.rs`
- Modify: `crates/codegen/xai-grok-shell/src/leader/server.rs`

- [ ] **Step 1: Write failing configured-filter and ACP metadata tests**

```rust
#[test]
fn only_configured_provider_models_are_available() {
    let catalog = sample_catalog(["openai", "anthropic"]);
    let resolver = fake_credentials().stored("openai", "key");
    let models = provider_model_entries(&catalog, &Config::default(), &resolver);
    assert!(models.contains_key("openai/gpt-5"));
    assert!(!models.contains_key("anthropic/claude-sonnet-4"));
}

#[test]
fn acp_metadata_contains_provider_context_reasoning_and_cost_but_no_secret() {
    let info = to_acp_model_info(&one_provider_model()).remove("openai/gpt-5").unwrap();
    let meta = info.meta.unwrap();
    assert_eq!(meta["providerId"], "openai");
    assert_eq!(meta["providerName"], "OpenAI");
    assert_eq!(meta["totalContextTokens"], 400_000);
    assert_eq!(meta["supportsReasoningEffort"], true);
    assert_eq!(meta["inputCostPerMillion"], 1.25);
    assert!(!serde_json::to_string(&meta).unwrap().contains("apiKey"));
}

#[test]
fn provider_refresh_preserves_a_still_valid_current_selection() {
    let manager = manager_on("openai/gpt-5");
    manager.apply_provider_snapshot(snapshot_with(["openai/gpt-5", "openai/gpt-5-mini"]));
    assert_eq!(manager.current_model_id().0.as_ref(), "openai/gpt-5");
}
```

- [ ] **Step 2: Run focused model tests and verify provider models are absent**

Run: `cargo test -p xai-grok-shell agent::models::tests::only_configured_provider_models_are_available -- --nocapture`

Expected: FAIL because `ModelsManager` currently composes only bundled/prefetched xAI models and `[model.*]`.

- [ ] **Step 3: Compose snapshots without replacing the xAI pipeline**

Add `catalog_manager: xai_grok_catalog::CatalogManager` and provider-session credentials to `ModelsManager::Inner`. In `resolve_model_catalog`, first call existing `resolve_model_list(cfg, prefetched)` unchanged for xAI, then append catalog provider entries whose status is `Configured` or `Environment`, then apply explicit qualified model overrides. Preserve xAI keys/default ordering and xAI entitlement cache (`models_cache.json`, five-minute TTL, `FetchModelsResult`) exactly. Store provider catalog separately at `~/.grok/provider_catalog.json` with 24-hour freshness.

- [ ] **Step 4: Emit provider-aware ACP model metadata and updates**

Extend `to_acp_model_info` with `providerId`, `providerName`, `supportsReasoning`, `inputCostPerMillion`, and `outputCostPerMillion` when present. Never include credential origin, environment variable value, key, or authorization header. Continue using existing `x.ai/models/update`; add `x.ai/providers/update` to the `leader/server.rs` forwarding allowlist in Task 10.

- [ ] **Step 5: Run ModelsManager and ACP tests**

Run: `cargo test -p xai-grok-shell agent::models -- --nocapture && cargo test -p xai-grok-shell to_acp_model_info -- --nocapture`

Expected: PASS; xAI default tests remain unchanged, only configured providers are exposed, and current selection survives a replacement containing its ID.

- [ ] **Step 6: Commit provider model composition**

```bash
git add crates/codegen/xai-grok-shell/src/agent crates/codegen/xai-grok-shell/src/leader/server.rs
git commit -m "feat: expose configured provider models"
```

### Task 10: Add Provider ACP Surface And Background Refresh Triggers

**Files:**
- Create: `crates/codegen/xai-grok-shell/src/extensions/providers.rs`
- Modify: `crates/codegen/xai-grok-shell/src/extensions/mod.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/slash_commands.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/models.rs`
- Modify: `crates/codegen/xai-grok-shell/src/leader/server.rs`

- [ ] **Step 1: Write failing extension response and refresh tests**

```rust
#[tokio::test]
async fn provider_list_is_secret_free_and_reports_all_states() {
    let response = list_providers(&test_agent()).await.unwrap();
    assert_eq!(response.providers[0].status, ProviderAvailability::Configured);
    let json = serde_json::to_string(&response).unwrap();
    assert!(!json.contains("sk-"));
    assert!(!json.contains("api_key"));
}

#[tokio::test]
async fn store_key_rebuilds_models_and_broadcasts_both_updates() {
    let agent = test_agent_missing_openai();
    store_provider_key(&agent, StoreProviderKeyRequest { provider_id: "openai".into(), api_key: "secret".into() }).await.unwrap();
    assert!(agent.models_manager.available().keys().any(|id| id.0.as_ref() == "openai/gpt-5"));
    assert_eq!(agent.gateway.methods(), ["x.ai/providers/update", "x.ai/models/update"]);
}

#[tokio::test]
async fn opening_surface_returns_immediately_and_starts_one_stale_refresh() {
    let agent = test_agent_with_stale_catalog();
    let response = list_providers(&agent).await.unwrap();
    assert!(response.refresh_started);
    assert_eq!(agent.refresh_spawn_count(), 1);
}
```

- [ ] **Step 2: Run extension tests and verify routes are absent**

Run: `cargo test -p xai-grok-shell extensions::providers -- --nocapture`

Expected: FAIL because provider extension module/routes do not exist.

- [ ] **Step 3: Implement typed provider methods and notifications**

Implement methods:

```text
x.ai/providers/list       {} -> { providers, refreshStatus, refreshStarted }
x.ai/providers/store_key  { providerId, apiKey } -> { providerId, status }
x.ai/providers/clear_key  { providerId } -> { providerId, status }
x.ai/providers/refresh    {} -> { started }
```

`list` returns embedded/current state before spawning refresh. `store_key` rejects blank/unknown/unavailable providers, writes securely, rebuilds model availability, and broadcasts provider then model updates. `clear_key` removes only that provider scope. `refresh` coalesces with an `AtomicBool`, performs no model discovery, and on completion broadcasts immutable replacement snapshots. Return concise typed errors; detailed causes go only to local tracing and must not include request bodies/keys.

- [ ] **Step 4: Advertise provider commands and preserve xAI OAuth**

Add `/providers` to shell `BUILTIN_COMMANDS` with alias `login`, always-on gate, and a provider-management description. This advertisement is metadata only; pager handles the UI. Keep ACP auth methods, `Action::Login`, `dispatch_login`, xAI auth scopes, and xAI default resolution unchanged.

- [ ] **Step 5: Run shell provider and routing tests**

Run: `cargo test -p xai-grok-shell extensions::providers -- --nocapture && cargo test -p xai-grok-shell session::slash_commands -- --nocapture && cargo test -p xai-grok-shell leader::server -- --nocapture`

Expected: PASS; routes accept typed payloads, updates forward to clients, and `/providers` plus `/login` resolve to the same surface.

- [ ] **Step 6: Commit provider ACP management**

```bash
git add crates/codegen/xai-grok-shell/src/extensions crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs crates/codegen/xai-grok-shell/src/session/slash_commands.rs crates/codegen/xai-grok-shell/src/agent/models.rs crates/codegen/xai-grok-shell/src/leader/server.rs
git commit -m "feat: add provider management ACP surface"
```

### Task 11: Build `/providers`, `/login` Alias, And Masked Key Entry In Pager

**Files:**
- Create: `crates/codegen/xai-grok-pager/src/providers.rs`
- Create: `crates/codegen/xai-grok-pager/src/slash/commands/providers.rs`
- Create: `crates/codegen/xai-grok-pager/src/views/providers_modal.rs`
- Delete: `crates/codegen/xai-grok-pager/src/slash/commands/login.rs`
- Modify: `crates/codegen/xai-grok-pager/src/slash/commands/mod.rs`
- Modify: `crates/codegen/xai-grok-pager/src/views/modal.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/actions.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/dispatch/router.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/effects/mod.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/acp_handler/mod.rs`

- [ ] **Step 1: Write failing command alias and provider-state tests**

```rust
#[test]
fn providers_command_owns_login_alias() {
    let cmd = ProvidersCommand;
    assert_eq!(cmd.name(), "providers");
    assert_eq!(cmd.aliases(), &["login"]);
    assert!(matches!(cmd.run(&mut exec_ctx(), ""), CommandResult::Action(Action::OpenProviders)));
}

#[test]
fn modal_rows_show_all_four_states_without_secrets() {
    let rows = provider_rows(sample_provider_state());
    assert_eq!(rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(), ["Configured", "Environment", "Missing key", "Unavailable"]);
    assert!(!format!("{rows:?}").contains("secret"));
}

#[test]
fn key_input_is_masked_and_task_result_discards_plaintext() {
    let mut modal = ProvidersModalState::entering_key("openai");
    modal.insert_str("sk-openai-secret");
    assert_eq!(modal.rendered_key(), "****************");
    let action = modal.submit().unwrap();
    assert!(matches!(action, Action::StoreProviderKey { provider_id, .. } if provider_id == "openai"));
}
```

- [ ] **Step 2: Run pager provider tests and verify types are absent**

Run: `cargo test -p xai-grok-pager providers -- --nocapture`

Expected: FAIL because `ProvidersCommand`, provider state, modal, and actions do not exist.

- [ ] **Step 3: Implement provider state, command, and modal**

`/providers` and `/login` both dispatch `Action::OpenProviders`, whose effect calls `x.ai/providers/list`. Render provider name/ID/status and concise refresh status. Selecting xAI dispatches the unchanged `Action::Login` OAuth/default flow. Selecting a missing API-key provider opens a masked textarea; submission moves the key directly into `Effect::StoreProviderKey` and immediately clears the modal buffer. Selecting configured/environment providers offers replace/clear as applicable. Unavailable rows are disabled and explain `Unsupported protocol or authentication`.

- [ ] **Step 4: Wire typed effects and secret-free task results**

Add `Effect::{ListProviders, StoreProviderKey, ClearProviderKey, RefreshProviders}` and task results that contain only `ProviderListResponse`, provider ID, status, or sanitized error. Construct ACP raw JSON only at effect execution. Do not derive `Debug` for an effect variant containing the key; provide a manual redacted `Debug` arm. On update, refresh provider modal rows and model state; never copy key text into toast/history/scrollback.

- [ ] **Step 5: Remove OAuth-only command registration and run pager tests**

Delete `login.rs`; register `ProvidersCommand` once. Keep all other `Action::Login` producers and dispatch tests because xAI selection still invokes them.

Run: `cargo test -p xai-grok-pager providers -- --nocapture && cargo test -p xai-grok-pager app::dispatch::tests::auth -- --nocapture && cargo test -p xai-grok-pager slash::commands -- --nocapture`

Expected: PASS; `/login` opens providers, xAI OAuth tests remain green, and key values never appear in snapshots/debug assertions.

- [ ] **Step 6: Commit the provider picker**

```bash
git add crates/codegen/xai-grok-pager/src
git commit -m "feat: add provider management picker"
```

### Task 12: Add Provider-Aware Fuzzy `/model` Picker And End-To-End Verification

**Files:**
- Modify: `crates/codegen/xai-grok-pager/Cargo.toml`
- Modify: `crates/codegen/xai-grok-pager/src/slash/command.rs`
- Modify: `crates/codegen/xai-grok-pager/src/slash/commands/model.rs`
- Modify: `crates/codegen/xai-grok-pager/src/views/picker.rs`
- Modify: `crates/codegen/xai-grok-pager/src/acp/model_state.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/acp_handler/settings.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/acp_handler/tests/models.rs`

- [ ] **Step 1: Write failing ranking, display, and background-update tests**

```rust
#[test]
fn empty_query_orders_current_then_provider_and_model_id() {
    let items = build_model_items(&provider_model_state("anthropic/claude-sonnet-4"));
    assert_eq!(items.iter().map(|i| i.insert_text.trim()).collect::<Vec<_>>(), ["Claude Sonnet 4", "GPT-5", "GPT-5 Mini"]);
}

#[test]
fn direct_provider_model_match_ranks_above_display_name_proxy() {
    let state = ranking_fixture();
    let items = scored_model_items(&state, "openai/gpt-5");
    assert_eq!(items[0].model_id.as_deref(), Some("openai/gpt-5"));
    assert!(items[0].explicit_score > items[1].explicit_score);
}

#[test]
fn row_contains_provider_context_and_reasoning() {
    let row = build_model_items(&provider_model_state("openai/gpt-5")).remove(0);
    assert!(row.display.contains("OpenAI"));
    assert!(row.description.contains("400k context"));
    assert!(row.description.contains("reasoning"));
}

#[test]
fn background_update_keeps_current_and_reports_cached_notice() {
    let mut state = provider_model_state("openai/gpt-5");
    apply_models_update(&mut state, update_with_status("openai/gpt-5", "Using cached provider catalog; refresh failed"));
    assert_eq!(state.current_model_id_str(), Some("openai/gpt-5"));
    assert_eq!(state.catalog_notice.as_deref(), Some("Using cached provider catalog; refresh failed"));
}
```

- [ ] **Step 2: Run model picker tests and verify current ordering/ranking fails**

Run: `cargo test -p xai-grok-pager slash::commands::model -- --nocapture && cargo test -p xai-grok-pager app::acp_handler::tests::models -- --nocapture`

Expected: FAIL because rows only index/display model names, ACP provider metadata is not parsed, and the picker has no explicit direct-match score or catalog notice.

- [ ] **Step 3: Parse provider metadata and build provider-aware rows**

Add `ModelPickerMetadata { provider_id, provider_name, context_window, reasoning }` accessors over ACP meta. Build each row with primary identity `Model Name  Provider Name`, description `provider/model · 400k context · reasoning`, insertion text unchanged as the model display name, and match text containing newline-separated provider ID, `provider/model`, model ID, and display name. Do not show costs in the row; retain costs in ACP metadata for future detail surfaces.

- [ ] **Step 4: Implement deterministic weighted fuzzy ranking**

Add `explicit_score: Option<i64>` and `model_id: Option<String>` to `ArgItem`; all non-model commands set both to `None`. For model query scoring, normalize ASCII lowercase and apply:

```rust
let direct = if qualified == query { 1_000_000 }
    else if qualified.starts_with(query) { 900_000 }
    else if model_id == query { 800_000 }
    else if provider_id == query { 700_000 }
    else { 0 };
let fuzzy = SkimMatcherV2::default().fuzzy_match(&match_text, query).unwrap_or(i64::MIN / 4);
let explicit_score = direct.saturating_add(fuzzy);
```

In `picker.rs`, sort explicit scores descending before the existing matcher/stable tie-break. With an empty query, bypass scores: current ID first, then `(provider_id, model_id)` ascending. Preserve reasoning effort phase detection and existing `Action::SetDefaultModel`, so selection continues through the current model-settings persistence path.

- [ ] **Step 5: Trigger stale refresh without delaying picker display**

When `/model` opens, render current `ModelState.available` immediately and dispatch one coalesced `RefreshProviders` effect. `x.ai/models/update` replaces available models, calls existing `ModelState::update_catalog`, and leaves current/effort untouched when current remains present. Show `Fresh`, `Refreshing provider catalog`, or the exact cached notice in the picker footer/toast; never close/reopen the picker or move selection during an in-flight refresh.

- [ ] **Step 6: Run pager model/UI tests**

Run: `cargo test -p xai-grok-pager slash::commands::model -- --nocapture && cargo test -p xai-grok-pager app::acp_handler::tests::models -- --nocapture && cargo test -p xai-grok-pager views::picker -- --nocapture`

Expected: PASS; direct qualified match wins, current-first ordering is stable, update preserves current selection, and refresh failure retains rows with the cached notice.

- [ ] **Step 7: Run security and scope scans**

Run:

```bash
rg 'GET /models|/v1/models|janus|sk-janus' crates/codegen/xai-grok-catalog crates/codegen/xai-grok-shell/src/extensions/providers.rs crates/codegen/xai-grok-pager/src/providers.rs
rg 'api_key|authorization|SecretString' crates/codegen/xai-grok-catalog/data/models-dev.json crates/codegen/xai-grok-catalog/data/overrides.json
cargo tree -p xai-grok-catalog --edges normal | rg 'xai-grok-(shell|pager|sampler|auth|sampling-types)|agent-client-protocol'
```

Expected: all three commands produce no output. This proves Plan 2 did not add generic dynamic `GET /models`, Janus, secrets in committed catalog data, or a dependency cycle.

- [ ] **Step 8: Run workspace verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p xai-grok-catalog
cargo test -p xai-grok-shell agent::models
cargo test -p xai-grok-shell auth::storage
cargo test -p xai-grok-shell extensions::providers
cargo test -p xai-grok-pager providers
cargo test -p xai-grok-pager slash::commands::model
cargo clippy -p xai-grok-catalog -p xai-grok-shell -p xai-grok-pager --all-targets -- -D warnings
cargo test --workspace
```

Expected: every command exits 0. Confirm existing xAI OAuth tests, xAI API-key tests, default-model tests, model selection persistence tests, provider cache fallback tests, and redaction/leakage tests are included in the passing output.

- [ ] **Step 9: Commit provider-aware model picking**

```bash
git add Cargo.toml Cargo.lock crates/codegen/xai-grok-pager
git commit -m "feat: add provider-aware model picker"
```

## Execution Constraints

- Execute only after the telemetry-removal plan is merged; use its maintained root `Cargo.toml` state and do not reintroduce removed telemetry dependencies or call sites.
- Do not implement generic dynamic provider `GET /models`, endpoint derivation, redirect credential handling, non-loopback HTTP opt-in, health checks, or Janus preset/setup. Those are Plan 3.
- Keep the existing xAI `ModelsManager` entitlement fetch/cache behavior, OAuth ACP auth methods, `API_KEY_SCOPE`, `/login` xAI action, and default-model fallback intact; only the command entry point becomes the provider surface alias.
- Never place credentials in `CatalogProvider`, `CatalogModel`, `NormalizedCatalog`, `CatalogSnapshot`, `CatalogCache`, `ModelInfo`, ACP model metadata, provider responses, picker rows, logs, errors, or debug output.
- Every non-xAI `ModelEntry` must carry `CredentialPolicy::ProviderApiKey` or `ExplicitModel`; default/fallback `XaiLegacy` is permitted only for existing xAI models. The cross-provider leakage regression test is a release blocker.
- Keep the crate graph one-way: `xai-grok-catalog -> serde/indexmap/reqwest/tokio/arc-swap/chrono/url`; `xai-grok-shell -> xai-grok-catalog`; `xai-grok-pager -> xai-grok-shell`. Conversion to current `ModelEntry`, `ApiBackend`, and ACP types belongs only in shell.
