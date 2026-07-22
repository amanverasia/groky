//! Filesystem locations for grok config files and binaries.

use std::path::PathBuf;
use std::sync::OnceLock;

static GROK_HOME: OnceLock<PathBuf> = OnceLock::new();

#[cfg(target_os = "macos")]
const CLAUDE_MANAGED_SETTINGS_PATH: &str =
    "/Library/Application Support/ClaudeCode/managed-settings.json";
#[cfg(target_os = "linux")]
const CLAUDE_MANAGED_SETTINGS_PATH: &str = "/etc/claude-code/managed-settings.json";

/// The user's home directory, canonicalized.
///
/// Uses [`dunce::canonicalize`] instead of [`std::fs::canonicalize`]: on
/// Windows, std returns a verbatim path (`\\?\C:\Users\...`) which external
/// tools choke on — e.g. `git clone` rejects `\\?\` destinations with
/// "Invalid argument", breaking marketplace cache clones under
/// `~/.groky/marketplace-cache`. `dunce` strips the prefix whenever the path
/// is safely representable in legacy form; on non-Windows it is identical to
/// `std::fs::canonicalize`.
fn canonical_user_home() -> PathBuf {
    #[allow(deprecated)]
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    dunce::canonicalize(&home).unwrap_or(home)
}

/// The default user grok directory (`~/.groky`, canonicalized) used when
/// neither `GROKY_HOME` nor `GROK_HOME` is set. Exposed so callers (e.g.
/// display helpers) can detect whether [`grok_home()`] is the default without
/// duplicating the computation.
///
/// Keep the dunce canonicalization in sync with the hand-rolled duplicate in
/// `xai_fast_worktree::db::resolve_grok_home` (deliberately standalone crate).
pub fn default_grok_home() -> PathBuf {
    canonical_user_home().join(".groky")
}

/// Pure grok-home resolution: `GROKY_HOME` → `GROK_HOME` (legacy) →
/// `home_dir/.groky`. No filesystem side effects.
///
/// Building block of [`grok_home()`], exposed (with explicit env values)
/// because the memoized `grok_home()` cannot be exercised repeatedly in one
/// test process.
pub fn resolve_grok_home_with(
    env_groky: Option<&str>,
    env_grok: Option<&str>,
    home_dir: &std::path::Path,
) -> PathBuf {
    if let Some(v) = env_groky {
        return PathBuf::from(v);
    }
    if let Some(v) = env_grok {
        return PathBuf::from(v);
    }
    home_dir.join(".groky")
}

/// One-time migration of a legacy `~/.grok` tree to the new `~/.groky` home.
///
/// Recursively copies `legacy` into a sibling staging dir
/// (`~/.groky.migrating-<pid>`) preserving file permissions (on Unix
/// `std::fs::copy` copies mode bits, so `auth.json` stays `0600`), then
/// atomically renames it to `new` on success. Symlinks and other
/// non-file/non-dir entries are skipped with a warning. The legacy tree is
/// never modified or deleted.
///
/// Returns `Ok(true)` when a migration ran, `Ok(false)` when it was skipped
/// (`new` already exists, or `legacy` is not a directory), and `Err` on copy
/// failure. On failure the staging dir is removed and `new` is left absent,
/// so a partial copy can never masquerade as a completed migration or block
/// a later retry; callers fall back to creating `new` fresh rather than
/// crashing.
pub fn migrate_legacy_home(
    legacy: &std::path::Path,
    new: &std::path::Path,
) -> std::io::Result<bool> {
    if new.exists() || !legacy.is_dir() {
        return Ok(false);
    }
    let staging_name = format!(
        "{}.migrating-{}",
        new.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "grok-home".to_string()),
        std::process::id()
    );
    let staging = new
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(staging_name);
    // A crashed earlier attempt by this pid (recycled) may have left the
    // staging dir behind; start clean.
    let _ = std::fs::remove_dir_all(&staging);
    let copied = copy_dir_recursive(legacy, &staging)
        // Rename is atomic: `new` appears fully-populated or not at all.
        .and_then(|()| std::fs::rename(&staging, new));
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    Ok(true)
}

/// Recursively copy `src` dir to `dst`, preserving permissions; skip
/// symlinks and other special entries with a warning.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if file_type.is_file() {
            // std::fs::copy preserves permission bits on Unix, so 0600
            // files (auth.json) stay 0600.
            std::fs::copy(&from, &to)?;
        } else {
            tracing::warn!(
                path = %from.display(),
                "skipping non-regular file during grok home migration"
            );
        }
    }
    // Dir mode is applied *after* populating so a read-only source dir mode
    // can't lock us (or failure cleanup) out of the freshly-created copy.
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(src) {
        let _ = std::fs::set_permissions(dst, meta.permissions());
    }
    Ok(())
}

/// Non-memoized core of [`grok_home()`]: resolve the home from the given env
/// values, run the one-time legacy migration when (and only when) the default
/// path is in play, and create the directory. Exposed for tests; production
/// code should call [`grok_home()`].
pub fn init_grok_home_with(
    env_groky: Option<&str>,
    env_grok: Option<&str>,
    home_dir: &std::path::Path,
) -> PathBuf {
    let resolved = resolve_grok_home_with(env_groky, env_grok, home_dir);
    // Migration only ever runs on the default path: an explicit env override
    // means the user opted out of `~/.groky` and we must not touch it.
    if env_groky.is_none() && env_grok.is_none() {
        let legacy = home_dir.join(".grok");
        match migrate_legacy_home(&legacy, &resolved) {
            Ok(true) => tracing::info!(
                from = %legacy.display(),
                to = %resolved.display(),
                "migrated legacy grok home to ~/.groky (original left untouched)"
            ),
            Ok(false) => {}
            Err(error) => tracing::warn!(
                from = %legacy.display(),
                to = %resolved.display(),
                %error,
                "failed to migrate legacy grok home; continuing with a fresh ~/.groky"
            ),
        }
    }
    let _ = std::fs::create_dir_all(&resolved);
    resolved
}

/// Per-user config directory: `$GROKY_HOME`, `$GROK_HOME` (legacy), or
/// `~/.groky`. Created if needed. On first default-path resolution, a legacy
/// `~/.grok` tree is copied to `~/.groky` (one-time migration; the legacy
/// tree is never modified).
pub fn grok_home() -> PathBuf {
    GROK_HOME
        .get_or_init(|| {
            let groky = std::env::var("GROKY_HOME").ok();
            let grok = std::env::var("GROK_HOME").ok();
            init_grok_home_with(groky.as_deref(), grok.as_deref(), &canonical_user_home())
        })
        .clone()
}

/// The user-global grok home, but only when one genuinely resolves: `Some` when
/// `$GROKY_HOME`/`$GROK_HOME` is set or a home directory is found, `None`
/// otherwise. Unlike [`grok_home()`], this never falls back to a cwd-relative
/// `.groky`, so callers that *scan* user-global grok resources (hooks,
/// marketplace sources, ...) don't mistake a project's tree for the
/// user-global one when no home resolves.
pub fn user_grok_home() -> Option<PathBuf> {
    #[allow(deprecated)]
    let resolvable = std::env::var_os("GROKY_HOME").is_some()
        || std::env::var_os("GROK_HOME").is_some()
        || std::env::home_dir().is_some();
    resolvable.then(grok_home)
}

/// Canonical groky application path: `$GROK_HOME/bin/groky` (Unix) or `groky.exe` (Windows).
pub fn grok_application() -> PathBuf {
    grok_application_in(&grok_home())
}

/// [`grok_application`] under an explicit home instead of `$GROK_HOME`.
pub fn grok_application_in(home: &std::path::Path) -> PathBuf {
    let name = if cfg!(windows) { "groky.exe" } else { "groky" };
    home.join("bin").join(name)
}

/// System-wide config directory: `/etc/grok/` on Unix, `None` on Windows.
pub fn system_config_dir() -> Option<PathBuf> {
    if cfg!(unix) {
        Some(PathBuf::from("/etc/grok"))
    } else {
        None
    }
}

/// System path for the managed-settings.json used for settings compat, if it exists.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn claude_managed_settings_path() -> Option<PathBuf> {
    let path = PathBuf::from(CLAUDE_MANAGED_SETTINGS_PATH);
    path.exists().then_some(path)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn claude_managed_settings_path() -> Option<PathBuf> {
    None
}

/// The platform path where managed-settings.json would live for settings
/// compat, whether or not it exists. `None` on unsupported platforms.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn claude_managed_settings_probe_path() -> Option<PathBuf> {
    Some(PathBuf::from(CLAUDE_MANAGED_SETTINGS_PATH))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn claude_managed_settings_probe_path() -> Option<PathBuf> {
    None
}

/// Max bytes for a single directory name component (macOS APFS, Linux ext4,
/// NTFS all enforce 255 bytes).
const MAX_DIRNAME_BYTES: usize = 255;

/// Encode a CWD string into a filesystem-safe directory name component.
///
/// Short CWDs (URL-encoded form <= 255 bytes) use URL-encoding for backward
/// compatibility and human readability on disk.
///
/// Long CWDs (> 255 bytes encoded) use a compact `{slug}-{blake3_hex16}`
/// form that is always <= 57 bytes. Callers must write a `.cwd` metadata
/// file via [`ensure_sessions_cwd_dir`] so the original CWD can be
/// recovered by [`decode_cwd_from_dirname`].
pub fn encode_cwd_dirname(cwd: &str) -> String {
    let url_encoded = urlencoding::encode(cwd);
    if url_encoded.len() <= MAX_DIRNAME_BYTES {
        return url_encoded.into_owned();
    }
    let hash = blake3::hash(cwd.as_bytes());
    let hash16 = &hash.to_hex()[..16];
    let leaf = std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");
    let slug = slugify(leaf, 40);
    let slug = if slug.is_empty() { "workspace" } else { &slug };
    format!("{slug}-{hash16}")
}

/// Recover the original CWD from a sessions CWD directory.
///
/// Tries URL-decoding the directory name first (works for short/legacy dirs).
/// Falls back to reading a `.cwd` metadata file inside the directory (written
/// by [`ensure_sessions_cwd_dir`] for hash-based dirs).
pub fn decode_cwd_from_dirname(dir: &std::path::Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    if let Ok(decoded) = urlencoding::decode(name) {
        let s = decoded.into_owned();
        // URL-decoded absolute CWDs always start with `/` (Unix) or a drive
        // letter (Windows).  The slug-hash form never does, so this
        // distinguishes the two encodings unambiguously.
        if s.starts_with('/') || (cfg!(windows) && s.chars().nth(1) == Some(':')) {
            return Some(s);
        }
    }
    std::fs::read_to_string(dir.join(".cwd"))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Build the CWD-level session directory path:
/// `grok_home()/sessions/{encode_cwd_dirname(cwd)}`.
///
/// Does **not** create the directory on disk — use [`ensure_sessions_cwd_dir`]
/// when the directory must exist.
pub fn sessions_cwd_dir(cwd: &str) -> PathBuf {
    grok_home().join("sessions").join(encode_cwd_dirname(cwd))
}

/// Create the CWD-level session directory and write a `.cwd` metadata file
/// when hash-based encoding is used (long paths).
///
/// For short paths the `.cwd` file is not written because the directory name
/// itself is reversible via URL-decoding.
pub fn ensure_sessions_cwd_dir(cwd: &str) -> std::io::Result<PathBuf> {
    let encoded_name = encode_cwd_dirname(cwd);
    let dir = grok_home().join("sessions").join(&encoded_name);
    std::fs::create_dir_all(&dir)?;
    // Hash-based encoding is in use when the dirname differs from the
    // plain URL-encoded form.  Write a `.cwd` file so decode can recover
    // the original path.  O_CREAT|O_EXCL via create_new avoids TOCTOU
    // races with parallel session starts.
    if encoded_name != urlencoding::encode(cwd).as_ref() {
        let cwd_file = dir.join(".cwd");
        match std::fs::File::create_new(&cwd_file) {
            Ok(mut f) => {
                std::io::Write::write_all(&mut f, cwd.as_bytes())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(dir)
}

/// Generate a URL-safe slug from a string.
///
/// Lowercases, replaces non-alphanumeric chars with `-`, collapses
/// consecutive dashes, and truncates to `max_len` characters.
fn slugify(input: &str, max_len: usize) -> String {
    let mut result = String::with_capacity(input.len());
    let mut prev_dash = false;
    for c in input.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c);
            prev_dash = false;
        } else if !prev_dash {
            result.push('-');
            prev_dash = true;
        }
    }
    let trimmed = result.trim_matches('-');
    trimmed.chars().take(max_len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Realistic CWDs that trigger the bug (URL-encoded > 255 bytes).
    const LONG_CWDS: &[&str] = &[
        "/Users/dev/Documents/開発プロジェクト/機能追加/テスト環境/ソースコード/main-branch",
        "/Users/user/Library/Mobile Documents/com~apple~CloudDocs/项目文件/深层嵌套目录/更深层次的/工作区域/project",
        "/Users/user/Library/CloudStorage/OneDrive-대한민국회사/프로젝트/개발환경/소스코드/백엔드/서비스/my-app",
        "/Users/user/Documents/工作文件夹/二零二六年项目/子目录一/子目录二/子目录三/源代码/code",
    ];

    #[test]
    fn long_cwd_uses_hash_fallback_within_name_max() {
        let long_cwd = format!("/Users/test/{}", "中".repeat(30));
        let encoded = encode_cwd_dirname(&long_cwd);
        assert!(encoded.len() <= MAX_DIRNAME_BYTES);
        assert!(!encoded.starts_with("%2F"));
    }

    #[test]
    fn different_long_paths_produce_different_hashes() {
        let a = format!("/Users/test/{}", "中".repeat(30));
        let b = format!("/Users/test/{}", "日".repeat(30));
        assert_ne!(encode_cwd_dirname(&a), encode_cwd_dirname(&b));
    }

    #[test]
    fn decode_reads_cwd_file_for_hash_dirs() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("some-slug-abcdef0123456789");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".cwd"), "/original/long/path").unwrap();
        assert_eq!(
            decode_cwd_from_dirname(&dir),
            Some("/original/long/path".to_string())
        );
    }

    #[test]
    fn decode_returns_none_without_cwd_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("some-slug-abcdef0123456789");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(decode_cwd_from_dirname(&dir), None);
    }

    #[test]
    fn cwd_file_write_is_idempotent_via_excl() {
        let tmp = TempDir::new().unwrap();
        let long_cwd = format!("/Users/test/{}", "中".repeat(30));
        let dir = tmp.path().join(encode_cwd_dirname(&long_cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd_file = dir.join(".cwd");
        std::fs::write(&cwd_file, &long_cwd).unwrap();
        match std::fs::File::create_new(&cwd_file) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            other => panic!("expected AlreadyExists, got: {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&cwd_file).unwrap(), long_cwd);
    }

    #[test]
    fn url_encoded_long_cwd_fails_on_real_filesystem() {
        let tmp = TempDir::new().unwrap();
        let url_encoded = urlencoding::encode(LONG_CWDS[0]).into_owned();
        let result = std::fs::create_dir_all(tmp.path().join(&url_encoded));
        assert!(result.is_err());
    }

    #[test]
    fn full_roundtrip_on_real_filesystem_for_long_cwds() {
        let tmp = TempDir::new().unwrap();
        for cwd in LONG_CWDS {
            let encoded = encode_cwd_dirname(cwd);
            let dir = tmp.path().join(&encoded);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(".cwd"), cwd).unwrap();
            assert_eq!(decode_cwd_from_dirname(&dir).as_deref(), Some(*cwd));
        }
    }

    #[test]
    fn short_cwds_use_url_encoding_and_roundtrip_on_real_filesystem() {
        let tmp = TempDir::new().unwrap();
        for cwd in [
            "/Users/foo/project",
            "/tmp",
            "/Users/user/Documents/project-名前",
        ] {
            let encoded = encode_cwd_dirname(cwd);
            assert_eq!(encoded, urlencoding::encode(cwd).into_owned());
            let dir = tmp.path().join(&encoded);
            std::fs::create_dir_all(&dir).unwrap();
            assert_eq!(decode_cwd_from_dirname(&dir).as_deref(), Some(cwd));
        }
    }

    #[test]
    fn default_grok_home_has_no_verbatim_prefix() {
        // On Windows, std::fs::canonicalize returns `\\?\C:\...` verbatim
        // paths that external tools (notably `git clone`) reject. The dunce
        // canonicalization must yield a plain path. No-op assertion on Unix.
        let home = default_grok_home();
        assert!(!home.to_string_lossy().starts_with(r"\\?\"));
        assert!(home.ends_with(".groky"));
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World!", 40), "hello-world");
    }

    #[test]
    fn slugify_cjk_produces_empty() {
        assert_eq!(slugify("深层目录", 40), "");
    }

    #[test]
    fn slugify_truncates() {
        assert_eq!(slugify(&"a".repeat(100), 10).len(), 10);
    }
}
