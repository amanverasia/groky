//! Integration tests for the `/providers` picker (Task 11).
//!
//! These live as an integration test because the pager's in-crate unit
//! test harness does not currently compile (pre-existing breakage); the
//! public lib surface is exercised instead.

use std::sync::Arc;

use xai_grok_pager::app::actions::Action;
use xai_grok_pager::providers::{
    ProviderInfo, ProviderListResponse, ProviderStatus, SecretKey, provider_rows,
};
use xai_grok_pager::slash::command::SlashCommand;
use xai_grok_pager::slash::commands::{builtin_commands, providers::ProvidersCommand};
use xai_grok_pager::slash::registry::CommandRegistry;
use xai_grok_pager::views::providers_modal::{
    ProvidersModalState, ProvidersOutcome, handle_providers_key,
};

fn sample_provider_state() -> ProviderListResponse {
    ProviderListResponse {
        providers: vec![
            ProviderInfo {
                provider_id: "xai".into(),
                provider_name: "xAI".into(),
                status: ProviderStatus::Configured,
            },
            ProviderInfo {
                provider_id: "openai".into(),
                provider_name: "OpenAI".into(),
                status: ProviderStatus::Environment,
            },
            ProviderInfo {
                provider_id: "anthropic".into(),
                provider_name: "Anthropic".into(),
                status: ProviderStatus::MissingKey,
            },
            ProviderInfo {
                provider_id: "legacy".into(),
                provider_name: "Legacy".into(),
                status: ProviderStatus::Unavailable,
            },
        ],
        refresh_status: "fresh".into(),
        refresh_started: false,
    }
}

#[test]
fn providers_command_owns_login_alias() {
    let cmd = ProvidersCommand;
    assert_eq!(cmd.name(), "providers");
    assert_eq!(cmd.aliases(), &["login"]);
    // NOTE: `cmd.run(...) == Action::OpenProviders` is asserted in the
    // #[cfg(test)] module of src/slash/commands/providers.rs; it cannot be
    // exercised here because `CommandExecCtx` uses crate-private types.
}

#[test]
fn login_alias_resolves_to_providers_in_registry() {
    let reg = CommandRegistry::new(builtin_commands());
    let login = reg.get("login").expect("/login must resolve");
    assert_eq!(login.name(), "providers", "/login must alias /providers");
    let providers = reg.get("providers").expect("/providers must resolve");
    assert_eq!(providers.name(), "providers");
    // The old standalone LoginCommand must be gone: exactly one command
    // may own the name/alias "login", and it is /providers.
    let owners: Vec<_> = builtin_commands()
        .iter()
        .filter(|c| c.name() == "login" || c.aliases().contains(&"login"))
        .map(|c| c.name().to_string())
        .collect();
    assert_eq!(owners, vec!["providers".to_string()]);
    let _: Vec<Arc<dyn SlashCommand>> = builtin_commands();
}

#[test]
fn modal_rows_show_all_four_states_without_secrets() {
    let rows = provider_rows(&sample_provider_state());
    assert_eq!(
        rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(),
        ["Configured", "Environment", "Missing key", "Unavailable"]
    );
    // Unavailable rows are disabled; all others are selectable.
    assert_eq!(
        rows.iter().map(|r| r.disabled).collect::<Vec<_>>(),
        [false, false, false, true]
    );
    assert!(!format!("{rows:?}").contains("secret"));
}

#[test]
fn key_input_is_masked_and_submit_discards_plaintext() {
    let mut modal = ProvidersModalState::entering_key("openai", "OpenAI", false);
    modal.insert_str("sk-openai-secret");
    assert_eq!(modal.rendered_key(), "****************");

    let action = modal.submit().expect("non-empty key must submit");
    match &action {
        Action::StoreProviderKey { provider_id, .. } => assert_eq!(provider_id, "openai"),
        other => panic!("expected StoreProviderKey, got {other:?}"),
    }
    // Buffer is cleared immediately on submit.
    assert_eq!(modal.rendered_key(), "");
    // The key must never surface through Debug formatting.
    assert!(!format!("{action:?}").contains("sk-openai-secret"));
}

#[test]
fn empty_key_does_not_submit() {
    let mut modal = ProvidersModalState::entering_key("openai", "OpenAI", true);
    assert!(modal.submit().is_none());
    modal.insert_str("   ");
    assert!(modal.submit().is_none());
}

#[test]
fn secret_key_debug_and_effect_debug_are_redacted() {
    let key = SecretKey::new("sk-super-secret".into());
    let debug = format!("{key:?}");
    assert!(!debug.contains("sk-super-secret"), "leaked: {debug}");
    assert!(debug.contains("redacted"), "got: {debug}");

    let action = Action::StoreProviderKey {
        provider_id: "openai".into(),
        api_key: SecretKey::new("sk-super-secret".into()),
    };
    assert!(!format!("{action:?}").contains("sk-super-secret"));
}

#[test]
fn provider_list_response_parses_shell_wire_format() {
    let json = r#"{
        "providers": [
            {"providerId": "xai", "providerName": "xAI", "status": "configured"},
            {"providerId": "openai", "providerName": "OpenAI", "status": "missing_key"},
            {"providerId": "zed", "providerName": "Zed", "status": "environment"},
            {"providerId": "old", "providerName": "Old", "status": "unavailable"}
        ],
        "refreshStatus": "stale",
        "refreshStarted": true
    }"#;
    let resp: ProviderListResponse = serde_json::from_str(json).expect("wire format parses");
    assert_eq!(resp.providers.len(), 4);
    assert_eq!(resp.providers[0].status, ProviderStatus::Configured);
    assert_eq!(resp.providers[1].status, ProviderStatus::MissingKey);
    assert_eq!(resp.providers[2].status, ProviderStatus::Environment);
    assert_eq!(resp.providers[3].status, ProviderStatus::Unavailable);
    assert_eq!(resp.refresh_status, "stale");
    assert!(resp.refresh_started);
}

/// The `r` key inside `/providers` is the explicit user refresh: it must
/// request a *forced* (unconditional) catalog refresh, unlike the
/// staleness-gated refresh fired on picker open.
#[test]
fn explicit_refresh_key_requests_forced_refresh() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut modal = ProvidersModalState::loading();
    modal.loading = false;
    let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
    match handle_providers_key(&mut modal, &key) {
        ProvidersOutcome::Action(Action::RefreshProviders { force }) => {
            assert!(force, "explicit refresh must set force: true");
        }
        _ => panic!("expected RefreshProviders action"),
    }
}
