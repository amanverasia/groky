//! Telemetry-engine configuration.
//!
//! Extracted from `xai-grok-shell::agent::config` so the data-collector
//! engine can construct a [`TelemetryClient`](crate::client::TelemetryClient)
//! without a build-time dependency on the shell.
//!
//! Shell still re-exports these types from their original paths so existing
//! call sites (and `Config` derive impls) compile unchanged.
use serde::{Deserialize, Serialize};

/// Local-only telemetry mode. Legacy values are accepted as disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TelemetryMode {
    #[default]
    Disabled,
}
impl TelemetryMode {
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
    pub fn is_enabled(&self) -> bool {
        false
    }
    pub fn session_metrics_enabled(&self) -> bool {
        false
    }
    pub fn parse(s: &str) -> Option<Self> {
        if s.trim().is_empty() {
            None
        } else {
            Some(Self::Disabled)
        }
    }
}
impl std::fmt::Display for TelemetryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "false")
    }
}
impl From<bool> for TelemetryMode {
    fn from(_enabled: bool) -> Self {
        Self::Disabled
    }
}
impl serde::Serialize for TelemetryMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(false)
    }
}
/// Wire format for `[features] telemetry`: accepts `true`, `false`, or `"session_metrics"`.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum TelemetryModeValue {
    Bool(bool),
    Str(String),
}
impl<'de> serde::Deserialize<'de> for TelemetryMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match TelemetryModeValue::deserialize(deserializer)? {
            TelemetryModeValue::Bool(value) => {
                let _ = value;
                Ok(Self::Disabled)
            }
            TelemetryModeValue::Str(value) => {
                let _ = value;
                Ok(Self::Disabled)
            }
        }
    }
}
/// Parse an env var as a `TelemetryMode`. Returns `None` if unset or empty.
pub fn env_telemetry_mode(name: &str) -> Option<TelemetryMode> {
    let value = std::env::var(name).ok()?;
    TelemetryMode::parse(&value)
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TelemetryConfig {}
impl TelemetryConfig {
    pub fn apply_env_overrides(&mut self) {}
}
/// Derive a stable deployment ID (UUIDv5) from the deployment key.
pub fn deployment_id_from_key(key: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, key.as_bytes()).to_string()
}
