use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use xai_grok_telemetry::TelemetryEvent;
use xai_grok_telemetry::client;
use xai_grok_telemetry::config::{TelemetryConfig, TelemetryMode};
use xai_grok_telemetry::session_ctx::{
    EmitterOrigin, emit_event, emit_event_with_origin, log_event, log_event_dual,
    log_session_event, log_session_event_with_origin,
};

struct SerializationProbe {
    serialized: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl Serialize for SerializationProbe {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.serialized.fetch_add(1, Ordering::SeqCst);
        Err(serde::ser::Error::custom("probe must not be serialized"))
    }
}

impl Drop for SerializationProbe {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl TelemetryEvent for SerializationProbe {
    const NAME: &'static str = "serialization_probe";
}

fn assert_payload_is_consumed_without_work(emit: impl FnOnce(SerializationProbe)) {
    let serialized = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    emit(SerializationProbe {
        serialized: Arc::clone(&serialized),
        dropped: Arc::clone(&dropped),
    });

    assert_eq!(serialized.load(Ordering::SeqCst), 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn public_event_apis_only_consume_payloads() {
    assert_payload_is_consumed_without_work(log_event);
    assert_payload_is_consumed_without_work(|payload| log_event_dual(true, payload));
    assert_payload_is_consumed_without_work(log_session_event);
    assert_payload_is_consumed_without_work(|payload| {
        log_session_event_with_origin(EmitterOrigin::Workspace, payload)
    });
    assert_payload_is_consumed_without_work(|payload| emit_event("probe", payload));
    assert_payload_is_consumed_without_work(|payload| {
        emit_event_with_origin(EmitterOrigin::Workspace, "probe", payload)
    });
}

#[test]
fn legacy_modes_are_always_disabled() {
    for input in ["true", "false", "enabled", "session_metrics"] {
        assert_eq!(TelemetryMode::parse(input), Some(TelemetryMode::Disabled));
    }

    for input in ["true", "false", "\"enabled\"", "\"session_metrics\""] {
        assert_eq!(
            serde_json::from_str::<TelemetryMode>(input).unwrap(),
            TelemetryMode::Disabled
        );
    }

    let mode = TelemetryMode::Disabled;
    assert!(mode.is_disabled());
    assert!(!mode.is_enabled());
    assert!(!mode.session_metrics_enabled());
}

#[test]
fn client_stays_disabled_after_legacy_enabled_init() {
    let mode = serde_json::from_str::<TelemetryMode>("true").unwrap();
    client::init(
        TelemetryConfig::default(),
        mode,
        None,
        None,
        None,
        None,
        "test".to_owned(),
        None,
        reqwest::Client::new(),
    );

    assert!(!client::is_enabled());
    assert!(!client::is_session_metrics_enabled());
    let _ = client::TelemetryClient::default();
}

#[test]
fn default_config_serializes_without_network_fields() {
    let value = serde_json::to_value(TelemetryConfig::default()).unwrap();
    let object = value.as_object().unwrap();
    for field in [
        "events_url",
        "events_api_key",
        "mixpanel_token",
        "mixpanel_enabled",
        "trace_upload",
        "otel_enabled",
        "otel_metrics_exporter",
        "otel_logs_exporter",
        "otel_endpoint",
        "otel_protocol",
        "otel_transport",
        "otel_log_user_prompts",
        "otel_log_tool_details",
    ] {
        assert!(
            !object.contains_key(field),
            "unexpected network field: {field}"
        );
    }
}
