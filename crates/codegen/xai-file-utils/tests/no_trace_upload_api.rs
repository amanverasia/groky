//! Source-boundary deny tests for the trace-upload removal (Plan 1 Task 8).
//!
//! `xai-file-utils` retains generic product-storage APIs (upload queue for
//! remote workspace sync, GCS/S3 upload helpers for search index sync and
//! video generation). These tests assert the trace-upload-only surface does
//! not creep back in, and that shell product-storage entry points (explicit
//! feedback, share, search remote sync) stay free of trace-upload symbols
//! while their product functions survive.

use std::path::{Path, PathBuf};

fn crate_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn assert_forbidden(path: &Path, forbidden: &[&str]) {
    let source = read(path);
    for symbol in forbidden {
        assert!(
            !source.contains(symbol),
            "trace-upload API {symbol} survived in {}",
            path.display()
        );
    }
}

#[test]
fn crate_source_has_no_trace_upload_api() {
    let files = [
        "lib.rs",
        "gcs.rs",
        "queue.rs",
        "s3.rs",
        "storage_client.rs",
        "upload_config.rs",
    ];
    // Trace-only surface. `TraceExportConfig`/`TraceExportSource` are NOT
    // listed: despite the legacy names they are the generic storage config
    // and resolver used by the product upload queue (remote workspace sync)
    // and search index sync.
    let forbidden = [
        "TraceUploadAttempted",
        "TraceUploadSucceeded",
        "SESSION_TRACES_BUCKET",
        "upload_session_metadata",
        "spawn_trace_upload",
        "grok-shell-trace-upload",
    ];
    for file in files {
        let path = crate_root().join("src").join(file);
        if !path.exists() {
            continue;
        }
        assert_forbidden(&path, &forbidden);
    }
}

/// Shell sources scanned relative to this crate. The shell lib's own test
/// runs are outside this task's verification budget, so the boundary
/// assertions live here as source scans.
fn shell_src() -> PathBuf {
    crate_root().join("../xai-grok-shell/src")
}

#[test]
fn shell_auth_refresh_has_no_diagnostic_uploader() {
    for file in [
        "auth/refresh/mod.rs",
        "auth/refresh/oidc_refresher.rs",
        "auth/manager.rs",
    ] {
        assert_forbidden(
            &shell_src().join(file),
            &[
                "DiagnosticUploader",
                "spawn_diagnostic_upload",
                "with_diagnostic_upload",
            ],
        );
    }
}

#[test]
fn shell_storage_auth_wrapper_is_storage_named() {
    let path = shell_src().join("upload/gcs.rs");
    assert_forbidden(&path, &["TraceExportConfigWithAuth"]);
    let source = read(&path);
    assert!(
        source.contains("StorageExportConfigWithAuth"),
        "product-storage auth wrapper missing from upload/gcs.rs"
    );
}

#[test]
fn shell_feedback_is_explicit_submission_only() {
    let path = shell_src().join("extensions/feedback.rs");
    assert_forbidden(
        &path,
        &[
            "unified_log_url",
            "upload_session_metadata",
            "spawn_trace_upload",
            "TraceUpload",
        ],
    );
    let source = read(&path);
    assert!(
        source.contains("submit_feedback_workflow"),
        "explicit feedback submission function missing from feedback.rs"
    );
}

#[test]
fn shell_share_has_no_trace_upload() {
    let path = shell_src().join("extensions/share.rs");
    assert_forbidden(
        &path,
        &[
            "unified_log_url",
            "upload_session_metadata",
            "spawn_trace_upload",
            "TraceUpload",
        ],
    );
    let source = read(&path);
    assert!(
        source.contains("share_session") && source.contains("ExportedSession"),
        "share/export functions missing from share.rs"
    );
}
