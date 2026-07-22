use std::time::Duration;

use serde::Deserialize;
use tokio::fs;

use xai_grok_shell::env::GrokBuildEnvironment;
use xai_grok_shell::util::grok_home::grok_home;

const TTL_SECONDS_BEFORE_AUTO_UPDATE: Duration = Duration::from_secs(60 * 30);

/// Minimal configuration the update system needs from the environment.
///
/// Constructed once from `GrokBuildEnvironment` at startup and threaded through the
/// update call chain so that `auto_update` and `version` never need to know
/// about the `GrokBuildEnvironment` enum directly.
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// Chat API proxy base URL (no longer used — kept for API compatibility with callers).
    pub proxy_base_url: String,
    /// Auth scope key for `~/.grok/auth.json`.
    pub auth_scope: String,
    /// Enterprise deployment key (GROK_DEPLOYMENT_KEY).
    pub deployment_key: Option<String>,
    /// Optional extra auth material forwarded with requests when present.
    pub alpha_test_key: Option<String>,
    /// Release channel: "stable" or "alpha". Loaded from config.
    pub channel: String,
    /// Custom npm registry URL. When set, passed as `--registry=` to npm CLI.
    pub npm_registry: Option<String>,
}

impl UpdateConfig {
    pub fn from_environment(env: &GrokBuildEnvironment) -> Self {
        Self {
            proxy_base_url: env.cli_chat_proxy_base_url(),
            auth_scope: xai_grok_shell::auth::GrokComConfig::default().auth_scope(),
            deployment_key: None,
            alpha_test_key: None,
            channel: "stable".to_string(),
            npm_registry: None,
        }
    }
}

#[derive(Debug, serde::Serialize, Deserialize)]
struct GrokVersion {
    version: String,
    #[serde(default)]
    stable_version: Option<String>,
    checked_at: String,
}

impl GrokVersion {
    fn is_fresh(&self, now: time::OffsetDateTime, ttl: Duration) -> bool {
        if let Ok(dt) = time::OffsetDateTime::parse(
            &self.checked_at,
            &time::format_description::well_known::Rfc3339,
        ) && dt <= now
        {
            // Clock-skew guard: future timestamps are never fresh.
            now - dt < ttl
        } else {
            false
        }
    }

    fn new(version: String, stable_version: Option<String>, now: time::OffsetDateTime) -> Self {
        let checked_at = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| now.to_string());
        Self {
            version,
            stable_version,
            checked_at,
        }
    }
}

/// Write the version cache to disk, recording that `version` was seen at the
/// current time. Call after confirming the version is current (no update
/// needed) or after a successful install.
///
/// `stable_version` records the current stable channel pointer so that
/// `channel_label()` can derive `[alpha]` vs `[stable]` without network I/O.
pub async fn write_version_cache(version: &str, stable_version: Option<&str>) {
    let version_path = grok_home().join("version.json");
    let now = time::OffsetDateTime::now_utc();
    let json = GrokVersion::new(
        version.to_string(),
        stable_version.map(|s| s.to_string()),
        now,
    );
    if let Some(dir) = version_path.parent()
        && let Err(e) = fs::create_dir_all(dir).await
    {
        tracing::warn!("failed to create version cache directory: {}", e);
        return;
    }
    let tmp = version_path.with_extension("json.tmp");
    let data = match serde_json::to_vec_pretty(&json) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("failed to serialize version cache: {}", e);
            return;
        }
    };
    if let Err(e) = fs::write(&tmp, data).await {
        tracing::warn!("failed to write version cache tmp file: {}", e);
        return;
    }
    if let Err(e) = fs::rename(&tmp, &version_path).await {
        tracing::warn!("failed to rename version cache file: {}", e);
    }
}

/// True if `version.json` exists and is within TTL.
pub async fn is_version_cache_fresh() -> bool {
    let version_path = grok_home().join("version.json");
    let now = time::OffsetDateTime::now_utc();
    if let Ok(version_str) = fs::read_to_string(&version_path).await
        && let Ok(version) = serde_json::from_str::<GrokVersion>(&version_str)
        && version.is_fresh(now, TTL_SECONDS_BEFORE_AUTO_UPDATE)
    {
        return true;
    }
    false
}

pub use xai_grok_version::installed as get_installed_grok_version;

/// Return the running binary's version as read off the disk (the versioned
/// binary name behind the `~/.grok/bin/groky` symlink).
///
/// Returns `None` when there is no managed install on disk — e.g. the user
/// installed the raw binary or compiled from source. The version cache
/// (`write_version_cache`) is the only persistent source then, so callers must
/// not assume the absence of a symlink means "no version data" and should fall
/// back to the cache or compile-time VERSION.
///
/// On Unix, `grok_application()` resolves to `~/.grok/bin/groky` (or a
/// configured `groky_path_exec`). We readlink it, extract the target filename
/// (e.g. `groky-0.1.150-linux-x64`), parse out the version portion with
/// `version_from_versioned_binary_name`, and then **verify the target exists**
/// via metadata() — a dangling link means the binary was deleted and must not
/// claim "already up to date" forever while no runnable binary exists.
/// NOTE: the symlink existing does not prove the *active installer*
/// maintains it — npm manages its own global install and a leftover symlink
/// from a previous internal install would lie about the npm install's
/// version. Callers must gate on the installer (see
/// `disk_version_for_installer` in `auto_update`).
pub fn installed_on_disk_version() -> Option<String> {
    #[cfg(unix)]
    {
        let app = xai_grok_shell::util::grok_home::grok_application();
        let target = std::fs::read_link(&app).ok()?;
        // metadata() follows the symlink: Err means the target is gone
        // (dangling link) and the version it names is not actually on disk.
        std::fs::metadata(&app).ok()?;
        version_from_versioned_binary_name(target.file_name()?.to_str()?, "groky")
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Extract the `<version>` portion of a versioned binary file name.
///
/// Handles the internal layout (`grok-0.1.150-macos-aarch64`, including
/// pre-releases: `grok-0.1.150-alpha.1-linux-x86_64` → `0.1.150-alpha.1`)
/// and the npm layout without a platform suffix (`grok-0.1.150`,
/// `grok-0.1.150-alpha.1`): everything between the `{bin_prefix}-` prefix
/// and the first platform-OS component is the version, validated as semver
/// so unknown layouts (`grok-latest`, `grok-pager-*` when `bin_prefix` is
/// `grok`) return `None` instead of garbage.
///
/// Shared by the disk-version probe above and `cleanup_old_downloads` in
/// `auto_update` — keep it the single place that understands this naming.
pub(crate) fn version_from_versioned_binary_name(name: &str, bin_prefix: &str) -> Option<String> {
    const PLATFORM_OS: &[&str] = &["macos", "linux", "darwin", "windows"];
    let suffix = name.strip_prefix(bin_prefix)?.strip_prefix('-')?;
    let parts: Vec<&str> = suffix.split('-').collect();
    let platform_start = parts
        .iter()
        .position(|p| PLATFORM_OS.contains(p))
        .unwrap_or(parts.len());
    let ver_str = parts[..platform_start].join("-");
    semver::Version::parse(&ver_str).ok()?;
    Some(ver_str)
}

/// Read the cached stable version from `~/.grok/version.json` (sync, for display).
///
/// Returns `None` if the file doesn't exist, can't be parsed, or has no
/// `stable_version` field (e.g. written by an older binary).
pub fn cached_stable_version() -> Option<String> {
    let version_path = grok_home().join("version.json");
    let content = std::fs::read_to_string(&version_path).ok()?;
    let gv: GrokVersion = serde_json::from_str(&content).ok()?;
    gv.stable_version
}

/// Pure comparison: derive the channel name from current vs stable pointer.
///
/// Returns `Some("alpha")` when `current > stable`, `Some("stable")` when
/// `current <= stable`, or `None` when either version fails to parse.
fn derive_channel<'a>(current: &str, stable: &str) -> Option<&'a str> {
    let current_v = semver::Version::parse(current).ok()?;
    let stable_v = semver::Version::parse(stable).ok()?;
    if current_v > stable_v {
        Some("alpha")
    } else {
        Some("stable")
    }
}

/// Machine-readable channel name derived from the cached stable pointer.
///
/// Returns `Some("alpha")` when the current version is ahead of the cached
/// stable pointer, `Some("stable")` when at or behind, or `None` when no
/// cached pointer is available (first launch, old cache format, parse error).
///
/// The result is computed once and cached for the process lifetime.
pub fn channel_name() -> Option<&'static str> {
    use std::sync::OnceLock;
    static NAME: OnceLock<Option<&'static str>> = OnceLock::new();
    *NAME.get_or_init(|| {
        let stable = cached_stable_version()?;
        derive_channel(xai_grok_version::VERSION, &stable)
    })
}

/// Channel label derived from the cached stable pointer.
///
/// Compares the compiled-in `VERSION` against the stable pointer stored in
/// `~/.grok/version.json` (written by a previous upstream-grok auto-updater
/// run, if any):
/// - `" [alpha]"` when the current version is ahead of stable,
/// - `" [stable]"` when at or behind stable,
/// - `""` when no cached pointer is available (first launch, old cache format).
///
/// The result is computed once and cached for the process lifetime.
pub fn channel_label() -> &'static str {
    use std::sync::OnceLock;
    static LABEL: OnceLock<&'static str> = OnceLock::new();
    LABEL.get_or_init(|| {
        let stable = match cached_stable_version() {
            Some(s) => s,
            None => return "",
        };
        match derive_channel(xai_grok_version::VERSION, &stable) {
            Some("alpha") => " [alpha]",
            Some(_) => " [stable]",
            None => "",
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that a future `checked_at` timestamp (e.g. from clock skew or
    /// NTP time-warp) is never considered fresh. Without the clock-skew guard
    /// this would return true indefinitely, silently disabling auto-update.
    #[test]
    fn test_is_fresh_rejects_future_timestamp() {
        let now = time::OffsetDateTime::now_utc();
        let future = now + Duration::from_secs(600);
        let v = GrokVersion::new("0.1.200".to_string(), None, future);
        assert!(
            !v.is_fresh(now, Duration::from_secs(30)),
            "Future timestamp must not be considered fresh (clock-skew guard)."
        );
    }

    /// Disk-version probe: parsing the version out of the managed install's
    /// symlink-target file name (`grok-<version>-<platform>`).
    #[test]
    fn test_version_from_versioned_binary_name() {
        let cases: &[(&str, Option<&str>)] = &[
            ("grok-0.2.46-darwin-arm64", Some("0.2.46")),
            ("grok-0.1.220-linux-x86_64", Some("0.1.220")),
            ("grok-0.2.5-windows-x86_64.exe", Some("0.2.5")),
            // Pre-releases must round-trip whole — truncating to "0.1.220"
            // would make an alpha install masquerade as the release and
            // mask alpha → stable updates.
            ("grok-0.1.220-alpha.4-linux-x86_64", Some("0.1.220-alpha.4")),
            ("grok-0.1.220-alpha.4", Some("0.1.220-alpha.4")), // npm layout
            ("grok-pager-0.1.5-darwin-arm64", None),           // "pager" is not a version
            ("grok-garbage-darwin-arm64", None),               // unparseable version
            ("grok-0.2.46", Some("0.2.46")),                   // no platform suffix
            ("other-0.2.46-darwin-arm64", None),               // wrong prefix
            ("grok-latest", None),                             // symlink alias, not a version
            ("grok", None),                                    // bare name
            ("", None),
        ];
        for (name, expected) in cases {
            assert_eq!(
                version_from_versioned_binary_name(name, "grok").as_deref(),
                *expected,
                "version_from_versioned_binary_name({name:?})"
            );
        }

        // bin_prefix discrimination: the pager binary parses under its own
        // prefix but not under "grok".
        assert_eq!(
            version_from_versioned_binary_name("grok-pager-0.1.5-darwin-arm64", "grok-pager")
                .as_deref(),
            Some("0.1.5")
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // derive_channel — invariant matrix
    //
    // Tests the pure comparison logic that determines [alpha] vs [stable].
    // Covers current 0.1.X-alpha.N, future 0.2.X, edge cases, and errors.
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_derive_channel_matrix() {
        // (current, stable_pointer, expected_channel)
        let cases: &[(&str, &str, Option<&str>)] = &[
            // ── Current 0.1.X workflow ──
            ("0.1.220-alpha.2", "0.1.219", Some("alpha")), // alpha ahead of stable
            ("0.1.219", "0.1.219", Some("stable")),        // stable user on latest
            ("0.1.218", "0.1.219", Some("stable")),        // stable user behind latest
            ("0.1.220-alpha.2", "0.1.220-alpha.2", Some("stable")), // pointer matches exactly
            ("0.1.220-alpha.2", "0.1.220", Some("stable")), // semver: release > pre-release
            // ── Future 0.2.X workflow ──
            ("0.2.5", "0.2.3", Some("alpha")), // alpha ahead of stable
            ("0.2.5", "0.2.5", Some("stable")), // promoted to stable
            ("0.2.3", "0.2.5", Some("stable")), // behind stable
            ("0.2.0", "0.2.0", Some("stable")), // first release, both 0.2.0
            // ── Cross-regime upgrade ──
            ("0.2.0", "0.1.219", Some("alpha")), // new regime ahead of old stable
            ("0.1.220-alpha.2", "0.2.0", Some("stable")), // old pre-release < new stable
            // ── Error cases ──
            ("garbage", "0.1.219", None), // unparseable current
            ("0.1.219", "garbage", None), // unparseable stable
            ("", "0.1.219", None),        // empty current
            ("0.1.219", "", None),        // empty stable
        ];

        for (current, stable, expected) in cases {
            let result = derive_channel(current, stable);
            assert_eq!(
                result, *expected,
                "derive_channel({:?}, {:?}) = {:?}, expected {:?}",
                current, stable, result, expected,
            );
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // GrokVersion JSON shape — backward compatibility invariants
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_version_json_backward_compat() {
        // Old format (no stable_version) must parse — serde(default) fills None.
        let old = r#"{"version":"0.1.180","checked_at":"2026-04-22T10:30:00Z"}"#;
        let v: GrokVersion = serde_json::from_str(old).unwrap();
        assert_eq!(v.version, "0.1.180");
        assert!(v.stable_version.is_none());

        // New format: all fields populated.
        let new = r#"{"version":"0.1.220-alpha.2","stable_version":"0.1.219","checked_at":"2026-04-22T10:30:00Z"}"#;
        let parsed: GrokVersion = serde_json::from_str(new).unwrap();
        assert_eq!(parsed.version, "0.1.220-alpha.2");
        assert_eq!(parsed.stable_version.as_deref(), Some("0.1.219"));
        assert!(
            time::OffsetDateTime::parse(
                &parsed.checked_at,
                &time::format_description::well_known::Rfc3339,
            )
            .is_ok()
        );

        // Unknown fields are ignored (forward-compat).
        let future = r#"{"version":"0.1.180","checked_at":"2026-04-22T10:30:00Z","future":"ok"}"#;
        assert!(serde_json::from_str::<GrokVersion>(future).is_ok());

        // Missing required field (checked_at) is rejected.
        let missing = r#"{"version":"0.1.180"}"#;
        assert!(serde_json::from_str::<GrokVersion>(missing).is_err());
    }

    // ──────────────────────────────────────────────────────────────────────
    // is_fresh — TTL boundary invariants
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_is_fresh_ttl_boundaries() {
        let now = time::OffsetDateTime::now_utc();
        let v = GrokVersion::new("0.1.200".to_string(), None, now);

        // Within TTL → fresh
        assert!(v.is_fresh(now, Duration::from_secs(60)));
        assert!(v.is_fresh(now + Duration::from_secs(29), Duration::from_secs(30)));

        // At TTL boundary → NOT fresh (strict <)
        assert!(!v.is_fresh(now + Duration::from_secs(30), Duration::from_secs(30)));

        // Past TTL → not fresh
        assert!(!v.is_fresh(now + Duration::from_secs(31), Duration::from_secs(30)));

        // Zero TTL → never fresh
        assert!(!v.is_fresh(now, Duration::ZERO));

        // Malformed timestamp → not fresh
        let bad = GrokVersion {
            version: "0.1.200".to_string(),
            stable_version: None,
            checked_at: "not-rfc3339".to_string(),
        };
        assert!(!bad.is_fresh(now, Duration::from_secs(60)));
    }

    // ──────────────────────────────────────────────────────────────────────
    // UpdateConfig defaults
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_update_config_default_channel_is_stable() {
        use xai_grok_shell::env::GrokBuildEnvironment;
        let cfg = UpdateConfig::from_environment(&GrokBuildEnvironment::Production);
        assert_eq!(cfg.channel, "stable");
    }
}
