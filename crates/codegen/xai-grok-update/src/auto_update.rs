//! Local-only compatibility facade for the former auto-updater.
//!
//! groky does not phone home. The upstream implementation checked an
//! update endpoint at startup, wrote `cli.auto_update = true` into the
//! user's config on first run, and could download the upstream `grok`
//! binary into `~/.groky/downloads` and swap it into `~/.groky/bin` —
//! silently replacing groky with upstream grok once the channel pointer
//! advanced. This facade keeps the public API so `xai-grok-pager-bin` and
//! `xai-grok-pager` compile unchanged, but performs no network I/O, spawns
//! no processes, mutates no config, and never touches the filesystem.
//!
//! Users update groky by re-running the installer (see README).
//! Guarded by bin/check-no-network-telemetry.sh, which forbids the upstream
//! endpoints and the reqwest dependency from returning to this crate.

use anyhow::Result;

use crate::version::{UpdateConfig, get_installed_grok_version};

#[derive(Clone, Copy, Debug)]
pub enum UpdateRunMode {
    Blocking,
    NonBlocking,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub installer: Option<String>,
    pub channel: String,
    pub auto_update: Option<bool>,
    pub error: Option<String>,
}

/// Format and print an [`UpdateStatus`] to stdout.
pub fn print_update_status(status: &UpdateStatus, json: bool) -> anyhow::Result<()> {
    if json {
        let payload = serde_json::to_string(status)?;
        println!("{payload}");
        return Ok(());
    }

    if let Some(error) = status.error.as_deref() {
        println!("groky - v{} [{}]", status.current_version, status.channel);
        println!("Update check failed: {error}");
        return Ok(());
    }

    let channel_label = format!(" [{}]", status.channel);

    if status.update_available {
        if let Some(latest_version) = status.latest_version.as_deref() {
            println!(
                "A new version of groky is available: {} -> {}{}",
                status.current_version, latest_version, channel_label
            );
        } else {
            println!("A new version of groky is available.");
        }
        return Ok(());
    }

    if let Some(latest_version) = status.latest_version.as_deref() {
        println!(
            "groky - v{} (latest: {}){}",
            status.current_version, latest_version, channel_label
        );
        return Ok(());
    }

    println!("groky - v{}{}", status.current_version, channel_label);
    Ok(())
}

/// Facade: reports the current version with no update available. Never
/// consults an installer, the network, or the config file.
pub async fn check_update_status(update_config: &UpdateConfig) -> UpdateStatus {
    UpdateStatus {
        current_version: get_installed_grok_version(),
        latest_version: None,
        update_available: false,
        installer: None,
        channel: update_config.channel.clone(),
        auto_update: Some(false),
        error: None,
    }
}

#[derive(Debug)]
pub struct EnsureLatestOutcome {
    pub installed: Option<String>,
    pub relaunch_needed: bool,
}

/// Facade: nothing to install, no relaunch ever needed.
pub async fn ensure_latest_on_disk(_update_config: &UpdateConfig) -> Result<EnsureLatestOutcome> {
    Ok(EnsureLatestOutcome {
        installed: None,
        relaunch_needed: false,
    })
}

#[derive(Debug, Clone)]
pub struct UpdateAvailable {
    pub latest_version: String,
}

/// Result of the (disabled) startup background check. The `download` field
/// keeps its upstream type so `main.rs` compiles unchanged; it is always
/// `None` here.
pub struct BackgroundUpdateCheck {
    pub update: Option<UpdateAvailable>,
    pub download: Option<tokio::process::Child>,
}

/// Facade: no background check is performed; nothing is ever available.
pub async fn check_update_background(_update_config: &UpdateConfig) -> BackgroundUpdateCheck {
    BackgroundUpdateCheck {
        update: None,
        download: None,
    }
}

/// Facade: never updates, never writes `cli.auto_update` into the config
/// (the upstream version force-enabled auto-update on first run here).
pub async fn run_update_if_available(
    _run_mode: UpdateRunMode,
    _interactive: bool,
    _update_config: &UpdateConfig,
) -> Result<bool> {
    Ok(false)
}

/// Facade: keeps the in-memory channel coherent for `--check` output but
/// never persists it to the config file.
pub async fn apply_channel_switch(channel_switch: Option<&str>, update_config: &mut UpdateConfig) {
    if let Some(ch) = channel_switch {
        update_config.channel = ch.to_string();
    }
}

/// Facade for `groky update`: explains how groky is actually updated.
pub async fn run_update(
    _force: bool,
    _pinned_version: Option<&str>,
    _channel_switch: Option<&str>,
    _update_config: &mut UpdateConfig,
) -> Result<Option<String>> {
    println!("groky does not auto-update or check for updates.");
    println!("To update, re-run the installer:");
    println!(
        "  curl -fsSL https://raw.githubusercontent.com/amanverasia/groky/main/install.sh | bash"
    );
    Ok(None)
}
