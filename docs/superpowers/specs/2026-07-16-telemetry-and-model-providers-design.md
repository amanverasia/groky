# Telemetry Removal and Multi-Provider Model Support

## Summary

Groky will become a maintained offering alongside Grok Build with two related changes:

1. Remove all networked telemetry, error reporting, and trace upload while retaining useful local logs and instrumentation.
2. Add a hybrid provider and model catalog, provider credential management, a configured-provider model picker, generic OpenAI-compatible model discovery, and a first-class Janus preset.

The provider design reuses groky's existing model entries, API backends, sampler, and picker infrastructure. A new catalog crate translates provider-level catalog data into the existing per-model representation rather than replacing the inference stack.

## Goals

- Ensure groky sends no analytics, product events, crash reports, OpenTelemetry data, or trace uploads.
- Preserve local structured logs, timing data, crash formatting, and other on-device diagnostics.
- Ship an offline-capable provider and model catalog generated from models.dev.
- Refresh catalog data at runtime so users can access new models between groky releases.
- Show models from configured providers in a single fuzzy-searchable picker.
- Support API-key authentication through secure local storage and provider environment variables.
- Discover models from generic OpenAI-compatible gateways.
- Provide a first-class setup flow for Janus without coupling the core implementation to Janus.

## Non-Goals

- Adding OAuth flows for Anthropic, OpenAI, GitHub Copilot, or other providers in v1.
- Adding provider plugins or arbitrary executable credential resolvers in v1.
- Adding native operating-system keychain integration in v1.
- Replacing groky's existing sampler or its `chat_completions`, `responses`, and `messages` API backends.
- Showing unconfigured providers' models in the model picker.
- Removing update checks or announcement fetching, which are product features rather than telemetry.

## Architecture

### Catalog Boundary

Add a focused `xai-grok-catalog` crate. It owns:

- Provider and model catalog types.
- Parsing and normalizing models.dev data.
- The embedded catalog snapshot.
- Provider and model override data.
- Runtime catalog refresh and disk caching.
- Generic provider model discovery.
- The Janus preset.
- Provider credential lookup from the dedicated auth store and environment.
- Conversion of available provider models into existing `ModelEntry` values.

The catalog crate must not send inference requests or own conversation state. Existing sampling crates continue to implement the wire protocols. Existing user-defined `[model.<name>]` entries remain supported.

The catalog's public interface exposes provider metadata and an immutable available-model snapshot. Consumers do not need to know whether an entry came from the embedded catalog, runtime overlay, dynamic discovery, or explicit configuration.

### Merge Precedence

Catalog data is layered from lowest to highest precedence:

1. Embedded generated catalog.
2. Valid runtime catalog overlay.
3. Hand-maintained provider and model corrections.
4. Dynamically discovered provider models.
5. Explicit user provider/model configuration.

Entries merge by provider ID and model ID. Higher layers replace only fields they explicitly supply. Explicit local configuration always wins. Failed refreshes never delete entries from a previously valid snapshot.

## Telemetry Removal

### Remove

- Delete the `xai-mixpanel` crate and remove it from the Cargo workspace.
- Remove product-event HTTP emission and Mixpanel emission.
- Remove Sentry initialization, dependencies, DSN configuration, and reporting.
- Remove external OpenTelemetry export, OTLP HTTP export, exporter configuration, and related environment handling.
- Remove trace upload to GCS, including its configuration gate and URL resolution.
- Remove telemetry configuration for event URLs, API keys, Mixpanel, trace uploads, and external exporters.

The root `Cargo.toml` currently identifies itself as generated. This maintained fork will update it to remove deleted workspace members and dependencies and will treat the workspace manifest as maintained source unless a replacement generation workflow is introduced later.

### Retain

- Local unified, debug, memory, hook, and sampling logs.
- Prompt timing and local instrumentation.
- Local tracing provided by `xai-tracing`, with all network exporters removed.
- Local crash handling and formatting, after verifying no upload path remains.
- Existing telemetry event call sites where removing them would create broad churn.

The `telemetry_event!` and `track()` interfaces remain temporarily as no-ops to avoid broad consumer churn; they do not copy event payloads into local logs. `TelemetryMode` remains as a compatibility type with `Disabled` as its only supported value. There must be no mode that re-enables remote emission, no persisted analytics queue, and no collection for later upload.

Update checks and announcement fetching remain unchanged. Their network traffic must remain distinguishable from analytics and must not include session or usage data.

## Catalog Generation

A deterministic repository script fetches `https://models.dev/api.json`, validates it, normalizes supported fields, applies stable filtering, and writes a committed JSON snapshot embedded in the binary.

The generated snapshot contains, where available:

- Provider ID and display name.
- Provider API base URL.
- Provider API-key environment variable names.
- Model ID and display name.
- API protocol/backend.
- Context-window size.
- Reasoning capability.
- Input and output token costs.

Provider-specific fixes belong in a small, reviewed overrides file rather than in generated output. Overrides may correct URLs, protocols, metadata, or compatibility properties and may exclude known-incompatible entries.

CI reruns generation or an equivalent deterministic check. It fails when the committed snapshot does not match generator output, making catalog updates an explicit maintenance action.

## Runtime Catalog Refresh

Groky loads the embedded catalog first and then merges the last valid runtime cache. Catalog availability must never block startup.

Opening `/model` or `/providers` starts a background refresh when the cache is stale. The default freshness interval is 24 hours. Refresh uses a short timeout, a response-size limit, and conditional requests through `ETag` or `Last-Modified` when the server supports them.

The runtime source initially defaults to models.dev. Its URL is configurable so a future groky-operated curated endpoint can become the source without changing the picker or catalog consumers.

Remote data is normalized and fully validated before replacing the active overlay. An invalid response is rejected as a whole. Groky continues using the last valid cache, or the embedded catalog if no cache exists.

Cache writes use a temporary file and atomic rename. Cache metadata includes:

- Catalog schema version.
- Source URL.
- Fetch timestamp.
- HTTP validators.
- Normalized payload.

Unknown fields are ignored. An unsupported schema version causes fallback to the embedded snapshot. Refresh failures are recorded only in local debug logs and as concise UI status messages.

## Provider Authentication

Add `/providers` as the primary provider-management surface, with `/login` as an alias. It lists all catalog providers and shows one of these states:

- `Configured`: a stored credential exists or the provider allows unauthenticated use.
- `Environment`: a supported provider environment variable resolves to a credential.
- `Missing key`: the provider is supported but has no usable credential.
- `Unavailable`: the provider requires an unsupported protocol or authentication method.

Selecting an API-key provider prompts for a key and stores it in `~/.grok/auth.json`. The file must be created with mode `0600`; restricted permissions must apply from file creation rather than through a later chmod. Credentials must not be serialized into catalog caches, model snapshots, logs, errors, or debug formatting.

Credential resolution order is:

1. Explicit CLI or current-session credential.
2. Credential stored in `~/.grok/auth.json`.
3. Provider-specific environment variable declared by the catalog.
4. Existing per-model `api_key` or `env_key` configuration.

A higher-precedence credential owns resolution for that provider. Stored credentials intentionally take precedence over environment variables. Existing xAI OAuth behavior remains intact and separate from the new API-key-only provider flow.

## Model Picker

`/model` and Ctrl+M show a flat list of models from configured providers only. A provider is configured when its required credentials resolve or when it explicitly permits unauthenticated access.

Each row shows the model name, provider, context window, and reasoning capability without overwhelming the primary model/provider identity. Search indexes:

- Provider ID.
- `provider/model`.
- Model ID.
- Model display name.

Direct provider/model matches should rank above proxy matches. With no query, the current model appears first and remaining models are ordered consistently by provider and model ID.

The picker opens immediately from the current immutable snapshot. If data is stale, refresh occurs in the background and replaces the snapshot without changing the current selection. A refresh failure keeps the previous list and displays a concise cached-data notice. Selection persists through groky's existing model-settings path.

## Generic Dynamic Providers

Extend custom provider configuration with optional model discovery. An OpenAI-compatible dynamic provider supplies:

- Provider ID and display name.
- API base URL.
- API backend, defaulting to `chat_completions`.
- Credential source or an explicit unauthenticated setting.
- A discovery flag or model-list endpoint override.
- Optional static model definitions and metadata overrides.

By default, discovery requests `GET /models` relative to the normalized API base URL. For a base URL ending in `/v1`, this produces `/v1/models`. It sends the provider bearer credential when present and accepts the OpenAI model-list response format.

Static and discovered models merge by model ID, with explicit local entries winning. Discovery is optional, so existing custom-model configuration remains valid without changes. Failed discovery retains explicit and previously cached entries.

Discovery enforces response-size, model-count, and field-length limits. Authorization headers must not be forwarded by redirects to a different origin. Plain HTTP is allowed by default only for loopback endpoints; non-loopback plain HTTP requires explicit user opt-in.

## Janus Integration

Janus is implemented as a first-class preset over the generic dynamic-provider mechanism.

Preset defaults:

- Provider ID: `janus`.
- Display name: `Janus`.
- Base URL: `http://127.0.0.1:20128/v1`.
- API backend: `chat_completions`.
- Health endpoint: `GET /v1/health`.
- Discovery endpoint: `GET /v1/models`.
- Authentication: optional bearer API key, with an `sk-janus-` input hint.

The setup flow is:

1. Select Janus in `/providers`.
2. Enter or confirm the base URL.
3. Optionally enter an API key.
4. Check the health endpoint.
5. Fetch the model list.
6. Add discovered models to the available snapshot.

The base URL can be overridden for Docker, LAN, Tailscale, or reverse-proxy deployments. Janus model IDs remain unchanged, including upstream-style names such as `openai/gpt-4o` and combo names such as `best-effort`. All requests still target the configured Janus base URL.

When an exact discovered model ID matches bundled catalog metadata, groky enriches missing context-window, reasoning, and cost data from that entry. Unknown models and combos use conservative defaults and remain selectable. Users may explicitly override protocol or metadata for nonstandard deployments.

Janus-specific failures are actionable:

- Connection failure reports the attempted URL and suggests checking the Janus service.
- HTTP 401 or 403 directs the user to update the Janus key through `/providers`.
- An empty model list is accepted but clearly reported.
- Cached models remain visible while Janus is offline, although inference reports the connection failure.

Credentials must never appear in health-check, discovery, or inference errors or local logs.

## Data Flow

### Startup

1. Load the embedded catalog.
2. Merge the last valid runtime catalog cache.
3. Load user provider and model configuration.
4. Resolve provider availability without remote requests.
5. Build an immutable model snapshot for configured providers.
6. Preserve the current xAI default when no additional provider is configured.

### Background Refresh

1. `/model` or `/providers` renders from the current snapshot.
2. Stale catalog and dynamic-provider refreshes run asynchronously with timeouts and bounded concurrency.
3. Responses are validated and normalized before becoming visible.
4. Valid results are atomically cached and merged into a replacement snapshot.
5. The UI receives the replacement snapshot without changing the user's selection.
6. Failures retain the existing snapshot and produce a concise status plus detailed local diagnostics.

### Inference

1. The selected catalog model has already been converted into an existing `ModelEntry`.
2. Credential resolution supplies the provider credential without adding it to the model cache.
3. Existing groky configuration selects `chat_completions`, `responses`, or `messages`.
4. The existing sampler sends and streams the request.

Models requiring unsupported wire protocols are excluded from availability and explained in local diagnostics.

## Errors And Security

Catalog and provider errors use typed categories:

- Provider unavailable.
- Authentication failed.
- Invalid catalog or discovery response.
- Unsupported protocol.
- Connection or timeout failure.
- Cache read/write failure.

User-facing errors must be concise and actionable. Detailed causes belong in local debug logs. Secrets are represented by redacting wrapper types whose debug and display implementations cannot expose credential values.

Catalog and model cache structures do not contain secrets. Remote requests have bounded time, response size, and concurrency. Catalog failures cannot block startup, invalidate the last known-good snapshot, or trigger telemetry.

## Testing Strategy

### Unit Tests

- Catalog parsing and normalization.
- Layered merge precedence and partial-field replacement.
- Schema-version rejection and fallback.
- Models.dev-to-`ModelEntry` protocol mapping.
- Credential precedence and provider availability.
- Credential display/debug redaction.
- URL normalization and discovery endpoint derivation.
- Fuzzy model-search ranking.

### Fixture Tests

- Representative models.dev payloads.
- OpenAI-compatible model-list payloads.
- Janus model and combo IDs.
- Unknown fields and future-compatible payloads.
- Malformed, oversized, and over-count responses.

### Integration Tests

Local mock servers cover:

- Catalog refresh and conditional requests.
- Atomic cache replacement and invalid-cache fallback.
- Timeout and offline fallback.
- Correct authorization headers.
- Cross-origin redirect credential stripping.
- Janus health and model discovery.
- Authenticated and unauthenticated dynamic providers.

### UI Tests

- Provider status display.
- Configured-provider filtering.
- Fuzzy search and direct-match ranking.
- Background refresh status.
- Cached-data warning.
- Model selection persistence.

### Telemetry Removal Verification

- Workspace dependency and source scans confirm Mixpanel, Sentry, event endpoints, external OTLP export, and trace-upload implementations are absent.
- Tests confirm retained local logs still operate.
- Workspace build and tests run without telemetry-related feature flags or credentials.
- A smoke test with recorded outbound requests confirms no analytics or reporting endpoint is contacted.

## Delivery Sequence

Implementation should proceed in independently verifiable stages:

1. Remove networked telemetry while preserving local diagnostics.
2. Add catalog types, deterministic generation, embedded snapshot, and conversion to existing model entries.
3. Add runtime refresh, validation, and disk caching.
4. Add API-key auth storage and provider management UI.
5. Integrate configured-provider filtering and search into the model picker.
6. Add generic dynamic OpenAI-compatible provider discovery.
7. Add the Janus preset and setup flow.
8. Complete security, integration, UI, and outbound-network verification.

Each stage must keep the workspace buildable and retain existing xAI model behavior.
