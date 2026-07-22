//! Tests for the `~/.groky` config home: env precedence (`GROKY_HOME` →
//! `GROK_HOME` → default `~/.groky`) and one-time legacy migration from
//! `~/.grok`.
//!
//! The memoized `grok_home()` cannot be exercised repeatedly in one process,
//! so these tests target the non-memoized building blocks it is composed of:
//! [`xai_grok_config::resolve_grok_home_with`],
//! [`xai_grok_config::migrate_legacy_home`], and
//! [`xai_grok_config::init_grok_home_with`].

use std::path::PathBuf;

use tempfile::TempDir;
use xai_grok_config::{init_grok_home_with, migrate_legacy_home, resolve_grok_home_with};

// ── Precedence ──────────────────────────────────────────────────────────────

#[test]
fn groky_home_env_wins_over_grok_home() {
    let home = TempDir::new().unwrap();
    let resolved = resolve_grok_home_with(Some("/custom/groky"), Some("/custom/grok"), home.path());
    assert_eq!(resolved, PathBuf::from("/custom/groky"));
}

#[test]
fn grok_home_env_honored_when_groky_unset() {
    let home = TempDir::new().unwrap();
    let resolved = resolve_grok_home_with(None, Some("/custom/grok"), home.path());
    assert_eq!(resolved, PathBuf::from("/custom/grok"));
}

#[test]
fn default_is_dot_groky_under_home() {
    let home = TempDir::new().unwrap();
    let resolved = resolve_grok_home_with(None, None, home.path());
    assert_eq!(resolved, home.path().join(".groky"));
}

// ── Migration ───────────────────────────────────────────────────────────────

#[test]
fn migration_copies_files_recursively() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    let new = tmp.path().join(".groky");
    std::fs::create_dir_all(legacy.join("sessions").join("nested")).unwrap();
    std::fs::write(legacy.join("config.toml"), "x = 1").unwrap();
    std::fs::write(legacy.join("sessions").join("nested").join("s.json"), "{}").unwrap();

    let migrated = migrate_legacy_home(&legacy, &new).unwrap();
    assert!(migrated);
    assert_eq!(
        std::fs::read_to_string(new.join("config.toml")).unwrap(),
        "x = 1"
    );
    assert_eq!(
        std::fs::read_to_string(new.join("sessions").join("nested").join("s.json")).unwrap(),
        "{}"
    );
    // Legacy tree untouched.
    assert_eq!(
        std::fs::read_to_string(legacy.join("config.toml")).unwrap(),
        "x = 1"
    );
}

#[cfg(unix)]
#[test]
fn migration_preserves_0600_on_auth_json() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    let new = tmp.path().join(".groky");
    std::fs::create_dir_all(&legacy).unwrap();
    let auth = legacy.join("auth.json");
    std::fs::write(&auth, "{\"token\":\"secret\"}").unwrap();
    std::fs::set_permissions(&auth, std::fs::Permissions::from_mode(0o600)).unwrap();

    assert!(migrate_legacy_home(&legacy, &new).unwrap());
    let mode = std::fs::metadata(new.join("auth.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "auth.json must stay 0600 after migration");
}

#[test]
fn migration_skipped_when_target_exists() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    let new = tmp.path().join(".groky");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("config.toml"), "legacy").unwrap();
    std::fs::create_dir_all(&new).unwrap();
    std::fs::write(new.join("config.toml"), "current").unwrap();

    let migrated = migrate_legacy_home(&legacy, &new).unwrap();
    assert!(!migrated, "must not migrate over an existing ~/.groky");
    assert_eq!(
        std::fs::read_to_string(new.join("config.toml")).unwrap(),
        "current",
        "existing ~/.groky contents must be untouched"
    );
}

#[test]
fn migration_skipped_when_legacy_missing() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    let new = tmp.path().join(".groky");
    assert!(!migrate_legacy_home(&legacy, &new).unwrap());
    assert!(
        !new.exists(),
        "no ~/.groky should be created by a no-op migration"
    );
}

#[cfg(unix)]
#[test]
fn migration_skips_symlinks_but_copies_files() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    let new = tmp.path().join(".groky");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("config.toml"), "x = 1").unwrap();
    std::os::unix::fs::symlink(legacy.join("config.toml"), legacy.join("link.toml")).unwrap();

    assert!(migrate_legacy_home(&legacy, &new).unwrap());
    assert!(new.join("config.toml").is_file());
    assert!(
        !new.join("link.toml").exists(),
        "symlinks are skipped (with a warning), not copied"
    );
}

// ── init (resolution + conditional migration) ───────────────────────────────

#[test]
fn init_migrates_on_default_path() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("config.toml"), "x = 1").unwrap();

    let resolved = init_grok_home_with(None, None, tmp.path());
    assert_eq!(resolved, tmp.path().join(".groky"));
    assert_eq!(
        std::fs::read_to_string(resolved.join("config.toml")).unwrap(),
        "x = 1"
    );
}

#[test]
fn init_skips_migration_when_env_var_set() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("config.toml"), "x = 1").unwrap();
    let custom = tmp.path().join("custom-home");

    // GROK_HOME (legacy env) set: no migration, resolved is the env value.
    let resolved = init_grok_home_with(None, Some(custom.to_str().unwrap()), tmp.path());
    assert_eq!(resolved, custom);
    assert!(
        !tmp.path().join(".groky").exists(),
        "migration must never run when an env override is set"
    );

    // GROKY_HOME set: same.
    let custom2 = tmp.path().join("custom-home-2");
    let resolved = init_grok_home_with(Some(custom2.to_str().unwrap()), None, tmp.path());
    assert_eq!(resolved, custom2);
    assert!(!tmp.path().join(".groky").exists());
}

#[test]
fn init_creates_fresh_groky_without_legacy() {
    let tmp = TempDir::new().unwrap();
    let resolved = init_grok_home_with(None, None, tmp.path());
    assert_eq!(resolved, tmp.path().join(".groky"));
    assert!(resolved.is_dir());
}

#[cfg(unix)]
#[test]
fn init_falls_back_to_fresh_groky_when_copy_fails() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("config.toml"), "x = 1").unwrap();
    std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o000)).unwrap();

    // Root (some CI containers) ignores permission bits; skip if unreadable
    // couldn't be simulated.
    if std::fs::read_dir(&legacy).is_ok() {
        std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipping: cannot make legacy dir unreadable (running as root?)");
        return;
    }

    let resolved = init_grok_home_with(None, None, tmp.path());
    // Restore perms so TempDir cleanup can remove the tree.
    std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(resolved, tmp.path().join(".groky"));
    assert!(
        resolved.is_dir(),
        "fresh ~/.groky created despite copy failure"
    );
    assert_eq!(
        std::fs::read_dir(&resolved).unwrap().count(),
        0,
        "fallback ~/.groky must be fresh-empty, never a partial copy"
    );
    assert!(
        legacy.join("config.toml").exists(),
        "legacy tree never deleted"
    );
    assert_no_staging_leftovers(tmp.path());
}

/// A mid-copy failure (some entries copied, then an unreadable subdir aborts)
/// must not leave a partial `~/.groky` behind: the target stays absent so a
/// later run — once the obstacle is gone — migrates fully instead of being
/// locked out by the partial tree.
#[cfg(unix)]
#[test]
fn failed_migration_leaves_no_partial_groky_and_retry_migrates_fully() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join(".grok");
    let new = tmp.path().join(".groky");
    std::fs::create_dir_all(legacy.join("locked")).unwrap();
    std::fs::write(legacy.join("a.txt"), "a").unwrap();
    std::fs::write(legacy.join("locked").join("inner.txt"), "inner").unwrap();
    std::fs::set_permissions(
        &legacy.join("locked"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();

    if std::fs::read_dir(legacy.join("locked")).is_ok() {
        std::fs::set_permissions(
            &legacy.join("locked"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        eprintln!("skipping: cannot make subdir unreadable (running as root?)");
        return;
    }

    let result = migrate_legacy_home(&legacy, &new);
    // Restore perms first so TempDir cleanup works even if asserts fail.
    std::fs::set_permissions(
        &legacy.join("locked"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    assert!(result.is_err(), "mid-copy failure must surface as Err");
    assert!(
        !new.exists(),
        "a failed migration must not leave a partial ~/.groky that blocks retries"
    );
    assert_no_staging_leftovers(tmp.path());

    // Obstacle removed: a retry must migrate the full tree.
    assert!(migrate_legacy_home(&legacy, &new).unwrap());
    assert_eq!(std::fs::read_to_string(new.join("a.txt")).unwrap(), "a");
    assert_eq!(
        std::fs::read_to_string(new.join("locked").join("inner.txt")).unwrap(),
        "inner"
    );
}

/// No `~/.groky.migrating-*` staging directories may survive a migration
/// attempt (success or failure) in the home dir.
fn assert_no_staging_leftovers(home: &std::path::Path) {
    let leftovers: Vec<_> = std::fs::read_dir(home)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".migrating"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging dirs left behind: {leftovers:?}"
    );
}
