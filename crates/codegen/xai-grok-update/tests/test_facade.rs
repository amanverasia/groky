//! Facade contract: the updater performs no update, reports none available,
//! and returns inert values. Network absence is enforced structurally (no
//! reqwest dep, no endpoint strings — see bin/check-no-network-telemetry.sh).

use xai_grok_update::UpdateConfig;
use xai_grok_update::auto_update::{self, UpdateRunMode};

fn test_config() -> UpdateConfig {
    UpdateConfig {
        proxy_base_url: "http://127.0.0.1:9".to_string(),
        auth_scope: "test".to_string(),
        deployment_key: None,
        alpha_test_key: None,
        channel: "stable".to_string(),
        npm_registry: None,
    }
}

#[tokio::test]
async fn run_update_if_available_reports_no_update() {
    let cfg = test_config();
    let updated = auto_update::run_update_if_available(UpdateRunMode::NonBlocking, false, &cfg)
        .await
        .unwrap();
    assert!(!updated, "facade must never report an update was performed");
}

#[tokio::test]
async fn check_update_background_is_empty() {
    let cfg = test_config();
    let check = auto_update::check_update_background(&cfg).await;
    assert!(check.update.is_none());
    assert!(check.download.is_none());
}

#[tokio::test]
async fn ensure_latest_on_disk_is_inert() {
    let cfg = test_config();
    let outcome = auto_update::ensure_latest_on_disk(&cfg).await.unwrap();
    assert!(outcome.installed.is_none());
    assert!(!outcome.relaunch_needed);
}

#[tokio::test]
async fn check_update_status_reports_no_update_and_keeps_json_contract() {
    let cfg = test_config();
    let status = auto_update::check_update_status(&cfg).await;
    assert!(!status.update_available);
    assert!(status.error.is_none());
    assert_eq!(status.channel, "stable");
    // `groky update --check --json` wire contract: camelCase field names.
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("\"currentVersion\""));
    assert!(json.contains("\"updateAvailable\":false"));
}

#[tokio::test]
async fn run_update_installs_nothing() {
    let mut cfg = test_config();
    let installed = auto_update::run_update(false, None, None, &mut cfg)
        .await
        .unwrap();
    assert!(installed.is_none());
}

#[tokio::test]
async fn apply_channel_switch_updates_memory_only() {
    let mut cfg = test_config();
    auto_update::apply_channel_switch(Some("alpha"), &mut cfg).await;
    assert_eq!(cfg.channel, "alpha");
}

#[tokio::test]
async fn enforce_minimum_version_is_inert() {
    let cfg = test_config();
    // Must return without exiting, fetching, or installing.
    xai_grok_update::enforce_minimum_version_or_exit(&cfg).await;
}
