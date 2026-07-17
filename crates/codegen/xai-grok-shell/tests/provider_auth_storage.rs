//! Integration tests for provider-scoped API key storage in `auth.json`.
//!
//! Lives as an integration test (rather than in `auth/storage.rs` unit
//! tests) because the shell lib test target does not currently compile;
//! this target links against the public library API independently.

use xai_grok_shell::auth::{
    clear_provider_api_key, provider_api_key_scope, read_api_key, read_provider_api_key,
    store_api_key, store_provider_api_key,
};

#[test]
fn provider_key_roundtrip_preserves_xai_and_oauth_scopes() {
    let dir = tempfile::tempdir().unwrap();
    store_api_key(dir.path(), "xai-key").unwrap();
    store_provider_api_key(dir.path(), "openai", "openai-key").unwrap();
    assert_eq!(read_api_key(dir.path()).as_deref(), Some("xai-key"));
    assert_eq!(
        read_provider_api_key(dir.path(), "openai").as_deref(),
        Some("openai-key")
    );
    clear_provider_api_key(dir.path(), "openai").unwrap();
    assert_eq!(read_api_key(dir.path()).as_deref(), Some("xai-key"));
    assert_eq!(read_provider_api_key(dir.path(), "openai"), None);
}

#[cfg(unix)]
#[test]
fn provider_key_file_is_created_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    store_provider_api_key(dir.path(), "anthropic", "secret").unwrap();
    assert_eq!(
        std::fs::metadata(dir.path().join("auth.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn provider_scope_validates_provider_ids() {
    assert_eq!(
        provider_api_key_scope("openai").as_deref(),
        Ok("provider::openai")
    );
    assert_eq!(
        provider_api_key_scope("open_ai-2").as_deref(),
        Ok("provider::open_ai-2")
    );
    assert!(provider_api_key_scope("").is_err());
    assert!(provider_api_key_scope("OpenAI").is_err());
    assert!(provider_api_key_scope("open ai").is_err());
    assert!(provider_api_key_scope("open/ai").is_err());
}

#[test]
fn blank_keys_and_invalid_provider_ids_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    assert!(store_provider_api_key(dir.path(), "openai", "").is_err());
    assert!(store_provider_api_key(dir.path(), "openai", "   ").is_err());
    assert!(store_provider_api_key(dir.path(), "Bad Provider", "key").is_err());
    assert!(!dir.path().join("auth.json").exists());
}

#[test]
fn clearing_a_missing_provider_key_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    clear_provider_api_key(dir.path(), "openai").unwrap();
    assert_eq!(read_provider_api_key(dir.path(), "openai"), None);
}
