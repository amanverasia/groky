//! Shell-side adapter that threads the live `AuthManager` through to the
//! `StorageClient` constructed inside `xai_file_utils::gcs::*` helpers.
//!
//! Background: the storage helpers (`upload_bytes`, `upload_file`,
//! `upload_stream`, `upload_bytes_signed`) build a `StorageClient` per call.
//! Without a `StorageConfig` impl that provides `proxy_credentials` /
//! `proxy_attribution`, that client falls back to a static `user_token`
//! snapshot baked into `TraceExportConfig` at construction time and emits no
//! attribution event on 401. That snapshot becomes stale on rotation and is
//! empty during the 5-minute pre-refresh buffer window in `AuthManager`, both
//! of which manifest as `POST /v1/storage` 401s at the proxy.
//!
//! [`TraceExportConfigWithAuth`] wraps a bare `TraceExportConfig` plus an
//! optional `Arc<AuthManager>` and implements `StorageConfig` such that, when
//! the manager is present, the constructed `StorageClient` gets:
//!
//!   1. A refresh-aware `ShellAuthCredentialProvider` (live token from
//!      `auth_manager.current()`, falling back to `expired_auth()` so
//!      the buffer window is covered), and
//!   2. A `StorageClientAttributionBridge` that emits the
//!      `auth_401_attribution` event on 401 with the right consumer tag.
//!
//! Retained for non-telemetry product storage (e.g. the session-search
//! remote index sync); the trace/diagnostic upload pipeline that used to
//! live alongside it has been removed.
use crate::auth::AuthManager;
use crate::auth::credential_provider::{
    ShellAuthCredentialProvider, StorageClientAttributionBridge,
};
use std::sync::Arc;
use xai_file_utils::gcs::StorageConfig;
use xai_file_utils::storage_client::Auth401AttributionCallback;
use xai_file_utils::{TraceExportConfig, UploadMethod};
use xai_grok_auth::AuthCredentialProvider;
/// Owned wrapper that pairs a `TraceExportConfig` with an optional live
/// `AuthManager`. See module docs for why this exists; in short, it's the
/// shell-side adapter that lets `xai_file_utils::gcs::*` helpers wire
/// refresh-aware credentials and 401-attribution into the per-call
/// `StorageClient` they construct internally.
///
/// `auth_manager == None` is supported (for tests, direct-mode upload,
/// and a few sites without an `AuthManager` in scope) and degrades to
/// the pre-existing snapshot-based behavior.
#[derive(Clone)]
pub(crate) struct TraceExportConfigWithAuth {
    inner: TraceExportConfig,
    auth_manager: Option<Arc<AuthManager>>,
}
impl TraceExportConfigWithAuth {
    pub(crate) fn new(inner: TraceExportConfig, auth_manager: Option<Arc<AuthManager>>) -> Self {
        Self {
            inner,
            auth_manager,
        }
    }
}
impl StorageConfig for TraceExportConfigWithAuth {
    fn bucket_url(&self) -> &str {
        self.inner.bucket_url()
    }
    fn upload_method(&self) -> &UploadMethod {
        self.inner.upload_method()
    }
    fn proxy_credentials(&self) -> Option<Arc<dyn AuthCredentialProvider>> {
        let am = self.auth_manager.as_ref()?;
        let UploadMethod::Proxy {
            deployment_key,
            alpha_test_key,
            ..
        } = &self.inner.upload_method
        else {
            return None;
        };
        Some(Arc::new(ShellAuthCredentialProvider::new(
            am.clone(),
            deployment_key.clone(),
            alpha_test_key.clone(),
        )))
    }
    fn proxy_attribution(&self) -> Option<Arc<dyn Auth401AttributionCallback>> {
        let am = self.auth_manager.as_ref()?;
        if !matches!(self.inner.upload_method, UploadMethod::Proxy { .. }) {
            return None;
        }
        Some(Arc::new(StorageClientAttributionBridge::new(
            am.clone(),
            None,
        )))
    }
    fn proxy_http_client(&self) -> Option<reqwest::Client> {
        Some(crate::http::shared_upload_client())
    }
}
/// Convenience trait for wrapping a `TraceExportConfig` at upload call
/// sites. Pattern:
///
/// ```ignore
/// xai_file_utils::gcs::upload_bytes(
///     &gcs_config.with_auth(Some(auth_manager.clone())),
///     ...,
/// ).await
/// ```
///
/// At sites without an `AuthManager` in scope, pass `None` (degrades to
/// snapshot behavior; same as calling the helper with the bare config).
pub(crate) trait WithAuth {
    fn with_auth(&self, auth_manager: Option<Arc<AuthManager>>) -> TraceExportConfigWithAuth;
}
impl WithAuth for TraceExportConfig {
    fn with_auth(&self, auth_manager: Option<Arc<AuthManager>>) -> TraceExportConfigWithAuth {
        TraceExportConfigWithAuth::new(self.clone(), auth_manager)
    }
}
