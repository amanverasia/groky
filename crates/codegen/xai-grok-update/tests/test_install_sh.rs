//! Release-contract tests for the pager installers.
//!
//! The bash installer runs end-to-end against a fake `curl` which serves a
//! versioned Groky release tarball and companion SHA-256 manifest. It proves a
//! valid artifact installs and a checksum mismatch preserves the prior binary.
//! The remaining tests assert all four published entrypoints use the supported
//! release contract and explicitly reject unavailable Windows assets.
//! Installers are resolved from the sibling `xai-grok-pager` crate.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn script_path(name: &str) -> Option<PathBuf> {
    dunce::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../xai-grok-pager/scripts/{name}")),
    )
    .ok()
    .filter(|p| p.exists())
}

fn host_platform() -> String {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "aarch64"
    };
    format!("{os}-{arch}")
}

const GOOD_SCRIPT: &str = "#!/bin/sh\nexit 0\n";

/// Write a fake `curl` that serves a release tarball and its checksum.
/// `$FAKE_MODE` (full|truncate|garbage|other_filename) selects the served artifact.
fn write_fake_curl(dir: &Path) {
    let body = r#"#!/bin/bash
mode="${FAKE_MODE:-full}"
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) shift; out="$1" ;;
    -*) : ;;
    *) url="$1" ;;
  esac
  shift
done
[ -n "$out" ] || { printf 'v0.1.181'; exit 0; }

if [[ "$url" == *.sha256 ]]; then
  tarball="${out%.sha256}"
  case "$mode" in
    full)
      sha256sum "$tarball" | sed "s|  $tarball$|  $(basename "$tarball")|" > "$out"
      ;;
    other_filename)
      printf 'a valid checksum for a different file' > "$(dirname "$tarball")/another-release.tar.gz"
      sha256sum "$(dirname "$tarball")/another-release.tar.gz" | sed 's|.*/|  |' > "$out"
      ;;
    *)
      printf '0000000000000000000000000000000000000000000000000000000000000000  %s\n' "$(basename "$tarball")" > "$out"
      ;;
  esac
  exit 0
fi

case "$mode" in
  full)
    release_dir="${out%.tar.gz}"
    mkdir -p "$release_dir"
    printf '%s' '#!/bin/sh\nexit 0\n' > "$release_dir/groky"
    chmod 755 "$release_dir/groky"
    tar -C "$(dirname "$release_dir")" -czf "$out" "$(basename "$release_dir")"
    rm -rf "$release_dir"
    ;;
  truncate) printf '\0\0\0\0' > "$out" ;;
  garbage)  printf 'not a groky release tarball' > "$out" ;;
esac
"#;
    let path = dir.join("curl");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Seed a valid previous-good binary + symlink in the isolated home.
fn seed_previous_good(home: &Path, platform: &str) -> PathBuf {
    let downloads = home.join(".grok").join("downloads");
    let bin = home.join(".grok").join("bin");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let prev = downloads.join(format!("grok-{platform}"));
    std::fs::write(&prev, GOOD_SCRIPT).unwrap();
    std::fs::set_permissions(&prev, std::fs::Permissions::from_mode(0o755)).unwrap();
    let link = bin.join("grok");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(format!("../downloads/grok-{platform}"), &link).unwrap();
    dunce::canonicalize(&prev).unwrap()
}

/// Re-resolve `$BIN_DIR/grok` from disk and re-run it: the active grok must
/// always execute, and never be a `.tmp`/partial file.
fn assert_active_grok_runs(home: &Path) {
    let link = home.join(".grok").join("bin").join("grok");
    assert!(link.is_symlink(), "grok must remain a symlink");
    let resolved =
        dunce::canonicalize(&link).unwrap_or_else(|e| panic!("grok symlink dangles: {e}"));
    let name = resolved.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        !name.contains(".tmp"),
        "active grok must not be a temp file: {name}"
    );
    let ok = Command::new(&resolved)
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "active grok must run: {}", resolved.display());
}

fn run_installer_with_env(
    install_sh: &Path,
    home: &Path,
    fakebin: &Path,
    mode: &str,
    envs: &[(&str, PathBuf)],
) -> bool {
    let path_env = format!("{}:/usr/bin:/bin", fakebin.display());
    let mut command = Command::new("/bin/bash");
    command
        .arg(install_sh)
        .arg("0.1.181")
        .env_clear()
        .env("HOME", home)
        .env("PATH", path_env)
        .env("GROK_CHANNEL", "stable")
        .env("FAKE_MODE", mode);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.status().expect("spawn bash install.sh").success()
}

fn run_installer(install_sh: &Path, home: &Path, fakebin: &Path, mode: &str) -> bool {
    run_installer_with_env(
        install_sh,
        home,
        fakebin,
        mode,
        &[("GROK_BIN_DIR", home.join(".grok").join("bin"))],
    )
}

#[test]
fn pager_bash_installers_accept_checked_releases_and_reject_bad_checksums() {
    let platform = host_platform();
    let fakedir = tempfile::tempdir().unwrap();
    write_fake_curl(fakedir.path());

    for script_name in ["install.sh", "install-enterprise.sh"] {
        let script = script_path(script_name)
            .unwrap_or_else(|| panic!("missing {script_name} relative to updater crate"));
        let cases = [
            ("full", true),
            ("truncate", false),
            ("garbage", false),
            ("other_filename", false),
            ("full", true),
        ];

        for (mode, expect_ok) in cases {
            let home = tempfile::tempdir().unwrap();
            seed_previous_good(home.path(), &platform);

            let ok = run_installer(&script, home.path(), fakedir.path(), mode);
            assert_eq!(
                ok, expect_ok,
                "{script_name} mode={mode} exit success mismatch"
            );

            // A rejected artifact must leave the working previous binary active.
            assert_active_grok_runs(home.path());
        }
    }
}

#[test]
fn pager_bash_installers_default_to_canonical_groky_home() {
    let fakedir = tempfile::tempdir().unwrap();
    write_fake_curl(fakedir.path());

    for script_name in ["install.sh", "install-enterprise.sh"] {
        let script = script_path(script_name).unwrap_or_else(|| panic!("missing {script_name}"));
        let home = tempfile::tempdir().unwrap();
        let canonical = home.path().join("canonical");
        let legacy = home.path().join("legacy");

        assert!(
            run_installer_with_env(
                &script,
                home.path(),
                fakedir.path(),
                "full",
                &[
                    ("GROKY_HOME", canonical.clone()),
                    ("GROK_HOME", legacy.clone())
                ],
            ),
            "{script_name} must install using GROKY_HOME before GROK_HOME"
        );
        let target = if cfg!(target_arch = "x86_64") {
            "x86_64-unknown-linux-gnu"
        } else {
            "aarch64-unknown-linux-gnu"
        };
        assert!(
            canonical
                .join("downloads")
                .join(format!("groky-{target}"))
                .is_file()
        );
        assert!(canonical.join("bin/groky").is_symlink());
        assert!(canonical.join("bin/agent").is_symlink());
        assert!(!legacy.exists(), "GROK_HOME must lose to GROKY_HOME");

        let default_home = tempfile::tempdir().unwrap();
        assert!(
            run_installer_with_env(&script, default_home.path(), fakedir.path(), "full", &[]),
            "{script_name} must install with the default home"
        );
        assert!(default_home.path().join(".groky/downloads").is_dir());
        assert!(default_home.path().join(".groky/bin/groky").is_symlink());
        assert!(default_home.path().join(".groky/bin/agent").is_symlink());
        assert!(!default_home.path().join(".grok").exists());
    }
}

#[test]
fn pager_bash_installers_follow_the_groky_github_release_contract() {
    for script in ["install.sh", "install-enterprise.sh"] {
        let path = script_path(script).unwrap_or_else(|| panic!("missing {script}"));
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        assert!(
            body.contains("github.com/amanverasia/groky/releases/download/${release_tag}"),
            "{script} must request assets below the resolved release tag"
        );
        assert!(
            body.contains("tarball=\"groky-${tag}-${target}.tar.gz\""),
            "{script} must request the tagged groky tarball"
        );
        assert!(
            body.contains("$tarball.sha256"),
            "{script} must download the tarball checksum"
        );
        assert!(
            body.contains("sha256sum") && body.contains("actual_checksum"),
            "{script} must verify the checksum against the expected tarball"
        );
        assert!(
            body.contains("tar xzf") && body.contains("install -m 755"),
            "{script} must extract and install the verified binary"
        );
        assert!(
            !body.contains("/${CHANNEL}")
                && !body.contains("/stable")
                && !body.contains("/alpha")
                && !body.contains("/enterprise"),
            "{script} must not request a legacy channel pointer"
        );
        assert!(
            !body.contains("grok-${version}") && !body.contains("grok-$version"),
            "{script} must not request a legacy raw grok binary asset"
        );
    }
}

#[test]
fn pager_powershell_installers_make_no_legacy_or_release_asset_requests() {
    for script in ["install.ps1", "install-enterprise.ps1"] {
        let path = script_path(script).unwrap_or_else(|| panic!("missing {script}"));
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            !body.contains("Invoke-WebRequest") && !body.contains("releases/download"),
            "{script} must fail before requesting an unavailable Windows asset"
        );
        assert!(
            !body.contains("/stable") && !body.contains("/alpha") && !body.contains("/enterprise"),
            "{script} must not request a legacy channel pointer"
        );
    }
}

#[test]
fn pager_installers_explicitly_reject_windows_until_windows_release_assets_exist() {
    for script in ["install.sh", "install-enterprise.sh"] {
        let path = script_path(script).unwrap_or_else(|| panic!("missing {script}"));
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            body.contains("Windows is not supported") && !body.contains("os=\"windows\""),
            "{script} must explicitly reject Windows rather than request an unpublished asset"
        );
    }

    for script in ["install.ps1", "install-enterprise.ps1"] {
        let path = script_path(script).unwrap_or_else(|| panic!("missing {script}"));
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            body.contains("Windows release assets are not available"),
            "{script} must explicitly reject unsupported Windows installation"
        );
    }
}
