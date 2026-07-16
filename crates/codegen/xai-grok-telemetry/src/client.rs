//! Local-only compatibility facade for the former telemetry client.

use chrono::{Local, SecondsFormat};

use crate::config::{TelemetryConfig, TelemetryMode};
use crate::http::OriginClientInfo;

/// Event property map shared by all telemetry modules.
pub type Metadata = serde_json::Map<String, serde_json::Value>;

#[derive(Clone, Debug, Default)]
pub struct TelemetryClient;

impl TelemetryClient {
    pub fn from_config(
        _config: TelemetryConfig,
        _mode: TelemetryMode,
        _user_id: Option<String>,
        _team_id: Option<String>,
        _deployment_key: Option<String>,
        _origin_client: Option<OriginClientInfo>,
        _shell_version: String,
        _subscription_tier: Option<String>,
        _http_client: reqwest::Client,
    ) -> Self {
        Self
    }
}

/// Telemetry is permanently disabled.
pub fn is_enabled() -> bool {
    false
}

/// Session metrics are permanently disabled.
pub fn is_session_metrics_enabled() -> bool {
    false
}

pub struct UserContext {
    pub country: String,
    pub language: String,
    pub timestamp: String,
}

impl UserContext {
    pub fn collect() -> Self {
        let default_language = whoami::Language::En(whoami::Country::Any);
        let lang = whoami::langs()
            .ok()
            .and_then(|mut langs| langs.next())
            .unwrap_or(default_language);
        Self {
            country: lang.country().to_string(),
            language: lang.to_string(),
            timestamp: Local::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        }
    }
}

/// Consume an event without emitting it.
pub async fn track(_event_name: &str, _request_id: &str, _ctx: &UserContext, _metadata: Metadata) {}

/// Retained as a no-op for source compatibility.
pub fn sync_profile() {}

/// Retained as a no-op with its existing argument list for source compatibility.
pub fn init(
    _config: TelemetryConfig,
    _mode: TelemetryMode,
    _user_id: Option<String>,
    _team_id: Option<String>,
    _deployment_key: Option<String>,
    _origin_client: Option<OriginClientInfo>,
    _shell_version: String,
    _subscription_tier: Option<String>,
    _http_client: reqwest::Client,
) {
}

/// Retained as a no-op with its existing argument list for source compatibility.
pub fn init_if_needed(
    _config: TelemetryConfig,
    _mode: TelemetryMode,
    _user_id: Option<String>,
    _team_id: Option<String>,
    _deployment_key: Option<String>,
    _origin_client: Option<OriginClientInfo>,
    _shell_version: String,
    _subscription_tier: Option<String>,
    _http_client: reqwest::Client,
) {
}
