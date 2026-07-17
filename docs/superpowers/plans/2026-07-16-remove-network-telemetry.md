# Remove Network Telemetry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove all networked analytics, error reporting, OTLP export, Computer Hub telemetry donation, and trace upload while preserving local diagnostics.

**Architecture:** Keep `xai-grok-telemetry` as a local-only compatibility facade with no-op event APIs, while deleting network emitters and their integration points. Remove upload and donation paths in build-preserving stages, retaining local logs and crash formatting.

**Tech Stack:** Rust, Cargo workspace, Tokio, serde, existing shell/pager test infrastructure

---

## File Responsibility Map

### Local-only telemetry facade

- Keep and simplify `crates/codegen/xai-grok-telemetry/src/lib.rs`: export only local log/instrumentation modules and compatibility event APIs.
- Replace `crates/codegen/xai-grok-telemetry/src/client.rs`: retain `Metadata`, `TelemetryClient`, `UserContext`, `init`, `init_if_needed`, `is_enabled`, `is_session_metrics_enabled`, `track`, and `sync_profile` as source-compatible no-ops; no global queue, HTTP client, identity collection, or payload enrichment remains.
- Replace `crates/codegen/xai-grok-telemetry/src/config.rs`: retain `TelemetryMode` and `TelemetryConfig` as compatibility types; `TelemetryMode` has only `Disabled`, all legacy boolean/string inputs deserialize to `Disabled`, and `TelemetryConfig` has no event URL, API key, Mixpanel, trace-upload, or OTLP fields.
- Simplify `crates/codegen/xai-grok-telemetry/src/session_ctx.rs`: retain `TelemetryCtx`, the task-local session span, `EmitterOrigin`, and all public event functions, but make event functions synchronous no-ops that do not serialize or clone their payloads.
- Simplify `crates/codegen/xai-grok-telemetry/src/events.rs`: retain event structs and `TelemetryEvent::NAME`; remove `external_record` and make the existing `external = ...` macro arm ignore the mapping expression so call sites remain unchanged while the external schema is deleted.
- Keep `crates/codegen/xai-grok-telemetry/src/appender.rs`, `context.rs`, `debug_log.rs`, `enums.rs`, `hooks_log.rs`, `instrumentation.rs`, `memory_log.rs`, `memory_telemetry.rs`, `prompt_timing.rs`, `sampling_log.rs`, and `unified_log.rs`: these implement local diagnostics.
- Keep `crates/codegen/xai-grok-telemetry/src/http.rs`: it only re-exports `OriginClientInfo` for compatibility signatures and performs no request.
- Keep `crates/codegen/xai-grok-telemetry/src/id.rs` because non-telemetry product request/session code consumes its stable local identifier. Delete `redact_common.rs` with the exporter modules. Keep `session_metrics.rs` event structs, but remove `TraceUploadReason` and trace-upload lifecycle structs after Task 7 removes their call sites.

### Product analytics and Mixpanel

- Delete `crates/codegen/xai-mixpanel/Cargo.toml` and `crates/codegen/xai-mixpanel/src/lib.rs`.
- Delete `crates/codegen/xai-grok-telemetry/tests/manual_auth_emit.rs`; it is a manual remote-emission executable, not a local diagnostic test.
- Update root `Cargo.toml`: remove workspace member `crates/codegen/xai-mixpanel` and workspace dependency `xai-mixpanel`.
- Update `crates/codegen/xai-grok-telemetry/Cargo.toml`: remove `xai-mixpanel`, `reqwest`, `chrono`, `whoami`, `mid`, `obfstr`, and other dependencies left unused by the local-only facade.

### Internal and external OTLP

- Delete `crates/codegen/xai-grok-telemetry/src/external/config.rs`, `emit.rs`, `mod.rs`, `providers.rs`, `redact.rs`, `schema.rs`, `tests.rs`, and `truncate.rs`.
- Delete `crates/codegen/xai-grok-telemetry/src/otel_layer/mod.rs` and `otel_layer/redact.rs`.
- Delete `crates/codegen/xai-grok-telemetry/src/otlp_http.rs`.
- Delete `crates/codegen/xai-grok-telemetry/src/redact_common.rs`.
- Delete `crates/codegen/xai-grok-telemetry/tests/external_otlp.rs` and `external_otlp_session_ctx.rs`.
- Update `crates/codegen/xai-grok-pager-bin/src/main.rs`: remove internal `build_otel_layer`, external `external::init`, OTEL guard, and OTEL shutdown/flush wiring while retaining fmt, debug, sampling, hook, and instrumentation layers.
- Update `crates/codegen/xai-grok-pager/src/tracing.rs`: remove both OTLP layers and keep `TracingChannelMakeWriter`, local fmt, instrumentation, sampling, hooks, and debug-log firehose.
- Update `crates/codegen/xai-grok-pager/src/app/signal_handler.rs` and `crates/codegen/xai-grok-shell/src/instrumentation.rs`: remove remote exporter shutdown calls while preserving terminal restoration and local profiler shutdown.
- Update `crates/codegen/xai-grok-shell/src/auth/credential_provider.rs` and `auth/flow.rs`: remove the process-wide OTEL credential provider, identity synchronization, and external flush calls; authentication behavior itself remains unchanged.
- Update `crates/codegen/xai-grok-shell/src/agent/config.rs`, `config/mod.rs`, and `config/tests.rs`: remove all internal/external OTLP fields, environment resolution, managed-config pins, and tests.
- Update `crates/codegen/xai-grok-config-types/src/lib.rs`: remove `external_otel_disabled`, `external_otel_content_gates_locked`, `telemetry_enabled`, `telemetry_mode`, and `trace_upload_enabled`; preserve unrelated remote settings.

### Sentry and crash handling

- Delete `crates/codegen/xai-grok-telemetry/src/sentry.rs` and remove the `sentry` dependency from `crates/codegen/xai-grok-telemetry/Cargo.toml`.
- Update `crates/codegen/xai-grok-pager-bin/src/main.rs`: remove `sentry::init`, the guard, and shutdown flush.
- Update `crates/codegen/xai-grok-pager/src/app/signal_handler.rs`: remove Sentry flush only; preserve stderr restoration, panic/crash formatting, and exit behavior.
- Keep `crates/codegen/xai-crash-handler/**`: this is local crash capture/formatting and has no upload path after the shell upload removal.
- Do not remove unrelated user-configured MCP/plugin examples named `sentry`; scans must distinguish the telemetry SDK/dependency from ordinary strings used as plugin names.

### `xai-tracing` and trace context

- Update `crates/common/xai-tracing/src/fastrace.rs`: delete `init_fastrace` and the OTLP `OpenTelemetryReporter`; retain the local fastrace span helpers. Delete request middleware that injects `traceparent`, `tracestate`, or exporter correlation headers.
- Update `crates/common/xai-tracing/src/lib.rs`: stop exporting `init_fastrace` and network propagation APIs.
- Delete `crates/common/xai-tracing/src/http_client.rs`, `grpc_client.rs`, and `testing.rs` if their only responsibility remains distributed trace-header injection; keep `dispatch.rs`, `timer.rs`, and `tokio.rs` for local timing/instrumentation.
- Update `crates/common/xai-tracing/Cargo.toml`: remove `fastrace-opentelemetry`, `opentelemetry*`, `tracing-opentelemetry`, `reqwest*`, `tonic`, `tower`, and wiremock dependencies that become unused.
- Replace `crates/codegen/xai-file-utils/src/trace_context.rs` with compatibility functions that return `None`, return request builders unchanged, and create ordinary local `tracing` spans. This avoids broad call-site churn while guaranteeing that inference/service requests carry no `traceparent` or `tracestate` telemetry correlation.
- Update `crates/codegen/xai-file-utils/Cargo.toml`: remove `opentelemetry`, `opentelemetry_sdk`, `tracing-opentelemetry`, and related test dependencies.

### Computer Hub donation

- Delete `crates/common/xai-computer-hub-sdk/src/donate_pump.rs`, `log_donate.rs`, `metric_donate.rs`, and `trace_donate.rs`.
- Update `crates/common/xai-computer-hub-sdk/src/lib.rs`, `server.rs`, and `Cargo.toml`: remove donation modules, pump state, donation methods, drain fences, exports, and OTLP/base64/prost dependencies while preserving the Hub tool transport.
- Update `crates/common/xai-tool-protocol/src/frames.rs`, `lib.rs`, and `methods.rs`: remove `TracesDonateParams`, `LogsDonateParams`, `MetricsDonateParams`, donation size constants, and `Method::{TracesDonate,LogsDonate,MetricsDonate}`.
- Update `crates/codegen/xai-grok-workspace/src/handle.rs` and `bin/workspace_server.rs`: remove reporter/layer accessors, `GROK_WORKSPACE_OTLP_ENDPOINT`, donation startup, and drain handles; retain the local fmt subscriber and workspace metrics used locally.

### Shell trace upload

- Delete `crates/codegen/xai-grok-shell/src/upload/config_files.rs`, `manifest.rs`, `trace.rs`, and `turn.rs`.
- Update `crates/codegen/xai-grok-shell/src/upload/mod.rs`: retain only `gcs` while it is needed for explicit feedback/share/storage features; no trace artifact API may remain.
- Trim `crates/codegen/xai-grok-shell/src/upload/gcs.rs`: retain only non-telemetry helpers still used by explicit user features; remove `TraceExportConfigWithAuth`, trace bucket constants, auth-diagnostics upload, trace URL construction, and upload queue integration.
- Update `crates/codegen/xai-grok-shell/src/agent/mvp_agent/mod.rs`, `agent_ops.rs`, `acp_agent.rs`, `heap_profile.rs`, `subagent_coordinator.rs`, `agent/subagent/mod.rs`, `agent/subagent/handle_request.rs`, `agent/subagent/coordinator_query.rs`, and `agent/subagent/coordinator_lifecycle.rs`: remove `PromptTraceContext`, upload futures, artifact trackers, upload queue startup/reconciliation, metadata/archive/image/config/log uploads, and trace upload resolution; preserve prompt execution, local history, subagent coordination, and local heap profiling.
- Update `crates/codegen/xai-grok-shell/src/tools/notification_bridge.rs`, `tools/tool_context.rs`, `sampling/conversation.rs`, `session/commands.rs`, `extensions/feedback.rs`, `extensions/share.rs`, and `session/feedback_manager.rs`: remove trace upload channels/contexts and uploaded-log URLs. Keep explicit feedback submission and sharing, and add assertions that their request payloads contain no automatic diagnostic log, trace, archive, or configuration attachment.
- Update `crates/codegen/xai-grok-shell/src/agent/config.rs`, `config/mod.rs`, `config/tests.rs`, and `tests/test_heap_profile_monitor.rs`: remove trace-upload endpoint/credentials/bucket/region fields, `TelemetryConfig.trace_upload`, requirements pins, environment variables, resolution APIs, and upload-enabled tests.
- Delete `crates/codegen/xai-grok-pager/tests/pty_e2e/storage_upload_parks_on_401_and_drains_after_recovery.rs`; it validates the removed upload queue.
- Audit `crates/codegen/xai-file-utils/src/gcs.rs`, `queue.rs`, `s3.rs`, `storage_client.rs`, and `upload_config.rs` by symbol and consumer. Remove trace-upload-only types and branches, but retain generic cloud/storage modules used by explicit feedback, share, remote workspace, or other user-requested product features. Delete a module only when the Task 8 consumer scan shows no non-telemetry import.
- Remove trace-upload-only proxy DTO usage from `prod/mc/cli-chat-proxy-types` only when `cargo check` proves no non-telemetry consumer remains; do not remove snapshot, inference, feedback, or other product storage DTOs.

### Manifests, lockfile, and verification

- Update root `Cargo.toml`, all affected crate manifests, and `Cargo.lock` through `cargo check`, not hand-edited lockfile surgery.
- Add `bin/check-no-network-telemetry.sh`: source/dependency denylist with explicit allow rules for update checks and announcement fetching.
- Add `crates/codegen/xai-grok-pager/tests/pty_e2e/local_diagnostics_survive_telemetry_removal.rs`: exercise unified/debug/hook/sampling logs in the existing isolated PTY harness.
- Add `crates/codegen/xai-grok-telemetry/tests/local_only_facade.rs`: prove event payloads are never serialized.
- Keep `crates/codegen/xai-grok-announcements/**`, `crates/codegen/xai-grok-update/**`, and `crates/codegen/xai-grok-shell-base/src/util/changelog.rs` unchanged; their networking is explicitly allowed.

## Task 1: Establish the Local-Only Compatibility Contract

**Files:**
- Create: `crates/codegen/xai-grok-telemetry/tests/local_only_facade.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/config.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/events.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/session_ctx.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/client.rs`

- [ ] **Step 1: Write the failing facade tests**

Create `local_only_facade.rs` with a serialization probe and legacy config tests:

```rust
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use xai_grok_telemetry::config::{TelemetryConfig, TelemetryMode};
use xai_grok_telemetry::events::TelemetryEvent;

static SERIALIZED: AtomicBool = AtomicBool::new(false);

struct Probe;

impl Serialize for Probe {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SERIALIZED.store(true, Ordering::SeqCst);
        serializer.serialize_unit()
    }
}

impl TelemetryEvent for Probe {
    const NAME: &'static str = "probe";
}

#[tokio::test]
async fn event_apis_never_serialize_payloads() {
    SERIALIZED.store(false, Ordering::SeqCst);
    xai_grok_telemetry::log_event(Probe);
    xai_grok_telemetry::log_session_event(Probe);
    xai_grok_telemetry::emit_event("probe", Probe);
    tokio::task::yield_now().await;
    assert!(!SERIALIZED.load(Ordering::SeqCst));
}

#[test]
fn all_legacy_modes_resolve_to_disabled() {
    for input in ["true", "false", "\"enabled\"", "\"session_metrics\""] {
        let mode: TelemetryMode = serde_json::from_str(input).unwrap();
        assert_eq!(mode, TelemetryMode::Disabled);
    }
    assert_eq!(TelemetryMode::parse("enabled"), Some(TelemetryMode::Disabled));
}

#[test]
fn config_has_no_network_fields() {
    let value = serde_json::to_value(TelemetryConfig::default()).unwrap();
    let object = value.as_object().unwrap();
    for key in [
        "events_url", "events_api_key", "mixpanel_token", "mixpanel_enabled",
        "trace_upload", "otel_enabled", "otel_metrics_exporter",
        "otel_logs_exporter", "otel_endpoint", "otel_protocol",
    ] {
        assert!(!object.contains_key(key), "network field survived: {key}");
    }
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test -p xai-grok-telemetry --test local_only_facade -- --nocapture`

Expected: FAIL because enabled modes remain enabled, `TelemetryConfig` serializes network fields, and an enabled/external path can serialize `Probe`.

- [ ] **Step 3: Make the facade minimal and payload-free**

Replace `TelemetryMode` and `TelemetryConfig` with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TelemetryMode {
    #[default]
    Disabled,
}

impl TelemetryMode {
    pub fn is_disabled(self) -> bool { true }
    pub fn is_enabled(self) -> bool { false }
    pub fn session_metrics_enabled(self) -> bool { false }
    pub fn parse(s: &str) -> Option<Self> {
        (!s.trim().is_empty()).then_some(Self::Disabled)
    }
}

impl From<bool> for TelemetryMode {
    fn from(_: bool) -> Self { Self::Disabled }
}

impl serde::Serialize for TelemetryMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(false)
    }
}

impl<'de> serde::Deserialize<'de> for TelemetryMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
        Ok(Self::Disabled)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TelemetryConfig {}

impl TelemetryConfig {
    pub fn apply_env_overrides(&mut self) {}
}
```

Keep `TelemetryEvent` as `Serialize + Send + 'static` for source compatibility, but remove `external_record`; make both macro arms implement only `NAME`:

```rust
macro_rules! telemetry_event {
    ($struct:path, $name:literal) => {
        impl $crate::events::TelemetryEvent for $struct {
            const NAME: &'static str = $name;
        }
    };
    ($struct:path, $name:literal, external = $mapper:path) => {
        impl $crate::events::TelemetryEvent for $struct {
            const NAME: &'static str = $name;
        }
    };
}
```

Make every event API consume and immediately drop its arguments without formatting, cloning, spawning, or serializing:

```rust
pub fn log_event<T: TelemetryEvent>(_: T) {}
pub fn log_event_dual<T: TelemetryEvent>(_: bool, _: T) {}
pub fn log_session_event<T: TelemetryEvent>(_: T) {}
pub fn log_session_event_with_origin<T: TelemetryEvent>(_: EmitterOrigin, _: T) {}
pub fn emit_event<T: Serialize + Send + 'static>(_: impl Into<String>, _: T) {}
pub fn emit_event_with_origin<T: Serialize + Send + 'static>(
    _: EmitterOrigin,
    _: impl Into<String>,
    _: T,
) {}
```

Reduce client state and event APIs to no-ops. Keep existing argument lists for `init` and `init_if_needed` in this task so downstream crates remain buildable; prefix arguments with `_` and do not store them:

```rust
#[derive(Debug, Clone, Default)]
pub struct TelemetryClient;

pub fn is_enabled() -> bool { false }
pub fn is_session_metrics_enabled() -> bool { false }
pub async fn track(_: &str, _: &str, _: &UserContext, _: Metadata) {}
pub fn sync_profile() {}
```

- [ ] **Step 4: Verify the facade and its direct dependents are green**

Run: `cargo test -p xai-grok-telemetry --test local_only_facade && cargo check -p xai-grok-shell -p xai-grok-workspace -p xai-grok-pager`

Expected: all three facade tests PASS and all four crates compile.

- [ ] **Step 5: Commit**

```bash
git add crates/codegen/xai-grok-telemetry/src/client.rs crates/codegen/xai-grok-telemetry/src/config.rs crates/codegen/xai-grok-telemetry/src/events.rs crates/codegen/xai-grok-telemetry/src/session_ctx.rs crates/codegen/xai-grok-telemetry/tests/local_only_facade.rs
git commit -m "refactor: make telemetry event APIs local-only no-ops"
```

## Task 2: Remove Product Event and Mixpanel Emitters

**Files:**
- Delete: `crates/codegen/xai-mixpanel/Cargo.toml`
- Delete: `crates/codegen/xai-mixpanel/src/lib.rs`
- Delete: `crates/codegen/xai-grok-telemetry/tests/manual_auth_emit.rs`
- Modify: `Cargo.toml`
- Modify: `crates/codegen/xai-grok-telemetry/Cargo.toml`
- Modify: `crates/codegen/xai-grok-secrets/Cargo.toml`

- [ ] **Step 1: Add a failing source-level assertion to the facade test**

Append:

```rust
#[test]
fn product_analytics_crate_is_absent() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    assert!(!root.join("crates/codegen/xai-mixpanel").exists());
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(!manifest.contains("xai-mixpanel"));
}
```

- [ ] **Step 2: Run the assertion and verify RED**

Run: `cargo test -p xai-grok-telemetry --test local_only_facade product_analytics_crate_is_absent -- --exact`

Expected: FAIL at the directory assertion because `crates/codegen/xai-mixpanel` exists.

- [ ] **Step 3: Delete the emitter crate and manifest edges**

Delete the two `xai-mixpanel` files and its now-empty directory. Remove these exact root manifest entries:

```toml
"crates/codegen/xai-mixpanel",
xai-mixpanel = { path = "crates/codegen/xai-mixpanel" }
```

Remove `xai-mixpanel = { workspace = true }` from the telemetry manifest and delete `manual_auth_emit.rs`. Change the secrets crate description to describe sanitization without claiming outbound Sentry/Mixpanel use:

```toml
description = "Regex sanitizer for locally formatted Grok Build diagnostics"
```

- [ ] **Step 4: Verify the crate is absent and the facade builds**

Run: `cargo test -p xai-grok-telemetry --test local_only_facade product_analytics_crate_is_absent -- --exact && cargo check -p xai-grok-telemetry`

Expected: PASS; Cargo reports no package or dependency named `xai-mixpanel`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/codegen/xai-grok-telemetry/Cargo.toml crates/codegen/xai-grok-telemetry/tests crates/codegen/xai-grok-secrets/Cargo.toml
git add -u crates/codegen/xai-mixpanel
git commit -m "refactor: remove product analytics and mixpanel"
```

## Task 3: Remove Internal and External OTLP Export

**Files:**
- Delete: `crates/codegen/xai-grok-telemetry/src/external/`
- Delete: `crates/codegen/xai-grok-telemetry/src/otel_layer/`
- Delete: `crates/codegen/xai-grok-telemetry/src/otlp_http.rs`
- Delete: `crates/codegen/xai-grok-telemetry/src/redact_common.rs`
- Delete: `crates/codegen/xai-grok-telemetry/tests/external_otlp.rs`
- Delete: `crates/codegen/xai-grok-telemetry/tests/external_otlp_session_ctx.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/lib.rs`
- Modify: `crates/codegen/xai-grok-pager-bin/src/main.rs`
- Modify: `crates/codegen/xai-grok-pager/src/tracing.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/signal_handler.rs`
- Modify: `crates/codegen/xai-grok-shell/src/instrumentation.rs`
- Modify: `crates/codegen/xai-grok-shell/src/auth/credential_provider.rs`
- Modify: `crates/codegen/xai-grok-shell/src/auth/flow.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/config.rs`
- Modify: `crates/codegen/xai-grok-shell/src/config/mod.rs`
- Modify: `crates/codegen/xai-grok-shell/src/config/tests.rs`
- Modify: `crates/codegen/xai-grok-config-types/src/lib.rs`

- [ ] **Step 1: Add a failing compile-time local tracing test**

In `crates/codegen/xai-grok-pager/src/tracing.rs` tests, add:

```rust
#[test]
fn tracing_registry_contains_only_local_layers() {
    let source = include_str!("tracing.rs");
    for forbidden in ["build_otel_layer", "external::init", "OTEL_EXPORTER", "GROK_EXTERNAL_OTEL"] {
        assert!(!source.contains(forbidden), "remote tracing hook survived: {forbidden}");
    }
    assert!(source.contains("instrumentation::layer"));
    assert!(source.contains("sampling_log::layer"));
    assert!(source.contains("hooks_log::layer"));
    assert!(source.contains("debug_log::install_firehose"));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test -p xai-grok-pager tracing_registry_contains_only_local_layers -- --exact`

Expected: FAIL with `remote tracing hook survived: build_otel_layer`.

- [ ] **Step 3: Remove exporter modules and startup integration**

Delete the listed exporter files and remove these module exports from telemetry `lib.rs`:

```rust
pub mod external;
pub mod otel_layer;
pub(crate) mod otlp_http;
```

In pager tracing setup, build the registry with local layers only:

```rust
let registry = tracing_subscriber::registry()
    .with(fmt_layer.with_filter(env_filter))
    .with(xai_grok_telemetry::instrumentation::layer())
    .with(xai_grok_telemetry::sampling_log::layer())
    .with(xai_grok_telemetry::hooks_log::layer());
xai_grok_telemetry::debug_log::install_firehose(registry, app_entrypoint);
```

Apply the equivalent local-only chain in `xai-grok-pager/src/tracing.rs`. Remove every `otel_guard`, `shutdown_otel`, `external::init`, `external::set_identity`, `external::flush`, and `external::apply_remote_policy` call.

Delete from `EndpointsConfig` the fields from `otel_exporter_otlp_endpoint` through `otel_exporter_otlp_timeout`, and delete `parse_otlp_header_list`, `resolve_otlp_traces_endpoint`, `legacy_internal_otlp_traces_endpoint`, `resolve_otlp_headers`, `internal_otlp_consumed_standard_vars`, `resolve_traces_export_enabled`, `resolve_otlp_export_interval`, and `resolve_otlp_timeout`. Delete external OTEL resolution functions and their test block around `ext_client()`.

Delete from `TelemetryConfig` all `otel_*` fields. Delete `external_otel_disabled`, `external_otel_content_gates_locked`, `telemetry_enabled`, `telemetry_mode`, and `trace_upload_enabled` from `RemoteSettings` in `xai-grok-config-types`, plus the exact shell reads of those fields. Preserve all other remote settings, especially update, announcement, model, and feature settings.

- [ ] **Step 4: Verify local tracing and affected binaries**

Run: `cargo test -p xai-grok-pager tracing_registry_contains_only_local_layers -- --exact && cargo check -p xai-grok-pager-bin -p xai-grok-shell -p xai-grok-workspace`

Expected: PASS; no unresolved OTLP/external symbols and local tracing assertions remain true.

- [ ] **Step 5: Commit**

```bash
git add crates/codegen/xai-grok-telemetry crates/codegen/xai-grok-pager-bin/src/main.rs crates/codegen/xai-grok-pager/src/tracing.rs crates/codegen/xai-grok-pager/src/app/signal_handler.rs crates/codegen/xai-grok-shell/src/instrumentation.rs crates/codegen/xai-grok-shell/src/auth/credential_provider.rs crates/codegen/xai-grok-shell/src/auth/flow.rs crates/codegen/xai-grok-shell/src/agent/config.rs crates/codegen/xai-grok-shell/src/config crates/codegen/xai-grok-config-types/src/lib.rs
git commit -m "refactor: remove internal and external otlp export"
```

## Task 4: Remove Sentry Without Removing Local Crash Formatting

**Files:**
- Delete: `crates/codegen/xai-grok-telemetry/src/sentry.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/lib.rs`
- Modify: `crates/codegen/xai-grok-telemetry/Cargo.toml`
- Modify: `crates/codegen/xai-grok-pager-bin/src/main.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/signal_handler.rs`

- [ ] **Step 1: Add a failing source assertion beside signal-handler tests**

Add:

```rust
#[test]
fn shutdown_restores_terminal_without_remote_error_reporting() {
    let source = include_str!("signal_handler.rs");
    assert!(source.contains("restore_stderr"));
    assert!(!source.contains("sentry::"));
    assert!(!source.contains("Sentry"));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test -p xai-grok-pager shutdown_restores_terminal_without_remote_error_reporting -- --exact`

Expected: FAIL because `signal_handler.rs` calls `xai_grok_telemetry::sentry::flush_on_shutdown()`.

- [ ] **Step 3: Remove Sentry integration only**

Delete `sentry.rs`, remove `pub mod sentry;`, remove the complete `sentry = { ... }` dependency block, and remove pager startup/shutdown calls. Keep terminal restoration in this order:

```rust
restore_terminal_state();
restore_stderr();
xai_grok_telemetry::debug_log::flush();
```

Do not modify `crates/codegen/xai-crash-handler`; local panic messages, stack formatting, and crash files stay available.

- [ ] **Step 4: Verify signal handling and crash crate compilation**

Run: `cargo test -p xai-grok-pager shutdown_restores_terminal_without_remote_error_reporting -- --exact && cargo check -p xai-grok-pager-bin -p xai-crash-handler`

Expected: PASS with no Sentry SDK symbol while local crash handling compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/codegen/xai-grok-telemetry/src/lib.rs crates/codegen/xai-grok-telemetry/Cargo.toml crates/codegen/xai-grok-pager-bin/src/main.rs crates/codegen/xai-grok-pager/src/app/signal_handler.rs
git add -u crates/codegen/xai-grok-telemetry/src/sentry.rs
git commit -m "refactor: remove sentry reporting"
```

## Task 5: Remove Network Exporters and Correlation Headers From Tracing Utilities

**Files:**
- Modify: `crates/common/xai-tracing/src/fastrace.rs`
- Modify: `crates/common/xai-tracing/src/lib.rs`
- Delete: `crates/common/xai-tracing/src/http_client.rs`
- Delete: `crates/common/xai-tracing/src/grpc_client.rs`
- Delete: `crates/common/xai-tracing/src/testing.rs`
- Modify: `crates/common/xai-tracing/Cargo.toml`
- Modify: `crates/codegen/xai-file-utils/src/trace_context.rs`
- Modify: `crates/codegen/xai-file-utils/Cargo.toml`

- [ ] **Step 1: Replace propagation tests with a failing privacy contract**

Replace tests in `xai-file-utils/src/trace_context.rs` with:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn implementation_has_no_opentelemetry_bridge() {
        let source = include_str!("trace_context.rs");
        assert!(!source.contains("opentelemetry"));
        assert!(!source.contains("OpenTelemetrySpanExt"));
    }

    #[test]
    fn request_builder_never_gains_trace_headers() {
        let request = super::inject_trace_context_into_request(
            reqwest::Client::new().get("https://example.invalid"),
        )
        .build()
        .unwrap();
        assert!(!request.headers().contains_key("traceparent"));
        assert!(!request.headers().contains_key("tracestate"));
    }

    #[test]
    fn no_distributed_context_is_exposed() {
        assert_eq!(super::current_traceparent(), None);
        let span = super::span_from_meta_traceparent(&serde_json::Map::new());
        assert!(!span.is_none());
    }
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test -p xai-file-utils trace_context::tests -- --nocapture`

Expected: FAIL in `implementation_has_no_opentelemetry_bridge` because the current module imports `opentelemetry::global` and `OpenTelemetrySpanExt`.

- [ ] **Step 3: Convert trace context to local-only compatibility behavior**

Replace the module implementation with:

```rust
pub fn current_traceparent() -> Option<String> { None }

pub fn inject_trace_context_into_request(
    builder: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    builder
}

pub(crate) fn trace_context_headers() -> reqwest::header::HeaderMap {
    reqwest::header::HeaderMap::new()
}

pub fn span_from_meta_traceparent(
    _: &serde_json::Map<String, serde_json::Value>,
) -> tracing::Span {
    tracing::info_span!("acp_dispatch")
}

pub fn link_current_span_to_meta(_: &serde_json::Value) {}
```

Remove `init_fastrace` from `xai-tracing/src/fastrace.rs`; retain only local span creation helpers. Delete the three distributed-client files and exports. Remove their now-unused dependencies from both manifests.

- [ ] **Step 4: Verify local timing callers still compile**

Run: `cargo test -p xai-file-utils trace_context::tests && cargo test -p xai-tracing && cargo check -p xai-grok-shell -p xai-grok-workspace`

Expected: PASS; local span/timer APIs compile and request builders contain neither W3C trace header.

- [ ] **Step 5: Commit**

```bash
git add crates/common/xai-tracing crates/codegen/xai-file-utils/src/trace_context.rs crates/codegen/xai-file-utils/Cargo.toml
git commit -m "refactor: keep tracing local and remove correlation headers"
```

## Task 6: Remove Computer Hub Telemetry Donation and Wire Frames

**Files:**
- Delete: `crates/common/xai-computer-hub-sdk/src/donate_pump.rs`
- Delete: `crates/common/xai-computer-hub-sdk/src/log_donate.rs`
- Delete: `crates/common/xai-computer-hub-sdk/src/metric_donate.rs`
- Delete: `crates/common/xai-computer-hub-sdk/src/trace_donate.rs`
- Modify: `crates/common/xai-computer-hub-sdk/src/lib.rs`
- Modify: `crates/common/xai-computer-hub-sdk/src/server.rs`
- Modify: `crates/common/xai-computer-hub-sdk/Cargo.toml`
- Modify: `crates/common/xai-tool-protocol/src/frames.rs`
- Modify: `crates/common/xai-tool-protocol/src/lib.rs`
- Modify: `crates/common/xai-tool-protocol/src/methods.rs`
- Modify: `crates/codegen/xai-grok-workspace/src/handle.rs`
- Modify: `crates/codegen/xai-grok-workspace/src/bin/workspace_server.rs`

- [ ] **Step 1: Add a failing protocol deny test**

Add to `xai-tool-protocol/src/methods.rs` tests:

```rust
#[test]
fn telemetry_donation_methods_are_not_in_the_protocol() {
    for wire in ["traces.donate", "logs.donate", "metrics.donate"] {
        assert!(Method::from_wire_str(wire).is_none(), "donation method survived: {wire}");
    }
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test -p xai-tool-protocol telemetry_donation_methods_are_not_in_the_protocol -- --exact`

Expected: FAIL for `traces.donate` because it currently parses as `Method::TracesDonate`.

- [ ] **Step 3: Delete donation implementation and protocol surface**

Delete all four donation modules. Remove their module declarations/re-exports and remove `donation_pumps` from `ToolServerInner`. Delete `DonationPumps`, `donate_traces`, `donate_logs`, `donate_metrics`, pump setters, `flush_donations`, and `flush_donations_inner`; shutdown should proceed directly to existing connection/session teardown.

Delete from `frames.rs` the donation constants and params structs. Remove their re-exports from `lib.rs`. Remove these variants and wire values from `methods.rs`:

```rust
TracesDonate => "traces.donate",
LogsDonate => "logs.donate",
MetricsDonate => "metrics.donate",
```

In workspace startup, keep a strictly local subscriber:

```rust
tracing_subscriber::registry()
    .with(env_filter)
    .with(tracing_subscriber::fmt::layer())
    .init();
```

Delete the three reporter accessors from `WorkspaceHandle`, all reporter setup after `connect_local_workspace`, and donation drain handles at shutdown. Remove OTLP/fastrace/base64/prost dependencies that become unused from the Hub SDK manifest.

- [ ] **Step 4: Verify protocol, SDK, and workspace server**

Run: `cargo test -p xai-tool-protocol telemetry_donation_methods_are_not_in_the_protocol -- --exact && cargo test -p xai-computer-hub-sdk && cargo check -p xai-grok-workspace --bin workspace-server`

Expected: PASS with ordinary Hub tool methods intact and no donation reporter symbols.

- [ ] **Step 5: Commit**

```bash
git add crates/common/xai-computer-hub-sdk crates/common/xai-tool-protocol crates/codegen/xai-grok-workspace/src/handle.rs crates/codegen/xai-grok-workspace/src/bin/workspace_server.rs
git commit -m "refactor: remove computer hub telemetry donation"
```

## Task 7: Remove Shell Trace Upload Configuration and Artifact Pipeline

**Files:**
- Delete: `crates/codegen/xai-grok-shell/src/upload/config_files.rs`
- Delete: `crates/codegen/xai-grok-shell/src/upload/manifest.rs`
- Delete: `crates/codegen/xai-grok-shell/src/upload/trace.rs`
- Delete: `crates/codegen/xai-grok-shell/src/upload/turn.rs`
- Modify: `crates/codegen/xai-grok-shell/src/upload/mod.rs`
- Modify: `crates/codegen/xai-grok-shell/src/upload/gcs.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/config.rs`
- Modify: `crates/codegen/xai-grok-shell/src/config/mod.rs`
- Modify: `crates/codegen/xai-grok-shell/src/config/tests.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/mod.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/heap_profile.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/mvp_agent/subagent_coordinator.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/subagent/handle_request.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/subagent/coordinator_query.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/subagent/coordinator_lifecycle.rs`
- Modify: `crates/codegen/xai-grok-shell/src/tools/notification_bridge.rs`
- Modify: `crates/codegen/xai-grok-shell/src/tools/tool_context.rs`
- Modify: `crates/codegen/xai-grok-shell/src/sampling/conversation.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/commands.rs`
- Modify: `crates/codegen/xai-grok-shell/src/extensions/feedback.rs`
- Modify: `crates/codegen/xai-grok-shell/src/extensions/share.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/feedback_manager.rs`
- Modify: `crates/codegen/xai-grok-shell/src/session/storage/search_remote_sync.rs`
- Modify: `crates/codegen/xai-grok-shell/tests/test_heap_profile_monitor.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/session_metrics.rs`
- Modify: `crates/codegen/xai-grok-telemetry/src/events.rs`
- Modify: `crates/codegen/xai-grok-shell/src/agent/session_metrics.rs`
- Delete: `crates/codegen/xai-grok-pager/tests/pty_e2e/storage_upload_parks_on_401_and_drains_after_recovery.rs`

- [ ] **Step 1: Add failing config regression tests**

Add to `agent/config.rs` tests:

```rust
#[test]
fn telemetry_config_ignores_removed_upload_keys() {
    let parsed: xai_grok_telemetry::config::TelemetryConfig = toml::from_str(
        "trace_upload = true\nmixpanel_enabled = true\notel_endpoint = 'https://collector.invalid'",
    )
    .unwrap();
    assert_eq!(toml::Value::try_from(parsed).unwrap(), toml::Value::Table(Default::default()));
}

#[test]
fn endpoints_config_has_no_trace_upload_serialization() {
    let value = toml::Value::try_from(super::EndpointsConfig::default()).unwrap();
    let table = value.as_table().unwrap();
    for key in [
        "trace_upload_url", "trace_upload_bucket", "trace_upload_region",
        "trace_upload_credentials_file", "trace_upload_credentials",
        "trace_upload_endpoint_url",
    ] {
        assert!(!table.contains_key(key), "trace upload field survived: {key}");
    }
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test -p xai-grok-shell telemetry_config_ignores_removed_upload_keys -- --exact --nocapture && cargo test -p xai-grok-shell endpoints_config_has_no_trace_upload_serialization -- --exact --nocapture`

Expected: FAIL because `EndpointsConfig` still serializes all six trace-upload fields.

- [ ] **Step 3: Remove configuration, pipeline modules, and integration**

Delete the four upload modules and leave:

```rust
pub mod gcs;
```

in `upload/mod.rs` only while `gcs` has non-trace consumers. Remove from `EndpointsConfig` the six trace upload fields and all `resolve_trace_*`, `resolve_*upload_method`, `has_noninteractive_upload_auth`, and trace bucket methods. Remove `requirements.trace_upload`, `is_trace_upload_enabled`, `resolve_trace_upload`, and managed config handling for `telemetry.trace_upload` and trace endpoints.

In `MvpAgent`, delete `trace_upload_config`, `trace_upload_config_snapshot`, `trace_upload_config_with_reason`, `diagnostic_upload_config`, queue startup/reconciliation, and all upload futures. Prompt completion should await only model/tool/session work; do not replace uploads with local copies because local history and unified logs already own persistence.

Remove `PromptTraceContext`, `SyntheticTurnTraceRequest`, `UploadWait`, `ArtifactTracker`, and `ArtifactUploadContext` fields from tool, conversation, subagent, and command structures. Delete constructor arguments and channel creation together so every intermediate commit remains type-correct.

Delete `TraceUploadReason`, `TraceUploadAttempted`, `TraceUploadFailed`, `TraceUploadSkipped`, and `TraceUploadSucceeded` from `xai-grok-telemetry/src/session_metrics.rs`, remove their `telemetry_event!` bindings from `events.rs`, and remove their imports/emissions from `xai-grok-shell/src/agent/session_metrics.rs`.

For feedback/share, remove `unified_log_url` population and diagnostic upload calls. Keep explicit submission itself. For heap profiling, keep local profile creation/retention and remove only upload method/config fields and upload tasks.

Keep `upload/gcs.rs` for the explicit feedback and remote storage consumers. Remove `TraceExportConfigWithAuth`, `WithAuth`, `SESSION_TRACES_BUCKET`, `unified_log_url`, `upload_to_auth_diagnostics`, and all imports of `xai_file_utils::{TraceExportConfig, UploadMethod}`. Update `session/storage/search_remote_sync.rs` to use its existing product-storage client directly rather than the removed trace-export wrapper. `upload/mod.rs` must contain only `pub mod gcs;` at the end of this task.

- [ ] **Step 4: Verify shell behavior without upload APIs**

Run: `cargo test -p xai-grok-shell telemetry_config_ignores_removed_upload_keys -- --exact --nocapture && cargo test -p xai-grok-shell endpoints_config_has_no_trace_upload_serialization -- --exact --nocapture && cargo check -p xai-grok-shell -p xai-grok-pager && cargo test -p xai-grok-shell --test test_heap_profile_monitor`

Expected: PASS; heap profiles remain local and no upload module/type is referenced.

- [ ] **Step 5: Commit**

```bash
git add crates/codegen/xai-grok-shell crates/codegen/xai-grok-pager/tests/pty_e2e
git commit -m "refactor: remove shell trace upload pipeline"
```

## Task 8: Remove Trace-Upload APIs Without Removing Product Storage

**Files:**
- Create: `crates/codegen/xai-file-utils/tests/no_trace_upload_api.rs`
- Modify: `crates/codegen/xai-file-utils/src/gcs.rs`
- Modify: `crates/codegen/xai-file-utils/src/queue.rs`
- Modify: `crates/codegen/xai-file-utils/src/s3.rs`
- Modify: `crates/codegen/xai-file-utils/src/storage_client.rs`
- Modify: `crates/codegen/xai-file-utils/src/upload_config.rs`
- Modify: `crates/codegen/xai-file-utils/src/lib.rs`
- Modify: `crates/codegen/xai-file-utils/Cargo.toml`
- Modify: `crates/codegen/xai-grok-shell/Cargo.toml`
- Modify: `crates/codegen/xai-grok-shell/src/extensions/feedback.rs`
- Modify: `crates/codegen/xai-grok-shell/src/extensions/share.rs`

- [ ] **Step 1: Audit every generic storage module and its consumers**

Run:

```bash
rg -n 'xai_file_utils::(gcs|queue|s3|storage_client)|xai_file_utils::\{[^}]*(gcs|queue|s3|storage_client)' crates --glob '*.rs'
rg -n 'TraceExportConfig|TraceExportSource|TraceUpload|trace_upload|SESSION_TRACES_BUCKET|upload_session_metadata|spawn_trace_upload' crates/codegen/xai-file-utils crates/codegen/xai-grok-shell --glob '*.rs'
```

Expected: the first command lists explicit feedback/share and product-storage consumers, proving the generic modules cannot be deleted wholesale; the second command lists the trace-upload-specific API and its remaining call sites to remove. Record each first-command consumer in the implementation commit description as either retained product storage or removed telemetry.

- [ ] **Step 2: Add a failing trace-upload API deny test**

Create `crates/codegen/xai-file-utils/tests/no_trace_upload_api.rs`:

```rust
#[test]
fn crate_source_has_no_trace_upload_api() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = ["lib.rs", "gcs.rs", "queue.rs", "s3.rs", "storage_client.rs", "upload_config.rs"];
    let forbidden = [
        "TraceExportSource", "TraceUploadAttempted", "TraceUploadSucceeded",
        "SESSION_TRACES_BUCKET", "upload_session_metadata", "spawn_trace_upload",
    ];
    for file in files {
        let path = root.join("src").join(file);
        if !path.exists() { continue; }
        let source = std::fs::read_to_string(&path).unwrap();
        for symbol in forbidden {
            assert!(!source.contains(symbol), "trace-upload API {symbol} survived in {file}");
        }
    }
}
```

- [ ] **Step 3: Run the test and verify RED**

Run: `cargo test -p xai-file-utils --test no_trace_upload_api -- --nocapture`

Expected: FAIL with `trace-upload API TraceExportSource survived in queue.rs`.

- [ ] **Step 4: Remove trace-specific APIs and retain product storage**

Remove `TraceExportSource`, automatic session-trace queue entry points, trace bucket/path conventions, trace retry/reconciliation branches, and trace-only public re-exports from the five audited modules. Keep `TraceExportConfig`, `UploadMethod`, `gcs`, `s3`, `storage_client`, and generic queue/config APIs when the Step 1 scan ties them to explicit feedback, share, remote workspace, or user-requested storage. Preserve the names and behavior of surviving product-storage APIs; this telemetry task does not refactor them merely because they can upload bytes.

For each module, rerun the first Step 1 command scoped to that module. Delete the module and its `lib.rs` export only when the scoped command has no non-telemetry consumer after trace call-site removal. This rule is mandatory: module capability to upload bytes is not evidence that the module is telemetry.

Remove a manifest dependency only when `cargo machete` reports it unused after the trace-specific code is gone. Keep dependencies required by retained inference, updates, announcements, explicit feedback, sharing, remote workspace, and user-requested storage paths.

- [ ] **Step 5: Prove explicit feedback/share do not attach diagnostics automatically**

Add these source-boundary tests in the existing test modules after removing trace integration:

```rust
#[test]
fn feedback_submission_has_no_automatic_diagnostic_attachment() {
    let source = include_str!("feedback.rs");
    for forbidden in ["unified_log_url", "trace_upload_config", "resolve_trace_bucket_url"] {
        assert!(!source.contains(forbidden), "feedback diagnostic attachment survived: {forbidden}");
    }
    assert!(source.contains("submit_feedback_workflow"));
}

#[test]
fn explicit_share_keeps_session_export_but_has_no_trace_metadata_upload() {
    let source = include_str!("share.rs");
    for forbidden in ["get_trace_context", "upload_session_metadata", "SessionMetadataType"] {
        assert!(!source.contains(forbidden), "share trace attachment survived: {forbidden}");
    }
    assert!(source.contains("ExportedSession::from_local_session"));
    assert!(source.contains("share_session"));
}
```

Run: `cargo test -p xai-grok-shell extensions::feedback -- --nocapture && cargo test -p xai-grok-shell extensions::share -- --nocapture`

Expected: PASS; explicit submission/share behavior remains, and neither path automatically adds diagnostic data.

- [ ] **Step 6: Verify trace APIs are absent and product storage still compiles**

Run: `cargo test -p xai-file-utils --test no_trace_upload_api && cargo test -p xai-file-utils && cargo check -p xai-grok-shell`

Expected: PASS with no prohibited trace-upload API and with retained feedback/share/remote-storage consumers compiling.

- [ ] **Step 7: Commit**

```bash
git add crates/codegen/xai-file-utils crates/codegen/xai-grok-shell/Cargo.toml crates/codegen/xai-grok-shell/src/extensions/feedback.rs crates/codegen/xai-grok-shell/src/extensions/share.rs Cargo.lock
git commit -m "refactor: remove trace upload storage APIs"
```

## Task 9: Prove Retained Local Diagnostics Still Work

**Files:**
- Create: `crates/codegen/xai-grok-pager/tests/pty_e2e/local_diagnostics_survive_telemetry_removal.rs`
- Modify: `crates/codegen/xai-grok-pager/tests/pty_e2e.rs`
- Test: `crates/codegen/xai-grok-telemetry/src/debug_log.rs`
- Test: `crates/codegen/xai-grok-telemetry/src/hooks_log.rs`
- Test: `crates/codegen/xai-grok-telemetry/src/memory_log.rs`
- Test: `crates/codegen/xai-grok-telemetry/src/sampling_log.rs`
- Test: `crates/codegen/xai-grok-telemetry/src/unified_log.rs`

- [ ] **Step 1: Add a failing PTY local-diagnostics test**

Register the new module in `pty_e2e.rs`, following the existing one-file-per-test convention:

```rust
mod local_diagnostics_survive_telemetry_removal;
```

Before writing the ignored PTY body, add this non-ignored source contract in the same file:

```rust
#[test]
fn pager_installs_the_named_local_diagnostics_stack() {
    let source = include_str!("../../src/tracing.rs");
    assert!(source.contains("install_local_diagnostics"));
}
```

Use the same isolated `HOME`, `GROK_HOME`, fixture server, and process helpers as `ctrl_c_cancel_during_stream_recovers_cleanly.rs`. The test must launch with `--log-sampling`, execute one prompt and one local tool/hook path, exit cleanly, then assert:

```rust
assert!(grok_home.join("logs").exists());
assert!(read_matching_log(&grok_home, "unified").contains("session"));
assert!(read_matching_log(&grok_home, "debug").contains("session"));
assert!(read_matching_log(&grok_home, "sampling").contains("request"));
assert!(read_matching_log(&grok_home, "hooks").contains("hook"));
```

Implement `read_matching_log` in that test file by iterating `std::fs::read_dir(grok_home.join("logs"))`, selecting filenames containing the requested stem, sorting paths, and concatenating `read_to_string` results. This is deterministic and does not assume date-derived filenames.

- [ ] **Step 2: Run the PTY test and verify RED**

Run: `cargo test -p xai-grok-pager --test pty_e2e local_diagnostics_survive_telemetry_removal::pager_installs_the_named_local_diagnostics_stack -- --exact --nocapture`

Expected: FAIL in `pager_installs_the_named_local_diagnostics_stack` because `xai-grok-pager/src/tracing.rs` does not yet define or call `install_local_diagnostics`.

- [ ] **Step 3: Wire only the missing local layer or fixture event**

Extract and use an `install_local_diagnostics` helper in `xai-grok-pager/src/tracing.rs`; keep the production local registry exactly as:

```rust
let registry = tracing_subscriber::registry()
    .with(fmt_layer.with_filter(env_filter))
    .with(xai_grok_telemetry::instrumentation::layer())
    .with(xai_grok_telemetry::sampling_log::layer())
    .with(xai_grok_telemetry::hooks_log::layer());
xai_grok_telemetry::debug_log::install_firehose(registry, app_entrypoint);
```

The helper takes the existing filtered fmt layer and `app_entrypoint`, installs the four local layers above, and returns no guard because `debug_log` owns its local writer guards. Use the same layer list in `xai-grok-pager-bin/src/main.rs`.

If the hooks fixture does not naturally execute a hook, configure the existing test hook command to print `local-hook-ran`; do not add a telemetry event or network sink. If memory logging is feature-gated, cover it with its existing unit tests rather than forcing it into this PTY scenario.

- [ ] **Step 4: Run retained local diagnostic suites**

Run: `cargo test -p xai-grok-pager --test pty_e2e local_diagnostics_survive_telemetry_removal::pager_installs_the_named_local_diagnostics_stack -- --exact && cargo test -p xai-grok-pager --test pty_e2e local_diagnostics_survive_telemetry_removal::local_diagnostics_survive_telemetry_removal -- --ignored --exact --nocapture && cargo test -p xai-grok-telemetry debug_log && cargo test -p xai-grok-telemetry hooks_log && cargo test -p xai-grok-telemetry memory_log && cargo test -p xai-grok-telemetry sampling_log && cargo test -p xai-grok-telemetry unified_log`

Expected: all commands PASS; files are written locally and no event payload is copied into them by the compatibility event APIs.

- [ ] **Step 5: Commit**

```bash
git add crates/codegen/xai-grok-pager/tests/pty_e2e.rs crates/codegen/xai-grok-pager/tests/pty_e2e/local_diagnostics_survive_telemetry_removal.rs crates/codegen/xai-grok-pager/src/tracing.rs crates/codegen/xai-grok-pager-bin/src/main.rs
git commit -m "test: preserve local diagnostics without telemetry"
```

## Task 10: Prune Manifests, Scan Sources, and Smoke-Test Outbound Networking

**Files:**
- Create: `bin/check-no-network-telemetry.sh`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/codegen/xai-grok-telemetry/Cargo.toml`
- Modify: `crates/common/xai-tracing/Cargo.toml`
- Modify: `crates/common/xai-computer-hub-sdk/Cargo.toml`
- Modify: `crates/codegen/xai-file-utils/Cargo.toml`
- Modify: any affected crate manifest reported by `cargo machete`

- [ ] **Step 1: Add the failing denylist and dependency scan**

Create executable `bin/check-no-network-telemetry.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

forbidden_source='api\.mixpanel\.com|GROK_TELEMETRY_(EVENTS|MIXPANEL|TRACE_UPLOAD)|GROK_EXTERNAL_OTEL|GROK_INTERNAL_OTLP|GROK_WORKSPACE_OTLP_ENDPOINT|SENTRY_DSN|traces\.donate|logs\.donate|metrics\.donate|trace_upload_(url|bucket|region|credentials|endpoint)|spawn_trace_upload|upload_session_metadata'
if rg -n --glob '*.rs' --glob '*.toml' "$forbidden_source" crates Cargo.toml; then
  echo "network telemetry source survived" >&2
  exit 1
fi

forbidden_manifest='(^|[[:space:]])(sentry|xai-mixpanel|opentelemetry-otlp|opentelemetry-http|opentelemetry-proto|tracing-opentelemetry|fastrace-opentelemetry)[[:space:]]*='
if rg -n --glob 'Cargo.toml' "$forbidden_manifest" Cargo.toml crates; then
  echo "network telemetry manifest entry survived" >&2
  exit 1
fi

forbidden_deps='^(sentry|xai-mixpanel|opentelemetry-otlp|opentelemetry-http|opentelemetry-proto|tracing-opentelemetry|fastrace-opentelemetry) '
if cargo tree --workspace --prefix none | rg "$forbidden_deps"; then
  echo "network telemetry dependency survived" >&2
  exit 1
fi

# Product networking intentionally retained by the design.
rg -q 'xai-grok-update' Cargo.toml
rg -q 'xai-grok-announcements' Cargo.toml
```

Make it executable with `chmod +x bin/check-no-network-telemetry.sh`.

- [ ] **Step 2: Run the scan and verify RED**

Run: `bin/check-no-network-telemetry.sh`

Expected: FAIL with `network telemetry manifest entry survived`; root `Cargo.toml` still declares `opentelemetry-otlp` before final manifest cleanup.

- [ ] **Step 3: Remove stale dependencies and regenerate the lockfile**

Remove root workspace dependency declarations only when `cargo tree -i <crate>` reports no remaining consumer:

```bash
cargo tree -i opentelemetry-otlp
cargo tree -i opentelemetry-http
cargo tree -i opentelemetry-proto
cargo tree -i tracing-opentelemetry
cargo tree -i fastrace-opentelemetry
```

Delete each unused declaration from root `Cargo.toml` and each leaf manifest. Run:

```bash
cargo check --workspace --all-targets
cargo machete
```

For every `cargo machete` unused report in an edited crate, remove that exact dependency and rerun both commands. Accept `Cargo.lock` changes produced by Cargo. Do not remove dependencies used by inference, OAuth, updates, announcements, managed configuration, explicit feedback/share, or remote workspace product features.

- [ ] **Step 4: Run the source/dependency scan and outbound smoke check**

First run: `bin/check-no-network-telemetry.sh`

Expected: PASS with no denylist output.

Build the pager, then run a startup/exit smoke test in a network namespace that permits loopback fixture traffic but rejects external egress:

```bash
cargo build -p xai-grok-pager-bin
tmp_home="$(mktemp -d)"
unshare --user --map-root-user --net sh -c '
  ip link set lo up
  export HOME="$1" GROK_HOME="$1/.grok"
  export SENTRY_DSN="https://public@example.invalid/1"
  export GROK_TELEMETRY_EVENTS_URL="https://events.example.invalid"
  export GROK_TELEMETRY_MIXPANEL_TOKEN="forbidden"
  export GROK_EXTERNAL_OTEL=1
  export OTEL_EXPORTER_OTLP_ENDPOINT="https://otel.example.invalid"
  export GROK_INTERNAL_OTLP_TRACES_ENDPOINT="https://internal-otel.example.invalid/v1/traces"
  export GROK_TRACE_UPLOAD_URL="https://upload.example.invalid"
  timeout 10s "$2" --help >/dev/null
' sh "$tmp_home" target/debug/xai-grok-pager
rm -rf "$tmp_home"
```

Expected: exit status 0. The deliberately populated legacy telemetry environment variables cause no DNS/socket attempt and do not affect startup. This namespace check supplements, rather than replaces, the source/dependency scan because `--help` intentionally does not exercise inference, updates, or announcement fetching.

Then verify the retained networking crates explicitly:

Run: `cargo test -p xai-grok-update && cargo test -p xai-grok-announcements`

Expected: PASS; update and announcement networking remains supported.

- [ ] **Step 5: Run final workspace verification**

Run: `cargo fmt --all -- --check && cargo test -p xai-grok-telemetry -p xai-file-utils -p xai-tracing -p xai-tool-protocol -p xai-computer-hub-sdk -p xai-grok-shell -p xai-grok-workspace -p xai-grok-pager && cargo check --workspace --all-targets`

Expected: all tests and checks PASS. Review `git diff -- Cargo.toml Cargo.lock` and confirm lockfile removals correspond to deleted telemetry/export/upload dependencies.

- [ ] **Step 6: Commit**

```bash
git add bin/check-no-network-telemetry.sh Cargo.toml Cargo.lock crates/codegen/xai-grok-telemetry/Cargo.toml crates/common/xai-tracing/Cargo.toml crates/common/xai-computer-hub-sdk/Cargo.toml crates/codegen/xai-file-utils/Cargo.toml
git commit -m "build: remove network telemetry dependencies"
```

## Final Review Checklist

- [ ] `xai-grok-telemetry` event APIs do not serialize, clone, queue, log, or transmit payloads.
- [ ] `TelemetryMode` cannot represent or deserialize into an enabled state.
- [ ] Mixpanel/product events, Sentry, internal OTLP, external OTLP, Hub donation, and trace upload implementations are absent.
- [ ] No trace correlation header is added to inference or service requests.
- [ ] Unified, debug, memory, hooks, sampling, prompt timing, instrumentation, and local crash formatting remain functional.
- [ ] Root and leaf manifests contain no exporter/reporting dependencies; `Cargo.lock` agrees.
- [ ] Update checks, announcements, and changelog fetching remain unchanged.
- [ ] No provider/catalog/Janus/model-picker files were modified.
- [ ] Source/dependency scan, isolated outbound smoke check, focused tests, and workspace check all pass.
