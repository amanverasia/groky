//! Local-only telemetry facade for Grok Build sessions: event types are kept
//! for API compatibility, and the retained pieces are local-only diagnostics
//! (unified/debug/memory/hooks/sampling logs, instrumentation, prompt timing)
//! plus Sentry error reporting, whose removal is still pending. No OTLP or
//! analytics export remains.
//!
//! Extracted from `xai-file-utils` per review feedback so telemetry has
//! its own ownership boundary (see CODEOWNERS) and so downstream consumers
//! that only want event tracking + inference metrics no longer pull in
//! HTTP/identity dependencies.

mod appender;
pub mod client;
pub mod config;
pub mod context;
pub mod debug_log;
pub mod enums;
pub mod events;
pub mod hooks_log;
pub mod http;
pub mod id;
pub mod instrumentation;
pub mod memory_log;
pub mod memory_telemetry;
pub mod prompt_timing;
pub mod sampling_log;
pub mod sentry;
pub mod session_ctx;
pub mod session_metrics;
pub mod unified_log;

pub use client::{
    Metadata, TelemetryClient, UserContext, init, init_if_needed, is_enabled,
    is_session_metrics_enabled,
};
pub use events::TelemetryEvent;
pub use session_ctx::{
    EmitterOrigin, TelemetryCtx, emit_event, emit_event_with_origin, log_event, log_session_event,
    log_session_event_with_origin, with_session_ctx,
};
