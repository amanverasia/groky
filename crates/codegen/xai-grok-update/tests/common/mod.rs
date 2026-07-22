//! Shared helpers for integration tests.
//!
//! Each `tests/*.rs` integration test is its own binary, so each binary has
//! its own `OnceLock<GROK_HOME>`. The helpers below ensure the per-binary
//! initialization is identical: same env-var set, same isolation guarantees,
//! same reset between tests.
//!
//! Mirrors the GROK_HOME isolation pattern used in other integration tests.
//!
//! ## Usage
//!
//! ```ignore
//! mod common;
//! use common::{test_home, reset_home};
//!
//! #[tokio::test]
//! #[serial_test::serial]
//! async fn my_test() {
//!     let _ = test_home();   // initializes GROK_HOME once per binary
//!     reset_home();          // wipes state between tests
//!     // ...
//! }
//! ```

use std::path::PathBuf;
use std::sync::OnceLock;

// ─────────────────────────────────────────────────────────────────────────────
// GROK_HOME isolation
// ─────────────────────────────────────────────────────────────────────────────

/// Returns a process-wide test `GROK_HOME`, initialized exactly once per test
/// binary. Once initialized, `xai_grok_config::grok_home()` will resolve to
/// this directory for the lifetime of the process.
///
/// Also clears env vars that the auto-update code consults so a parent shell's
/// values can't pollute the baseline (e.g. running tests from `npm run` would
/// otherwise inherit `npm_config_user_agent` and `NPM_TOKEN`).
pub fn test_home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.keep();
        // SAFETY: called once at OnceLock init, before any other thread touches
        // these env vars. Tests using this helper must be `#[serial]`.
        unsafe {
            std::env::set_var("GROK_HOME", &path);
            std::env::remove_var("GROK_TEST_VERSION");
            std::env::remove_var("NPM_TOKEN");
            std::env::remove_var("GROK_INSTALLER");
            std::env::remove_var("GROK_MANAGED_BY_NPM");
            std::env::remove_var("GROK_MANAGED_BY_INTERNAL");
        }
        path
    })
}

/// Wipe state in `GROK_HOME` between tests so each test sees a clean home.
/// Removes the well-known files and subdirectories the update path writes,
/// and clears env vars that individual tests may set.
pub fn reset_home() {
    let home = test_home();
    let _ = std::fs::remove_file(home.join("config.toml"));
    let _ = std::fs::remove_file(home.join("version.json"));
    let _ = std::fs::remove_file(home.join("version.json.tmp"));
    let _ = std::fs::remove_dir_all(home.join("bin"));
    let _ = std::fs::remove_dir_all(home.join("downloads"));
    // SAFETY: tests using this helper must be `#[serial]`.
    unsafe {
        std::env::remove_var("GROK_TEST_VERSION");
        std::env::remove_var("NPM_TOKEN");
        std::env::remove_var("GROK_INSTALLER");
    }
}
