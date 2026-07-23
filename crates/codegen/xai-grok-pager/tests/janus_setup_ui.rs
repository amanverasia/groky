//! Integration tests for the Janus setup flow inside the `/providers`
//! picker (Plan 3, Task 10).
//!
//! These live as an integration test because the pager's in-crate unit
//! test harness does not currently compile (pre-existing breakage); the
//! public lib surface is exercised instead, mirroring `providers_ui.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use xai_grok_pager::app::actions::{Action, Effect};
use xai_grok_pager::providers::{
    JANUS_DEFAULT_BASE_URL, JANUS_INSECURE_URL_WARNING, JanusSetupParams, JanusSetupResponse,
    JanusSetupState, ProviderInfo, ProviderListResponse, ProviderStatus, SecretKey,
};
use xai_grok_pager::theme::Theme;
use xai_grok_pager::views::providers_modal::{
    ProvidersModalState, ProvidersMode, ProvidersOutcome, handle_providers_key,
    render_providers_overlay, route_providers_modal_key,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// A shell provider list that does NOT include janus, as before the first
/// configuration. The modal must still surface a Janus setup row.
fn list_without_janus() -> ProviderListResponse {
    ProviderListResponse {
        providers: vec![ProviderInfo {
            provider_id: "xai".into(),
            provider_name: "xAI".into(),
            status: ProviderStatus::Configured,
        }],
        refresh_status: "fresh".into(),
        refresh_started: false,
    }
}

/// Modal in list mode with rows loaded and the janus row selected.
fn modal_on_janus_row() -> ProvidersModalState {
    let mut modal = ProvidersModalState::loading();
    modal.apply_list(Ok(list_without_janus()));
    let janus_index = modal
        .rows
        .iter()
        .position(|r| r.provider_id == "janus")
        .expect("janus setup row must exist even when the shell omits it");
    modal.selected = janus_index;
    modal
}

/// Render the modal into a buffer and return the flattened text.
fn render_to_text(state: &mut ProvidersModalState) -> String {
    render_to_lines(state).join("\n") + "\n"
}

fn render_to_lines(state: &mut ProvidersModalState) -> Vec<String> {
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let theme = Theme::default();
    render_providers_overlay(&mut buf, area, state, false, &theme);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

/// Drive the modal from the base-URL state to the API-key state using the
/// current (loopback) URL.
fn advance_to_key_screen(modal: &mut ProvidersModalState) {
    match handle_providers_key(modal, &key(KeyCode::Enter)) {
        ProvidersOutcome::Changed => {}
        _ => panic!("loopback URL must advance without confirmation"),
    }
    assert!(
        matches!(modal.mode, ProvidersMode::JanusApiKey { .. }),
        "expected JanusApiKey state"
    );
}

// ── Footer shortcuts match each Janus state ──────────────────────────

#[test]
fn janus_footer_shortcuts_match_each_state() {
    let mut modal = modal_on_janus_row();
    handle_providers_key(&mut modal, &key(KeyCode::Enter));
    let base = render_to_text(&mut modal);
    assert!(base.contains("Enter continue"), "{base}");
    assert!(base.contains("Esc back"), "{base}");
    assert!(!base.contains("Enter save"), "{base}");

    modal.mode = ProvidersMode::JanusBaseUrl {
        value: "http://192.168.1.50:20128/v1".into(),
        insecure_confirmation_required: true,
    };
    let confirm = render_to_text(&mut modal);
    assert!(confirm.contains("Enter confirm"), "{confirm}");
    assert!(confirm.contains("Esc edit URL"), "{confirm}");

    modal.mode = ProvidersMode::JanusApiKey {
        base_url: JANUS_DEFAULT_BASE_URL.into(),
        allow_insecure_http: false,
        buffer: String::new(),
    };
    let key_screen = render_to_text(&mut modal);
    assert!(key_screen.contains("Enter set up"), "{key_screen}");
    assert!(key_screen.contains("Esc back"), "{key_screen}");

    modal.mode = ProvidersMode::JanusChecking {
        base_url: JANUS_DEFAULT_BASE_URL.into(),
    };
    let checking = render_to_text(&mut modal);
    assert!(checking.contains("Esc close"), "{checking}");
    assert!(!checking.contains("Enter select"), "{checking}");

    modal.mode = ProvidersMode::JanusResult {
        message: "Ready".into(),
        cached_models: 0,
    };
    let result_lines = render_to_lines(&mut modal);
    let shortcut_line = result_lines
        .iter()
        .position(|line| line.contains("Enter/Esc back"))
        .expect("result shortcut must be rendered");
    let bottom_border = result_lines
        .iter()
        .position(|line| line.contains('└') && line.contains('┘'))
        .expect("modal bottom border must render");
    assert_eq!(
        shortcut_line + 1,
        bottom_border,
        "shortcut must be on the footer row immediately above the bottom border"
    );
    let result = result_lines.join("\n");
    assert!(!result.contains("Enter select"), "{result}");
}

#[test]
fn insecure_confirmation_escape_returns_to_editing_and_checking_escape_closes() {
    let mut modal = modal_on_janus_row();
    modal.mode = ProvidersMode::JanusBaseUrl {
        value: "http://192.168.1.50:20128/v1".into(),
        insecure_confirmation_required: true,
    };
    assert!(matches!(
        route_providers_modal_key(&mut modal, &key(KeyCode::Esc)),
        ProvidersOutcome::Changed
    ));
    assert!(matches!(
        modal.mode,
        ProvidersMode::JanusBaseUrl {
            insecure_confirmation_required: false,
            ..
        }
    ));

    modal.mode = ProvidersMode::JanusChecking {
        base_url: JANUS_DEFAULT_BASE_URL.into(),
    };
    assert!(matches!(
        route_providers_modal_key(&mut modal, &key(KeyCode::Esc)),
        ProvidersOutcome::Close
    ));
}

// ── 1. Row selection opens base-URL entry with the exact default ─────

#[test]
fn selecting_janus_opens_base_url_prefilled_with_default() {
    let mut modal = modal_on_janus_row();
    match handle_providers_key(&mut modal, &key(KeyCode::Enter)) {
        ProvidersOutcome::Changed => {}
        _ => panic!("selecting janus must switch modes"),
    }
    match &modal.mode {
        ProvidersMode::JanusBaseUrl {
            value,
            insecure_confirmation_required,
        } => {
            assert_eq!(value, "http://127.0.0.1:20128/v1");
            assert_eq!(value, JANUS_DEFAULT_BASE_URL);
            assert!(!insecure_confirmation_required);
        }
        other => panic!("expected JanusBaseUrl, got {other:?}"),
    }
}

// ── 2. Key screen hints and masking ──────────────────────────────────

#[test]
fn key_screen_shows_optional_hints_and_masks_input() {
    let mut modal = modal_on_janus_row();
    handle_providers_key(&mut modal, &key(KeyCode::Enter)); // → base URL
    advance_to_key_screen(&mut modal);

    modal.insert_str("sk-janus-topsecret");
    let text = render_to_text(&mut modal);
    assert!(
        text.contains("Optional API key"),
        "missing title hint in:\n{text}"
    );
    assert!(
        text.contains("optional, starts with sk-janus-"),
        "missing format hint in:\n{text}"
    );
    assert!(
        !text.contains("sk-janus-topsecret"),
        "plaintext key leaked into the rendered buffer"
    );
    assert_eq!(modal.rendered_key(), "*".repeat("sk-janus-topsecret".len()));
}

/// Modal in the checking state, awaiting the setup task result.
fn modal_checking() -> ProvidersModalState {
    let mut modal = modal_on_janus_row();
    modal.mode = ProvidersMode::JanusChecking {
        base_url: JANUS_DEFAULT_BASE_URL.to_string(),
    };
    modal
}

// ── 3. Failed result appends the cached-models sentence ──────────────

#[test]
fn failed_result_with_cached_models_appends_cached_sentence() {
    let mut modal = modal_checking();
    modal.apply_janus_setup(Ok(JanusSetupResponse {
        state: JanusSetupState::Failed,
        model_count: 0,
        cached_models: 2,
        message: Some("Janus is not reachable.".into()),
    }));
    match &modal.mode {
        ProvidersMode::JanusResult {
            message,
            cached_models,
        } => {
            assert_eq!(
                message,
                "Janus is not reachable. 2 cached models remain available."
            );
            assert_eq!(*cached_models, 2);
        }
        other => panic!("expected JanusResult, got {other:?}"),
    }
}

#[test]
fn failed_result_without_cached_models_shows_message_alone() {
    let mut modal = modal_checking();
    modal.apply_janus_setup(Ok(JanusSetupResponse {
        state: JanusSetupState::Failed,
        model_count: 0,
        cached_models: 0,
        message: Some("Janus is not reachable.".into()),
    }));
    match &modal.mode {
        ProvidersMode::JanusResult { message, .. } => {
            assert_eq!(message, "Janus is not reachable.");
        }
        other => panic!("expected JanusResult, got {other:?}"),
    }
}

// ── 4. Empty result string is exact ──────────────────────────────────

#[test]
fn empty_result_shows_exact_no_models_message() {
    let mut modal = modal_checking();
    modal.apply_janus_setup(Ok(JanusSetupResponse {
        state: JanusSetupState::Empty,
        model_count: 0,
        cached_models: 0,
        message: Some("shell message ignored for empty".into()),
    }));
    match &modal.mode {
        ProvidersMode::JanusResult { message, .. } => {
            assert_eq!(message, "Janus is healthy but returned no models.");
        }
        other => panic!("expected JanusResult, got {other:?}"),
    }
}

// ── 5. Insecure non-loopback http requires confirmation ──────────────

#[test]
fn insecure_url_requires_confirmation_before_any_setup_action() {
    let mut modal = modal_on_janus_row();
    handle_providers_key(&mut modal, &key(KeyCode::Enter)); // → base URL
    modal.mode = ProvidersMode::JanusBaseUrl {
        value: "http://192.168.1.50:20128/v1".into(),
        insecure_confirmation_required: false,
    };

    // First Enter: no action, no state advance — only the confirmation.
    match handle_providers_key(&mut modal, &key(KeyCode::Enter)) {
        ProvidersOutcome::Changed => {}
        ProvidersOutcome::Action(_) | ProvidersOutcome::CloseWithAction(_) => {
            panic!("no SetupJanus action may fire before confirmation")
        }
        other_outcome => {
            let _ = other_outcome;
            panic!("expected Changed outcome for confirmation prompt");
        }
    }
    match &modal.mode {
        ProvidersMode::JanusBaseUrl {
            insecure_confirmation_required,
            ..
        } => assert!(insecure_confirmation_required),
        other => panic!("must remain in JanusBaseUrl, got {other:?}"),
    }
    let text = render_to_text(&mut modal);
    // The warning may be word-wrapped by the modal; verify the full copy
    // is rendered (in order, ignoring modal borders) and pin the exact
    // wording via the constant.
    let flat: String = text
        .split_whitespace()
        .filter(|tok| !tok.chars().all(|c| ('\u{2500}'..='\u{257f}').contains(&c)))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        flat.contains(
            "This URL sends prompts and credentials over plain HTTP. \
             Continue only if you trust this network."
        ),
        "confirmation text missing in:\n{text}"
    );
    assert_eq!(
        JANUS_INSECURE_URL_WARNING,
        "This URL sends prompts and credentials over plain HTTP. \
         Continue only if you trust this network."
    );

    // Second Enter confirms and advances to the key screen.
    handle_providers_key(&mut modal, &key(KeyCode::Enter));
    match &modal.mode {
        ProvidersMode::JanusApiKey {
            allow_insecure_http,
            ..
        } => assert!(allow_insecure_http, "confirmation must set the flag"),
        other => panic!("expected JanusApiKey after confirmation, got {other:?}"),
    }

    // Submitting (empty key) carries allow_insecure_http = true.
    match handle_providers_key(&mut modal, &key(KeyCode::Enter)) {
        ProvidersOutcome::Action(Action::SetupJanus(params)) => {
            assert!(params.allow_insecure_http);
            assert_eq!(params.base_url, "http://192.168.1.50:20128/v1");
            assert!(params.api_key.is_none(), "empty key must submit as None");
        }
        _ => panic!("expected SetupJanus action after confirmed submit"),
    }
}

// ── 6. Effect Debug prints has_api_key, never the key ────────────────

#[test]
fn setup_effect_debug_redacts_the_key() {
    let params = JanusSetupParams {
        base_url: JANUS_DEFAULT_BASE_URL.to_string(),
        api_key: Some(SecretKey::new("sk-janus-topsecret".into())),
        allow_insecure_http: false,
    };
    let params_debug = format!("{params:?}");
    assert!(
        params_debug.contains("has_api_key: true"),
        "got: {params_debug}"
    );
    assert!(!params_debug.contains("sk-janus-topsecret"));

    let effect = Effect::SetupJanus(JanusSetupParams {
        base_url: JANUS_DEFAULT_BASE_URL.to_string(),
        api_key: Some(SecretKey::new("sk-janus-topsecret".into())),
        allow_insecure_http: false,
    });
    let effect_debug = format!("{effect:?}");
    assert!(
        effect_debug.contains("has_api_key: true"),
        "got: {effect_debug}"
    );
    assert!(!effect_debug.contains("sk-janus-topsecret"));
}

// ── 7. Ready result string is exact and the key buffer is gone ───────

#[test]
fn ready_result_shows_exact_message_and_key_is_cleared() {
    let mut modal = modal_on_janus_row();
    handle_providers_key(&mut modal, &key(KeyCode::Enter)); // → base URL
    advance_to_key_screen(&mut modal);
    modal.insert_str("sk-janus-topsecret");

    // Submit: key leaves the widget inside a SecretKey; buffer cleared.
    match handle_providers_key(&mut modal, &key(KeyCode::Enter)) {
        ProvidersOutcome::Action(Action::SetupJanus(params)) => {
            assert!(params.api_key.is_some());
            assert!(!format!("{params:?}").contains("sk-janus-topsecret"));
        }
        _ => panic!("expected SetupJanus action"),
    }
    assert!(
        matches!(modal.mode, ProvidersMode::JanusChecking { .. }),
        "submit must enter the checking state"
    );
    assert_eq!(modal.rendered_key(), "", "buffer must be cleared on submit");
    let checking_text = render_to_text(&mut modal);
    // The checking line must match the spec copy exactly (no trailing
    // ellipsis or other decoration).
    let checking_line = checking_text
        .lines()
        .find(|l| l.contains("Checking"))
        .expect("checking notice must render")
        .trim_matches(|c: char| c.is_whitespace() || c == '\u{2502}')
        .trim();
    assert_eq!(
        checking_line,
        "Checking Janus health at http://127.0.0.1:20128/v1"
    );
    assert!(!checking_text.contains('\u{2026}'));
    assert!(!checking_text.contains("sk-janus-topsecret"));
    assert!(!format!("{:?}", modal.mode).contains("sk-janus-topsecret"));

    modal.apply_janus_setup(Ok(JanusSetupResponse {
        state: JanusSetupState::Ready,
        model_count: 2,
        cached_models: 0,
        message: None,
    }));
    match &modal.mode {
        ProvidersMode::JanusResult { message, .. } => {
            assert_eq!(message, "Janus is ready. 2 models available.");
        }
        other => panic!("expected JanusResult, got {other:?}"),
    }
    let result_text = render_to_text(&mut modal);
    assert!(result_text.contains("Janus is ready. 2 models available."));
    assert!(!result_text.contains("sk-janus-topsecret"));
}

// ── Result-screen keys stay inside the modal (full key path) ─────────

/// Esc (and Enter) on the Janus result screen must return to the provider
/// list, not close the modal: `route_providers_modal_key` is the complete
/// providers key path (modal chrome first, then the inner handler), exactly
/// as `AgentView::handle_modal_key` drives it.
#[test]
fn result_screen_esc_returns_to_list_through_full_modal_path() {
    for code in [KeyCode::Esc, KeyCode::Enter] {
        let mut modal = modal_checking();
        modal.apply_janus_setup(Ok(JanusSetupResponse {
            state: JanusSetupState::Ready,
            model_count: 2,
            cached_models: 0,
            message: None,
        }));
        match route_providers_modal_key(&mut modal, &key(code)) {
            ProvidersOutcome::Changed => {}
            ProvidersOutcome::Close => {
                panic!("{code:?} on the result screen must not close the modal")
            }
            _ => panic!("expected Changed outcome for {code:?}"),
        }
        assert!(
            matches!(modal.mode, ProvidersMode::List),
            "{code:?} must return to the provider list"
        );
    }

    // Sanity: chrome Esc still closes the modal from the list itself.
    let mut modal = modal_on_janus_row();
    assert!(matches!(
        route_providers_modal_key(&mut modal, &key(KeyCode::Esc)),
        ProvidersOutcome::Close
    ));
}

// ── Late results only land while checking ────────────────────────────

/// A `JanusSetupComplete` that arrives after the user has left the
/// checking state (Esc-closed and reopened the modal, started key entry,
/// …) must not hijack the current screen.
#[test]
fn late_setup_result_is_ignored_outside_checking_state() {
    let ready = JanusSetupResponse {
        state: JanusSetupState::Ready,
        model_count: 2,
        cached_models: 0,
        message: None,
    };

    // List mode: ignored.
    let mut modal = modal_on_janus_row();
    modal.apply_janus_setup(Ok(ready.clone()));
    assert!(matches!(modal.mode, ProvidersMode::List));

    // Key-entry mode for another provider: ignored.
    let mut modal = ProvidersModalState::entering_key("openai", "OpenAI", false);
    modal.insert_str("sk-openai-secret");
    modal.apply_janus_setup(Ok(ready));
    assert!(
        matches!(modal.mode, ProvidersMode::EnteringKey { .. }),
        "late result must not replace key entry"
    );
    assert_eq!(modal.rendered_key(), "*".repeat("sk-openai-secret".len()));
}

// ── Wire format ───────────────────────────────────────────────────────

#[test]
fn janus_setup_response_parses_shell_wire_format() {
    let json = r#"{
        "state": "ready",
        "modelCount": 3,
        "cachedModels": 0,
        "message": null
    }"#;
    let resp: JanusSetupResponse = serde_json::from_str(json).expect("wire format parses");
    assert_eq!(resp.state, JanusSetupState::Ready);
    assert_eq!(resp.model_count, 3);
    assert_eq!(resp.cached_models, 0);
    assert_eq!(resp.message, None);

    let failed: JanusSetupResponse = serde_json::from_str(
        r#"{"state":"failed","modelCount":0,"cachedModels":2,"message":"Janus is not reachable."}"#,
    )
    .expect("failed state parses");
    assert_eq!(failed.state, JanusSetupState::Failed);
    let empty: JanusSetupResponse =
        serde_json::from_str(r#"{"state":"empty","modelCount":0,"cachedModels":0,"message":"m"}"#)
            .expect("empty state parses");
    assert_eq!(empty.state, JanusSetupState::Empty);
}

// ── Insecure-URL detection helper ─────────────────────────────────────

#[test]
fn insecure_url_check_flags_only_non_loopback_http() {
    use xai_grok_pager::providers::is_insecure_non_loopback_http;
    assert!(!is_insecure_non_loopback_http("http://127.0.0.1:20128/v1"));
    assert!(!is_insecure_non_loopback_http("http://localhost:20128/v1"));
    assert!(!is_insecure_non_loopback_http("http://[::1]:20128/v1"));
    assert!(!is_insecure_non_loopback_http(
        "https://janus.example.com/v1"
    ));
    assert!(is_insecure_non_loopback_http(
        "http://192.168.1.50:20128/v1"
    ));
    assert!(is_insecure_non_loopback_http("http://janus.lan/v1"));
    assert!(is_insecure_non_loopback_http("http://user@janus.lan/v1"));
}
