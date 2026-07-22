//! Local-only facade: minimum-version enforcement is disabled in groky.
//!
//! The upstream implementation read `cli.minimum_version` from managed
//! config, fetched the latest version, and could auto-install and
//! `std::process::exit` — all of which depend on the (removed) updater
//! networking. groky never phones home, so this is a no-op.

use crate::version::UpdateConfig;

/// No-op. See module docs.
pub async fn enforce_minimum_version_or_exit(_update_config: &UpdateConfig) {}
