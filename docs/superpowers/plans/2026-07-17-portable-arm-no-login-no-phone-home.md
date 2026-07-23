# Portable ARM Builds, No Login, No Phone-Home — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship groky v0.1.1: aarch64 release binaries run on all ARMv8 hosts, the xAI auto-updater becomes a local-only facade with zero network I/O, and a credential-less first run lands in the app instead of a blocking grok.com login screen.

**Architecture:** Three independent changes. (1) Build config: `target-cpu=generic` for `aarch64-unknown-linux-gnu` plus a CI guard. (2) `xai-grok-update` keeps its public API (all external call sites in `xai-grok-pager-bin/src/main.rs` and `xai-grok-pager` compile unchanged) but every networked/installing/config-mutating body becomes an inert stub — the same "local-only compatibility facade" pattern already used by `xai-grok-telemetry`. (3) `xai-grok-shell::agent::auth_method::build_unpinned` stops advertising the zero-config grok.com login; the pager's empty-methods and auth-failure fallbacks change from "block on login screen" to "land in the app", with a passive `StartupWarning` hint on the welcome screen.

**Tech Stack:** Rust workspace (edition per `rust-toolchain.toml`), cargo, GitHub Actions, bash guard scripts. Test with `cargo test -p <crate>`.

**Spec:** `docs/superpowers/specs/2026-07-17-portable-arm-no-login-no-phone-home-design.md`

**Design refinements discovered during code recon (approved deviations from spec wording):**
- Interactive login methods that require explicit user configuration (enterprise OIDC, `auth_provider_command`, `[auth] preferred_method = "oidc"` pin) are KEPT. Only the zero-config grok.com default is removed. These never fire without the user writing config for them, so the no-phone-home policy holds.
- Removing the advertised method alone is NOT enough: with an empty methods list, `eager_auth_or_login_fallback` in `crates/codegen/xai-grok-pager/src/acp/mod.rs` fail-closes into a blocking Pending screen. Tasks 8–9 fix both layers.
- `minimum_version.rs` becomes a full no-op (its enforcement path could fetch versions and trigger installs; groky has neither).
- The mid-session 401 fallthrough (`method_id_after_cached_token_unavailable` in auth_method.rs) is left unchanged: it only fires for users who previously completed a grok.com login (pre-existing cached token), which is an explicit opt-in. Out of scope.

---

## Task 0: Branch

**Files:** none

- [x] **Step 0.1: Create a work branch from main**

```bash
git checkout -b fix/portable-arm-no-login-no-phone-home
```

---

## Task 1: Portable aarch64 build flags + CI guard

**Files:**
- Modify: `.cargo/config.toml:37-38`
- Modify: `.github/workflows/release.yml` (insert step before the `cargo build` step at line 61)

- [x] **Step 1.1: Change target-cpu to generic**

In `.cargo/config.toml`, replace:

```toml
[target.aarch64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=neoverse-v2", "-C", "force-unwind-tables=yes"]
```

with:

```toml
# target-cpu=generic (SIGILL fix): upstream tunes for Neoverse V2 (ARMv9-A)
# servers, which emits instructions that trap on ARMv8 hosts such as Oracle
# Ampere A1 (Neoverse N1). GitHub's arm runners are ARMv9, so only the CI
# guard in release.yml catches a regression before users do.
[target.aarch64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=generic", "-C", "force-unwind-tables=yes"]
```

- [x] **Step 1.2: Add the CI guard step to release.yml**

In `.github/workflows/release.yml`, insert between the `Derive version` step and the `cargo build` step (i.e., immediately before `- run: cargo build --release -p xai-grok-pager-bin`):

```yaml
      # Release binaries must be portable. An over-specific target-cpu
      # (e.g. upstream's neoverse-v2) generates instructions that SIGILL on
      # older CPUs; the runner itself is too new to catch it in the smoke
      # test. See docs/superpowers/specs/2026-07-17-portable-arm-no-login-no-phone-home-design.md
      - name: Guard portable target-cpu
        run: |
          if grep -En 'target-cpu=' .cargo/config.toml | grep -v 'target-cpu=generic'; then
            echo "non-generic target-cpu in .cargo/config.toml breaks portable Linux builds" >&2
            exit 1
          fi
```

- [x] **Step 1.3: Verify the guard logic locally**

```bash
bash -c "if grep -En 'target-cpu=' .cargo/config.toml | grep -v 'target-cpu=generic'; then echo FAIL; else echo PASS; fi"
```

Expected: `PASS` (after Step 1.1). Sanity-check the negative: temporarily `git stash`, run again, expect the neoverse-v2 line printed + `FAIL`, then `git stash pop`.

- [x] **Step 1.4: Commit**

```bash
git add .cargo/config.toml .github/workflows/release.yml
git commit -m "fix(build): use target-cpu=generic for aarch64 linux, guard in CI

Upstream's neoverse-v2 tuning emits ARMv9 instructions that SIGILL on
ARMv8 hosts (Oracle Ampere A1 / Neoverse N1). The release runner is
ARMv9 so the smoke test passed; add a CI guard so upstream syncs cannot
reintroduce the flag."
```

---

## Task 2: Updater facade — write the failing behavior tests

**Files:**
- Create: `crates/codegen/xai-grok-update/tests/test_facade.rs`

**TDD deviation, documented:** do NOT run these tests before Task 3–4 land. Against the current code, `run_update_if_available` reaches the real network and writes `cli.auto_update = true` into the developer's real `~/.groky` config — running it pre-facade is harmful, not just red. The "failing" evidence is that the assertions codify behavior the current code demonstrably violates (network fetch + config write, per `auto_update.rs:388-461,489-498`).

- [x] **Step 2.1: Write the facade contract tests**

Create `crates/codegen/xai-grok-update/tests/test_facade.rs`:

```rust
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
```

- [x] **Step 2.2: Verify it compiles against the current API (do not run)**

```bash
cargo test -p xai-grok-update --test test_facade --no-run
```

Expected: compiles (the facade preserves this exact API; compiling now proves the tests target the real signatures).

- [x] **Step 2.3: Commit**

```bash
git add crates/codegen/xai-grok-update/tests/test_facade.rs
git commit -m "test(update): add facade contract tests (not yet passing/run)"
```

---

## Task 3: Updater facade — strip networking from version.rs

**Files:**
- Modify: `crates/codegen/xai-grok-update/src/version.rs`

- [x] **Step 3.1: Delete the networked fetchers and endpoint constants**

Delete these items from `version.rs` (names as in the current file; line numbers pre-edit):

- `NPM_PACKAGE` const (line 13)
- `GH_RELEASE_REPO` const (line 14)
- `CLI_BASE_URL_PRIMARY`, `CLI_BASE_URL_FALLBACK`, `CLI_BASE_URLS` consts (lines 16–27)
- `semver_max` fn (99–104) — only the deleted fetchers used it
- `fetch_npm_version` (106–119), `fetch_npm_tag_for_test` (121–126), `fetch_npm_version_for_test` (128–136), `fetch_npm_tag` (138–172)
- `fetch_gh_release_version` (174–188), `fetch_gh_release_latest` (190–225)
- `fetch_gcs_version` (227–258), `fetch_gcs_version_from_base` (260–272), `fetch_gcs_channel_pointer` (274–339)
- `fetch_latest_version` (341–351), `get_latest_version` (390–402)
- `try_fetch_stable_pointer` (479–502)
- In `mod tests`: `test_semver_max_matrix` and `test_semver_max_invalid_input_returns_err`

Keep (unchanged): `TTL_SECONDS_BEFORE_AUTO_UPDATE`, `UpdateConfig` + `from_environment`, `GrokVersion` + `is_fresh`/`new`, `write_version_cache`, `is_version_cache_fresh`, `pub use xai_grok_version::installed as get_installed_grok_version`, `installed_on_disk_version`, `version_from_versioned_binary_name`, `cached_stable_version`, `derive_channel`, `channel_name`, `channel_label`, and all remaining tests.

- [x] **Step 3.2: Fix the imports at the top of version.rs**

Replace the import block (lines 1–10) with:

```rust
use std::time::Duration;

use serde::Deserialize;
use tokio::fs;

use xai_grok_shell::env::GrokBuildEnvironment;
use xai_grok_shell::util::grok_home::grok_home;
```

(Removed: `anyhow::Result`, `serde_json::Value`, `tokio::process::Command` — all only used by deleted fetchers.)

- [x] **Step 3.3: Update the module-level docs on write_version_cache and channel helpers**

In the doc comment of `write_version_cache` (pre-edit line 353–358) the text still reads correctly (local cache write). In the doc comment of `channel_label` (pre-edit 545–553) and `channel_name`, replace the phrase `(written by the auto-updater)` with `(written by a previous upstream-grok auto-updater run, if any)` so the docs don't claim an updater exists.

- [x] **Step 3.4: Check it compiles (auto_update.rs will now be broken — expected)**

```bash
cargo check -p xai-grok-update 2>&1 | tail -20
```

Expected: errors ONLY in `auto_update.rs` / `minimum_version.rs` referencing deleted items (`fetch_latest_version`, `get_latest_version`, `try_fetch_stable_pointer`, `CLI_BASE_URLS`, ...). No errors inside `version.rs` itself. Do not commit yet — Task 4 restores the build.

---

## Task 4: Updater facade — rewrite auto_update.rs and minimum_version.rs

**Files:**
- Rewrite: `crates/codegen/xai-grok-update/src/auto_update.rs` (entire file replaced)
- Rewrite: `crates/codegen/xai-grok-update/src/minimum_version.rs` (entire file replaced)
- Unchanged: `crates/codegen/xai-grok-update/src/lib.rs` (all its re-exports still resolve)

- [x] **Step 4.1: Replace auto_update.rs entirely with the facade**

```rust
//! Local-only compatibility facade for the former auto-updater.
//!
//! groky does not phone home. The upstream implementation checked
//! `https://x.ai/cli` at startup, wrote `cli.auto_update = true` into the
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
        println!(
            "Grok Build - v{} [{}]",
            status.current_version, status.channel
        );
        println!("Update check failed: {error}");
        return Ok(());
    }

    let channel_label = format!(" [{}]", status.channel);

    if status.update_available {
        if let Some(latest_version) = status.latest_version.as_deref() {
            println!(
                "A new version of Grok Build is available: {} -> {}{}",
                status.current_version, latest_version, channel_label
            );
        } else {
            println!("A new version of Grok Build is available.");
        }
        return Ok(());
    }

    if let Some(latest_version) = status.latest_version.as_deref() {
        println!(
            "Grok Build - v{} (latest: {}){}",
            status.current_version, latest_version, channel_label
        );
        return Ok(());
    }

    println!("Grok Build - v{}{}", status.current_version, channel_label);
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
```

Note what is intentionally gone (previously `pub` but with zero external callers, verified by repo-wide search): `auto_update_target`, `get_installer`, `restart_grok`, `run_install_script`, `download_with_progress`, `download_silent`, `install_internal_from_bases`, `install_internal_from_base`, `install_npm_for_test`, plus all private download/symlink/npm/gh machinery.

- [x] **Step 4.2: Replace minimum_version.rs entirely**

```rust
//! Local-only facade: minimum-version enforcement is disabled in groky.
//!
//! The upstream implementation read `cli.minimum_version` from managed
//! config, fetched the latest version, and could auto-install and
//! `std::process::exit` — all of which depend on the (removed) updater
//! networking. groky never phones home, so this is a no-op.

use crate::version::UpdateConfig;

/// No-op. See module docs.
pub async fn enforce_minimum_version_or_exit(_update_config: &UpdateConfig) {}
```

- [x] **Step 4.3: Verify lib.rs still resolves and the crate compiles**

`src/lib.rs` must remain exactly:

```rust
pub mod auto_update;
mod minimum_version;
pub mod version;

pub use auto_update::UpdateStatus;
pub use minimum_version::enforce_minimum_version_or_exit;
pub use version::{UpdateConfig, channel_label, channel_name, write_version_cache};
```

```bash
cargo check -p xai-grok-update
```

Expected: PASS (test files still reference deleted items — that's Task 5; `cargo check` without `--tests` skips them).

- [x] **Step 4.4: Verify all external consumers compile unchanged**

```bash
cargo check -p xai-grok-pager -p xai-grok-pager-bin
```

Expected: PASS with zero source changes in either crate.

- [x] **Step 4.5: Commit**

```bash
git add crates/codegen/xai-grok-update/src
git commit -m "feat(update): neuter updater into local-only facade

No network I/O, no npm/gh subprocesses, no downloads, no bin symlink
swaps, no config mutation (upstream force-wrote cli.auto_update=true on
first run). Public API preserved so all call sites compile unchanged;
groky updates via the installer instead."
```

---

## Task 5: Updater facade — prune deps and tests

**Files:**
- Modify: `crates/codegen/xai-grok-update/Cargo.toml`
- Delete: `crates/codegen/xai-grok-update/tests/test_network.rs`
- Delete: `crates/codegen/xai-grok-update/tests/test_install_internal.rs`
- Delete: `crates/codegen/xai-grok-update/tests/test_subprocess.rs`
- Delete: `crates/codegen/xai-grok-update/tests/test_blitz_cancel.rs`
- Delete: `crates/codegen/xai-grok-update/tests/test_concurrent_convergence.rs`
- Delete: `crates/codegen/xai-grok-update/tests/test_downgrade_matrix.rs`
- Delete: `crates/codegen/xai-grok-update/tests/test_check_status_regression.rs`
- Delete: `crates/codegen/xai-grok-update/tests/common/artifact_server.rs`
- Modify: `crates/codegen/xai-grok-update/tests/common/mod.rs`
- Keep: `tests/test_io.rs` (local version-cache I/O), `tests/test_facade.rs`, `tests/test_install_sh.rs` (exercises the legacy pager install script with a fake curl; no network)

- [x] **Step 5.1: Delete the network/install test files**

```bash
git rm crates/codegen/xai-grok-update/tests/test_network.rs \
       crates/codegen/xai-grok-update/tests/test_install_internal.rs \
       crates/codegen/xai-grok-update/tests/test_subprocess.rs \
       crates/codegen/xai-grok-update/tests/test_blitz_cancel.rs \
       crates/codegen/xai-grok-update/tests/test_concurrent_convergence.rs \
       crates/codegen/xai-grok-update/tests/test_downgrade_matrix.rs \
       crates/codegen/xai-grok-update/tests/test_check_status_regression.rs \
       crates/codegen/xai-grok-update/tests/common/artifact_server.rs
```

- [x] **Step 5.2: Replace the crate Cargo.toml**

Removed deps: `reqwest`, `indicatif`, `futures`, `thiserror`, `xai-grok-tools`; removed dev-dep: `wiremock`.

```toml
[package]
license = "Apache-2.0"
name = "xai-grok-update"
version = "0.1.220-alpha.4"
edition.workspace = true

[dependencies]
anyhow = { workspace = true }
semver = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
time = { workspace = true, features = ["serde"] }
tokio = { workspace = true, features = ["fs", "process", "io-util", "macros", "time"] }
tracing = { workspace = true }
xai-grok-shell = { workspace = true }
xai-grok-version = { workspace = true }

[dev-dependencies]
semver = { workspace = true }
serde_json = { workspace = true }
serial_test = { workspace = true }
tempfile = { workspace = true }
time = { workspace = true, features = ["serde"] }
tokio = { workspace = true, features = ["fs", "macros", "rt-multi-thread", "test-util"] }
dunce = { workspace = true }
```

- [x] **Step 5.3: Trim tests/common/mod.rs**

Open `crates/codegen/xai-grok-update/tests/common/mod.rs`. Remove any `mod artifact_server;` / `pub use artifact_server::...` lines and any helpers the compiler now flags as unused (e.g., `FakeBinGuard`, fake-npm/gh PATH helpers) after Step 5.1. Keep the GROK_HOME isolation guard and anything `test_io.rs` / `test_install_sh.rs` still use. Compiler-guided: iterate until Step 5.4 is warning-clean for dead code in tests/common.

- [x] **Step 5.4: Run the crate test suite — facade tests now run for real**

```bash
cargo test -p xai-grok-update
```

Expected: PASS. This includes `test_facade.rs` (Task 2), `test_io.rs`, `test_install_sh.rs`, and the retained unit tests in `version.rs`. Fix any dead-code fallout in `tests/common/mod.rs` per Step 5.3.

- [x] **Step 5.5: Clippy the crate**

```bash
cargo clippy -p xai-grok-update --all-targets
```

Expected: no new warnings (async-without-await stubs are fine; the signatures are the API contract).

- [x] **Step 5.6: Commit**

```bash
git add -A crates/codegen/xai-grok-update
git commit -m "test(update): drop network test suites, prune reqwest/indicatif/futures deps"
```

---

## Task 6: Flip the guard script from protecting to policing the updater

**Files:**
- Modify: `bin/check-no-network-telemetry.sh:2-3,31-34`

- [x] **Step 6.1: Update the header comment**

Replace line 3:

```bash
# Update/announcement networking is intentionally retained (asserted below).
```

with:

```bash
# The updater is a local-only facade: upstream endpoints must not return.
# Announcement types are local-only persistence (no HTTP).
```

- [x] **Step 6.2: Replace the retention assertions with endpoint prohibitions**

Replace the tail of the script (current lines 31–34):

```bash
# Product networking intentionally retained.
rg -q 'xai-grok-update' Cargo.toml
rg -q 'xai-grok-announcements' Cargo.toml
echo "no network telemetry found"
```

with:

```bash
# The updater facade must stay present (API compatibility for upstream
# syncs) but must never regain upstream endpoints or an HTTP client.
rg -q 'xai-grok-update' Cargo.toml
rg -q 'xai-grok-announcements' Cargo.toml

forbidden_update='x\.ai/cli|grok-build-public-artifacts|@xai-official/grok|xai-org-shared/grok-build'
if rg -n "$forbidden_update" crates/codegen/xai-grok-update/src; then
  echo "upstream update endpoint survived in xai-grok-update" >&2; exit 1
fi
if rg -n '^\s*reqwest\s*=' crates/codegen/xai-grok-update/Cargo.toml; then
  echo "HTTP client returned to xai-grok-update" >&2; exit 1
fi

echo "no network telemetry found"
```

- [x] **Step 6.3: Run the guard, then prove it can fail**

```bash
./bin/check-no-network-telemetry.sh
```

Expected: `no network telemetry found`, exit 0.

Negative check: append `// https://x.ai/cli` to `crates/codegen/xai-grok-update/src/lib.rs`, rerun, expect `upstream update endpoint survived` + exit 1, then revert with `git checkout -- crates/codegen/xai-grok-update/src/lib.rs`.

- [x] **Step 6.4: Commit**

```bash
git add bin/check-no-network-telemetry.sh
git commit -m "chore(guard): forbid upstream update endpoints instead of protecting them"
```

---

## Task 7: Stop advertising the zero-config grok.com login (shell)

**Files:**
- Modify: `crates/codegen/xai-grok-shell/src/agent/auth_method.rs` (`build_unpinned` at 217–263; tests at 684–695, 863–894, 1027–1051)

- [x] **Step 7.1: Update the tests first (failing)**

Replace `fresh_user_only_advertises_grok_com_and_requires_login` (lines 684–695) with:

```rust
    /// Brand-new user (no API key, no cached token): NO methods are
    /// advertised — groky does not offer the zero-config grok.com browser
    /// login, so the pager skips the login screen and lands in the app.
    #[test]
    fn fresh_user_advertises_no_methods_and_lands_in_app() {
        let built = build_auth_methods(default_inputs());

        assert!(built.methods.is_empty());
        assert!(built.default_auth_method_id.is_none());
    }
```

In `disable_api_key_auth_suppresses_xai_api_key_method` (863–894), replace the final assertion pair:

```rust
        assert_eq!(
            first_kind(&built.methods),
            Some(AuthMethodKind::GrokCom),
            "with api-key auth disabled and no cached token, the login method \
             must lead so the pager requires interactive login",
        );
        assert!(built.default_auth_method_id.is_none());
```

with:

```rust
        assert!(
            built.methods.is_empty(),
            "with api-key auth disabled and no cached token, nothing is \
             advertised — groky never falls back to browser login",
        );
        assert!(built.default_auth_method_id.is_none());
```

In `no_legacy_token_means_no_cached_token_advertised` (1027–1051), replace the final assertion:

```rust
        assert_eq!(
            first_kind(&built.methods),
            Some(AuthMethodKind::GrokCom),
            "no cached token AND no api key: pager must show login (grok.com first)",
        );
```

with:

```rust
        assert!(
            built.methods.is_empty(),
            "no cached token AND no api key: nothing is advertised — the \
             pager lands in the app without a login screen",
        );
```

- [x] **Step 7.2: Run tests to verify they fail**

```bash
cargo test -p xai-grok-shell agent::auth_method
```

Expected: FAIL — exactly the three updated tests (methods currently contain grok.com).

- [x] **Step 7.3: Make build_unpinned conditional**

In `build_unpinned` (only there — `build_pinned_oidc`'s call at lines 203–209 stays unconditional), replace:

```rust
    push_interactive_login(
        &mut methods,
        has_enterprise_oidc,
        enterprise_oidc_issuer,
        login_label,
        has_auth_provider_command,
    );

    BuiltAuthMethods {
        methods,
        default_auth_method_id,
    }
}
```

with:

```rust
    // groky: the zero-config grok.com browser login is not advertised, so a
    // credential-less first run lands in the app instead of a login screen.
    // Interactive login is still offered when the user explicitly configured
    // it: enterprise OIDC, an external auth provider command, or the
    // `[auth] preferred_method = "oidc"` pin (see `build_pinned_oidc`).
    if has_enterprise_oidc || has_auth_provider_command {
        push_interactive_login(
            &mut methods,
            has_enterprise_oidc,
            enterprise_oidc_issuer,
            login_label,
            has_auth_provider_command,
        );
    }

    BuiltAuthMethods {
        methods,
        default_auth_method_id,
    }
}
```

(The `push_interactive_login` call inside `build_pinned_oidc` is preceded by different context — match on the `build_unpinned` body which contains the `has_cached_token` block directly above.)

- [x] **Step 7.4: Run the full auth_method test module**

```bash
cargo test -p xai-grok-shell agent::auth_method
```

Expected: PASS — including the untouched `auth_provider_command_sets_external_provider_meta` (explicit config keeps grok.com), `enterprise_oidc_replaces_grok_com_but_xai_api_key_still_first`, `session_only_user_first_method_is_cached_token`, and both `pin_oidc_*` tests.

- [x] **Step 7.5: Commit**

```bash
git add crates/codegen/xai-grok-shell/src/agent/auth_method.rs
git commit -m "feat(auth): stop advertising zero-config grok.com login

Credential-less builds advertise no auth methods. Explicitly configured
interactive logins (enterprise OIDC, auth_provider_command, preferred
oidc pin) are unchanged."
```

---

## Task 8: Land in the app when no interactive login exists (pager ACP layer)

**Files:**
- Modify: `crates/codegen/xai-grok-pager/src/acp/mod.rs` (`eager_auth_or_login_fallback` at 646–707; tests module)

- [x] **Step 8.1: Write failing tests for the new pure fallback helper**

Add to the existing `#[cfg(test)] mod tests` in `acp/mod.rs` (near `shell_built_auth_methods_for_byok_user_skip_login_screen`):

```rust
    #[test]
    fn login_fallback_empty_methods_lands_in_app() {
        let (needs, label, id, mode, meta) = login_fallback_outcome(&[]);
        assert!(!needs, "no methods at all: land in the app, not a dead login screen");
        assert!(label.is_none() && id.is_none() && meta.is_none());
        assert_eq!(mode, AuthStartMode::Pending);
    }

    #[test]
    fn login_fallback_with_api_key_advertised_lands_in_app() {
        use xai_grok_shell::agent::auth_method::xai_api_key_auth_method;
        let methods = vec![xai_api_key_auth_method()];
        let (needs, _, id, _, _) = login_fallback_outcome(&methods);
        assert!(!needs, "failed api_key must not promote to interactive login");
        assert!(id.is_none());
    }

    #[test]
    fn login_fallback_without_interactive_method_lands_in_app() {
        use xai_grok_shell::agent::auth_method::cached_token_auth_method;
        let methods = vec![cached_token_auth_method()];
        let (needs, _, id, _, _) = login_fallback_outcome(&methods);
        assert!(
            !needs,
            "cached token failed and no interactive login is advertised: land in app",
        );
        assert!(id.is_none());
    }

    #[test]
    fn login_fallback_with_interactive_method_blocks_on_login() {
        use xai_grok_shell::agent::auth_method::oidc_auth_method;
        let methods = vec![oidc_auth_method("https://sso.example.com", None)];
        let (needs, label, id, mode, _) = login_fallback_outcome(&methods);
        assert!(needs, "explicitly configured interactive login still gates");
        assert!(label.is_some() && id.is_some());
        assert_eq!(mode, AuthStartMode::Pending);
    }
```

- [x] **Step 8.2: Run to verify failure**

```bash
cargo test -p xai-grok-pager --lib acp::tests::login_fallback
```

Expected: compile FAIL — `login_fallback_outcome` not found.

- [x] **Step 8.3: Implement the helper and rewire eager_auth_or_login_fallback**

Insert above `eager_auth_or_login_fallback` (after `find_interactive_login_method`, ~line 644):

```rust
/// Outcome when eager auth failed or no methods were advertised: decide
/// whether to block on the login screen. Pure and unit-testable.
///
/// - `xai.api_key` advertised: non-interactive credentials existed; a failed
///   api_key must not open a browser — land in the app.
/// - No interactive method advertised (groky removes the zero-config
///   grok.com login): land in the app; the first turn surfaces the auth
///   error and the welcome screen shows a credentials hint.
/// - Otherwise (explicitly configured OIDC/provider login): block on the
///   login screen with that method.
fn login_fallback_outcome(
    auth_methods: &[acp::AuthMethod],
) -> (
    bool,
    Option<String>,
    Option<acp::AuthMethodId>,
    AuthStartMode,
    Option<serde_json::Value>,
) {
    let has_api_key = auth_methods
        .iter()
        .any(|m| AuthMethodKind::from_id(m.id()) == AuthMethodKind::XaiApiKey);
    if has_api_key {
        return (false, None, None, AuthStartMode::Pending, None);
    }
    let (label, method_id, mode) = find_interactive_login_method(auth_methods);
    if method_id.is_none() {
        return (false, None, None, AuthStartMode::Pending, None);
    }
    (true, label, method_id, mode, None)
}
```

In `eager_auth_or_login_fallback`, replace the empty-methods branch:

```rust
    if auth_methods.is_empty() {
        // preferred_method pin unavailable — fail closed, no invented method.
        return (true, None, None, AuthStartMode::Pending, None);
    }
```

with:

```rust
    if auth_methods.is_empty() {
        // No credentials and no interactive login (groky does not advertise
        // the zero-config grok.com method): land in the app instead of a
        // login screen with no working method.
        return login_fallback_outcome(auth_methods);
    }
```

and replace the entire `Err(_)` arm:

```rust
        Err(_) => {
            // Non-interactive credentials were advertised; shell fallthrough
            // already preferred them — do not auto-open browser login.
            let has_api_key = auth_methods
                .iter()
                .any(|m| AuthMethodKind::from_id(m.id()) == AuthMethodKind::XaiApiKey);
            if has_api_key {
                return (false, login_label, login_method_id, auth_start_mode, None);
            }
            let (label, method_id, mode) = find_interactive_login_method(auth_methods);
            (true, label, method_id, mode, None)
        }
```

with:

```rust
        Err(_) => login_fallback_outcome(auth_methods),
```

(Behavior note: in the old `has_api_key` arm, `login_label`/`login_method_id`/`auth_start_mode` were structurally `(None, None, Pending)` here because this code is only reachable when `needs_login` was false — the simplification is equivalence, not a change.)

Also update the function's doc comment (lines 646–656): replace the sentence

```
/// Empty `auth_methods` (e.g. `preferred_method=api_key` with no key) is
/// fail-closed: needs_login without an interactive method.
```

with:

```
/// Empty `auth_methods` (no credentials, and groky advertises no zero-config
/// interactive login) lands in the app; the first turn surfaces the auth
/// error and the welcome screen shows a credentials hint.
```

- [x] **Step 8.4: Run the acp tests**

```bash
cargo test -p xai-grok-pager --lib acp::
```

Expected: PASS — the four new tests plus all existing `startup_auth_*` and `shell_built_auth_methods_for_byok_user_skip_login_screen` tests.

- [x] **Step 8.5: Commit**

```bash
git add crates/codegen/xai-grok-pager/src/acp/mod.rs
git commit -m "feat(pager): land in app when no interactive login is advertised"
```

---

## Task 9: Passive credentials hint on the welcome screen

**Files:**
- Modify: `crates/codegen/xai-grok-pager/src/startup.rs`
- Modify: `crates/codegen/xai-grok-pager/src/app/event_loop.rs` (immediately after the startup-warnings assembly block that ends at line 979)

- [x] **Step 9.1: Write the failing test for the pure helper**

In `crates/codegen/xai-grok-pager/src/startup.rs`, add at the end of the file (create the test module if none exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_credentials_hint_only_when_methods_empty() {
        let hint = no_credentials_hint(true).expect("hint expected when no methods");
        assert_eq!(hint.severity, WarningSeverity::Info);
        assert!(hint.message.contains("credentials"));
        assert!(hint.action.is_some());

        assert!(no_credentials_hint(false).is_none());
    }
}
```

- [x] **Step 9.2: Run to verify failure**

```bash
cargo test -p xai-grok-pager --lib startup::
```

Expected: compile FAIL — `no_credentials_hint` not found.

- [x] **Step 9.3: Implement the helper**

Add to `startup.rs` (above the test module):

```rust
/// Hint shown on the welcome screen when no credentials are configured.
///
/// groky advertises no zero-config interactive login; instead of a blocking
/// login screen, a credential-less start lands in the app with this passive
/// hint (rendered via the startup-warnings slot on the welcome view).
pub fn no_credentials_hint(auth_methods_empty: bool) -> Option<StartupWarning> {
    if !auth_methods_empty {
        return None;
    }
    Some(StartupWarning {
        severity: WarningSeverity::Info,
        message: "No API credentials configured".to_string(),
        action: Some("set XAI_API_KEY or run /providers".to_string()),
    })
}
```

If `WarningSeverity` does not yet derive the traits the test needs, extend its derive to `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` (it already has exactly this per startup.rs:21 — no change expected).

- [x] **Step 9.4: Run the helper test**

```bash
cargo test -p xai-grok-pager --lib startup::
```

Expected: PASS.

- [x] **Step 9.5: Wire it into the event loop**

In `crates/codegen/xai-grok-pager/src/app/event_loop.rs`, directly after the closing brace of the startup-warnings block (the `{ ... app.startup_warnings = crate::diagnostics::assemble_startup_warnings(...); }` block ending at line 979 — the hint must come AFTER that assignment or it would be overwritten), insert:

```rust
    // groky: a credential-less start lands in the app (no login screen).
    // Surface a passive hint in the welcome view's startup-warning slot.
    if let Some(hint) = crate::startup::no_credentials_hint(app.auth_methods.is_empty()) {
        app.startup_warnings.push(hint);
    }
```

(`app.auth_methods` was populated from connection metadata at line 633; empty means no env key, no cached token, no BYOK provider key, and no configured interactive login.)

- [x] **Step 9.6: Build and lint the pager**

```bash
cargo check -p xai-grok-pager && cargo clippy -p xai-grok-pager --lib
```

Expected: clean.

- [x] **Step 9.7: Commit**

```bash
git add crates/codegen/xai-grok-pager/src/startup.rs crates/codegen/xai-grok-pager/src/app/event_loop.rs
git commit -m "feat(pager): passive credentials hint on credential-less first run"
```

---

## Task 10: Workspace verification

**Files:** none (verification only)

- [x] **Step 10.1: Guard scripts**

```bash
./bin/check-no-network-telemetry.sh
```

Expected: `no network telemetry found`, exit 0.

- [x] **Step 10.2: Affected crate test suites**

```bash
cargo test -p xai-grok-update
cargo test -p xai-grok-shell agent::auth_method
cargo test -p xai-grok-pager --lib
```

Expected: all PASS. Notable canaries: `shell_built_auth_methods_for_byok_user_skip_login_screen` (BYOK unchanged), `pin_oidc_*` (explicit pin unchanged), `welcome_pending_*` key tests in app_view.rs (the Pending state still exists for explicitly configured logins — these tests set the state directly and must keep passing).

- [x] **Step 10.3: Full binary builds**

```bash
cargo check -p xai-grok-pager-bin
cargo build --release -p xai-grok-pager-bin
./target/release/groky --version
```

Expected: builds; prints `groky 0.1.0 ...` (bump lands in Task 11).

- [ ] **Step 10.4: Manual first-run smoke (empirical, no network)**

In a scratch HOME so real config is untouched, with no `XAI_API_KEY`:

```bash
HOME=$(mktemp -d) XAI_API_KEY= strace -f -e trace=network -o /tmp/groky-net.log ./target/release/groky || true
```

Verify: the TUI reaches the welcome screen (no "Login with grok.com / Quit" menu; the "No API credentials configured" hint shows), then quit with Ctrl+C. Then:

```bash
grep -E 'connect\(' /tmp/groky-net.log | grep -v 'AF_UNIX' || echo "NO NETWORK CONNECTIONS"
```

Expected: `NO NETWORK CONNECTIONS` (unix sockets to the local leader are fine). If `strace` is unavailable, skip the syscall check and note it in the PR.

---

## Task 11: Version bump, README note, release prep

**Files:**
- Modify: `crates/codegen/xai-grok-pager-bin/Cargo.toml:3` (`version = "0.1.0"` → `"0.1.1"`)
- Modify: `README.md` (Installation section)
- Modify: `Cargo.lock` (regenerated)

- [x] **Step 11.1: Bump the binary version**

In `crates/codegen/xai-grok-pager-bin/Cargo.toml` change:

```toml
version = "0.1.0"
```

to:

```toml
version = "0.1.1"
```

Then refresh the lockfile:

```bash
cargo check -p xai-grok-pager-bin
```

- [x] **Step 11.2: README update note**

In `README.md`, in the "One-line installer" subsection, after the sentence "The script verifies the sha256 checksum of every download.", add:

```markdown
groky never checks for updates or phones home; to update, re-run the
installer.
```

- [x] **Step 11.3: Verify version output**

```bash
cargo build --release -p xai-grok-pager-bin && ./target/release/groky --version
```

Expected: `groky 0.1.1 ...` (the release smoke test requires the tag and binary version to agree before tagging `v0.1.1`).

- [x] **Step 11.4: Commit**

```bash
git add crates/codegen/xai-grok-pager-bin/Cargo.toml Cargo.lock README.md
git commit -m "chore: bump groky to 0.1.1, document installer-based updates"
```

---

## Task 12: Integrate and release

- [x] **Step 12.1: Push the branch and open a PR** (use the finishing-a-development-branch skill; PR title: "Portable ARM builds, no login, no phone-home — v0.1.1")
- [ ] **Step 12.2: After merge, tag `v0.1.1`** — the release workflow builds, smoke-tests, and publishes assets (only on explicit user go-ahead).
- [ ] **Step 12.3: Empirical SIGILL verification** — on the Oracle Ampere A1 host: `curl -fsSL https://raw.githubusercontent.com/amanverasia/groky/main/install.sh | bash && groky --version`. Expected: `groky 0.1.1`, no illegal instruction. Optionally pre-verify the artifact locally with `qemu-aarch64 -cpu neoverse-n1 ./groky --version` if qemu-user is installed.
