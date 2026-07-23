//! `/providers` picker modal: provider rows with availability status,
//! masked API-key entry, key clearing, and catalog refresh.
//!
//! Key text lives only in [`ProvidersMode::EnteringKey`]'s buffer, is
//! rendered masked, and leaves this module only inside
//! [`crate::providers::SecretKey`] (redacted `Debug`). The buffer is
//! cleared immediately on submit.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::actions::Action;
use crate::providers::{
    JANUS_DEFAULT_BASE_URL, JANUS_INSECURE_URL_WARNING, JanusSetupParams, JanusSetupResponse,
    ProviderListResponse, ProviderRowView, ProviderStatus, SecretKey,
    is_insecure_non_loopback_http, janus_result_message,
};
use crate::theme::Theme;
use crate::views::modal_window::{
    self as mw, ModalSizing, ModalWindowConfig, ModalWindowState, Shortcut,
};

/// Sub-mode of the providers modal.
pub enum ProvidersMode {
    /// Browsing the provider list.
    List,
    /// Typing an API key for one provider. The buffer holds plaintext and
    /// must never be formatted; `Debug` below redacts it.
    EnteringKey {
        provider_id: String,
        provider_name: String,
        /// Plaintext key buffer — rendered masked, cleared on submit.
        buffer: String,
        /// True when replacing an existing key (display only).
        replace: bool,
    },
    /// Janus setup step 1: editable base URL. When the URL is plain HTTP
    /// to a non-loopback host, `insecure_confirmation_required` gates
    /// progress behind an explicit second Enter.
    JanusBaseUrl {
        value: String,
        insecure_confirmation_required: bool,
    },
    /// Janus setup step 2: optional API key. The buffer holds plaintext
    /// and must never be formatted; `Debug` below redacts it.
    JanusApiKey {
        base_url: String,
        allow_insecure_http: bool,
        /// Plaintext key buffer — rendered masked, cleared on submit.
        buffer: String,
    },
    /// Janus setup step 3: health probe in flight.
    JanusChecking { base_url: String },
    /// Janus setup finished; shows the outcome message.
    JanusResult {
        message: String,
        cached_models: usize,
    },
}

impl std::fmt::Debug for ProvidersMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::List => f.write_str("List"),
            Self::EnteringKey {
                provider_id,
                replace,
                ..
            } => f
                .debug_struct("EnteringKey")
                .field("provider_id", provider_id)
                .field("replace", replace)
                .field("buffer", &"\u{ab}redacted\u{bb}")
                .finish(),
            Self::JanusBaseUrl {
                value,
                insecure_confirmation_required,
            } => f
                .debug_struct("JanusBaseUrl")
                .field("value", value)
                .field(
                    "insecure_confirmation_required",
                    insecure_confirmation_required,
                )
                .finish(),
            Self::JanusApiKey {
                base_url,
                allow_insecure_http,
                ..
            } => f
                .debug_struct("JanusApiKey")
                .field("base_url", base_url)
                .field("allow_insecure_http", allow_insecure_http)
                .field("buffer", &"\u{ab}redacted\u{bb}")
                .finish(),
            Self::JanusChecking { base_url } => f
                .debug_struct("JanusChecking")
                .field("base_url", base_url)
                .finish(),
            Self::JanusResult {
                message,
                cached_models,
            } => f
                .debug_struct("JanusResult")
                .field("message", message)
                .field("cached_models", cached_models)
                .finish(),
        }
    }
}

/// Outcome of a key/mouse event inside the providers modal.
pub enum ProvidersOutcome {
    Unchanged,
    Changed,
    /// Close the modal.
    Close,
    /// Dispatch an action; the modal stays open (store/clear/refresh).
    Action(Action),
    /// Close the modal, then dispatch (xAI row → OAuth login flow).
    CloseWithAction(Action),
}

/// State for the `/providers` picker modal.
pub struct ProvidersModalState {
    pub rows: Vec<ProviderRowView>,
    /// Concise refresh status: `fresh`, `stale`, `refreshing`, `cachedAfterError`.
    pub refresh_status: String,
    /// True until the first `x.ai/providers/list` response lands.
    pub loading: bool,
    /// Sanitized load error, shown in place of rows.
    pub error: Option<String>,
    pub selected: usize,
    pub mode: ProvidersMode,
    pub window: ModalWindowState,
}

impl ProvidersModalState {
    /// Fresh modal in loading state (rows arrive via `ListProviders`).
    pub fn loading() -> Self {
        Self {
            rows: Vec::new(),
            refresh_status: String::new(),
            loading: true,
            error: None,
            selected: 0,
            mode: ProvidersMode::List,
            window: ModalWindowState::new(),
        }
    }

    /// Modal opened directly in key-entry mode (also the unit-test seam).
    pub fn entering_key(provider_id: &str, provider_name: &str, replace: bool) -> Self {
        let mut state = Self::loading();
        state.loading = false;
        state.mode = ProvidersMode::EnteringKey {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            buffer: String::new(),
            replace,
        };
        state
    }

    /// Append text to the focused text input (typing or paste): the key
    /// buffers or the Janus base URL. No-op elsewhere.
    pub fn insert_str(&mut self, s: &str) {
        match &mut self.mode {
            ProvidersMode::EnteringKey { buffer, .. }
            | ProvidersMode::JanusApiKey { buffer, .. } => buffer.push_str(s),
            ProvidersMode::JanusBaseUrl {
                value,
                insecure_confirmation_required,
            } if !*insecure_confirmation_required => value.push_str(s),
            _ => {}
        }
    }

    /// Masked rendering of the key buffer: one `*` per character.
    pub fn rendered_key(&self) -> String {
        match &self.mode {
            ProvidersMode::EnteringKey { buffer, .. }
            | ProvidersMode::JanusApiKey { buffer, .. } => "*".repeat(buffer.chars().count()),
            _ => String::new(),
        }
    }

    /// Submit the typed key: moves the plaintext out of the buffer into a
    /// [`SecretKey`] and clears the buffer immediately. Returns `None` for
    /// blank input or in list mode.
    pub fn submit(&mut self) -> Option<Action> {
        if let ProvidersMode::EnteringKey {
            provider_id,
            buffer,
            ..
        } = &mut self.mode
        {
            let key = std::mem::take(buffer);
            if key.trim().is_empty() {
                return None;
            }
            return Some(Action::StoreProviderKey {
                provider_id: provider_id.clone(),
                api_key: SecretKey::new(key),
            });
        }
        None
    }

    /// Apply a `x.ai/providers/list` task result.
    pub fn apply_list(&mut self, result: Result<ProviderListResponse, String>) {
        self.loading = false;
        match result {
            Ok(list) => {
                let mut rows = crate::providers::provider_rows(&list);
                crate::providers::ensure_janus_row(&mut rows);
                self.rows = rows;
                self.refresh_status = list.refresh_status;
                self.error = None;
                self.clamp_selection();
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Apply a `x.ai/providers/update` broadcast (replacement rows).
    pub fn apply_update(&mut self, mut rows: Vec<ProviderRowView>) {
        self.loading = false;
        crate::providers::ensure_janus_row(&mut rows);
        self.rows = rows;
        self.clamp_selection();
    }

    /// Apply the `x.ai/providers/setup_janus` task result: swap the
    /// checking spinner for the outcome message. Errors (transport or
    /// shell-side) render like a failure with no cached models. A late
    /// result that arrives when the modal is no longer in the checking
    /// state (user closed/reopened it, started key entry, …) is ignored
    /// so it cannot hijack an unrelated screen.
    pub fn apply_janus_setup(&mut self, result: Result<JanusSetupResponse, String>) {
        if !matches!(self.mode, ProvidersMode::JanusChecking { .. }) {
            return;
        }
        let (message, cached_models) = match result {
            Ok(resp) => {
                let cached = resp.cached_models;
                (janus_result_message(&resp), cached)
            }
            Err(e) => (e, 0),
        };
        self.mode = ProvidersMode::JanusResult {
            message,
            cached_models,
        };
    }

    /// Update one row's status after a store/clear result.
    pub fn apply_key_status(&mut self, provider_id: &str, status: ProviderStatus) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.provider_id == provider_id) {
            row.status = status;
            row.disabled = status == ProviderStatus::Unavailable;
        }
    }

    fn clamp_selection(&mut self) {
        if self.rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.rows.len() {
            self.selected = self.rows.len() - 1;
        }
    }

    fn move_selection(&mut self, down: bool) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        let len = self.rows.len();
        let next = if down {
            (self.selected + 1).min(len - 1)
        } else {
            self.selected.saturating_sub(1)
        };
        if next != self.selected {
            self.selected = next;
            true
        } else {
            false
        }
    }
}

/// Complete key routing for the providers modal, exactly as
/// `AgentView::handle_modal_key` drives it: modes that own every key
/// (text entry, and the Janus result screen whose Enter/Esc return to the
/// list) bypass the window chrome; all other modes let chrome handle
/// close (Esc / close button) first, then fall through to
/// [`handle_providers_key`].
///
/// `JanusChecking` deliberately stays on the chrome path: Esc during a
/// hung health probe closes the modal as an escape hatch, and a late
/// result is then discarded by [`ProvidersModalState::apply_janus_setup`].
pub fn route_providers_modal_key(
    state: &mut ProvidersModalState,
    key: &KeyEvent,
) -> ProvidersOutcome {
    let owns_all_keys = matches!(
        state.mode,
        ProvidersMode::EnteringKey { .. }
            | ProvidersMode::JanusBaseUrl { .. }
            | ProvidersMode::JanusApiKey { .. }
            | ProvidersMode::JanusResult { .. }
    );
    if !owns_all_keys {
        let chrome_cfg = ModalWindowConfig {
            title: "",
            tabs: None,
            shortcuts: &[],
            sizing: ModalSizing::default(),
            fold_info: None,
        };
        if matches!(
            mw::handle_modal_key(&mut state.window, key, &chrome_cfg),
            mw::ModalWindowOutcome::CloseRequested
        ) {
            return ProvidersOutcome::Close;
        }
    }
    handle_providers_key(state, key)
}

/// Keyboard handling for the providers modal (chrome Esc/close is routed
/// by [`route_providers_modal_key`] first; this sees everything else).
pub fn handle_providers_key(state: &mut ProvidersModalState, key: &KeyEvent) -> ProvidersOutcome {
    match &mut state.mode {
        ProvidersMode::EnteringKey { buffer, .. } => match key.code {
            KeyCode::Esc => {
                buffer.clear();
                state.mode = ProvidersMode::List;
                ProvidersOutcome::Changed
            }
            KeyCode::Enter => match state.submit() {
                Some(action) => {
                    state.mode = ProvidersMode::List;
                    ProvidersOutcome::Action(action)
                }
                None => ProvidersOutcome::Unchanged,
            },
            KeyCode::Backspace => {
                buffer.pop();
                ProvidersOutcome::Changed
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                buffer.clear();
                ProvidersOutcome::Changed
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                buffer.push(c);
                ProvidersOutcome::Changed
            }
            _ => ProvidersOutcome::Unchanged,
        },
        ProvidersMode::JanusBaseUrl {
            value,
            insecure_confirmation_required,
        } => match key.code {
            KeyCode::Esc => {
                if *insecure_confirmation_required {
                    // Back out of the confirmation to keep editing.
                    *insecure_confirmation_required = false;
                } else {
                    state.mode = ProvidersMode::List;
                }
                ProvidersOutcome::Changed
            }
            KeyCode::Enter => {
                let url = value.trim().to_string();
                if url.is_empty() {
                    return ProvidersOutcome::Unchanged;
                }
                let insecure = is_insecure_non_loopback_http(&url);
                if insecure && !*insecure_confirmation_required {
                    // First Enter on an insecure URL only reveals the
                    // confirmation; nothing is sent anywhere.
                    *insecure_confirmation_required = true;
                    return ProvidersOutcome::Changed;
                }
                // Loopback/https, or ConfirmInsecureProviderUrl (second
                // Enter): proceed to the optional key screen.
                state.mode = ProvidersMode::JanusApiKey {
                    base_url: url,
                    allow_insecure_http: insecure,
                    buffer: String::new(),
                };
                ProvidersOutcome::Changed
            }
            KeyCode::Backspace if !*insecure_confirmation_required => {
                value.pop();
                ProvidersOutcome::Changed
            }
            KeyCode::Char('u')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !*insecure_confirmation_required =>
            {
                value.clear();
                ProvidersOutcome::Changed
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !*insecure_confirmation_required =>
            {
                value.push(c);
                ProvidersOutcome::Changed
            }
            _ => ProvidersOutcome::Unchanged,
        },
        ProvidersMode::JanusApiKey {
            base_url,
            allow_insecure_http,
            buffer,
        } => match key.code {
            KeyCode::Esc => {
                buffer.clear();
                let value = base_url.clone();
                state.mode = ProvidersMode::JanusBaseUrl {
                    value,
                    insecure_confirmation_required: false,
                };
                ProvidersOutcome::Changed
            }
            KeyCode::Enter => {
                // Move the plaintext out of the widget immediately; an
                // empty/blank key means "leave any stored key unchanged".
                let raw = std::mem::take(buffer);
                let api_key = if raw.trim().is_empty() {
                    None
                } else {
                    Some(SecretKey::new(raw))
                };
                let params = JanusSetupParams {
                    base_url: base_url.clone(),
                    api_key,
                    allow_insecure_http: *allow_insecure_http,
                };
                state.mode = ProvidersMode::JanusChecking {
                    base_url: params.base_url.clone(),
                };
                ProvidersOutcome::Action(Action::SetupJanus(params))
            }
            KeyCode::Backspace => {
                buffer.pop();
                ProvidersOutcome::Changed
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                buffer.clear();
                ProvidersOutcome::Changed
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                buffer.push(c);
                ProvidersOutcome::Changed
            }
            _ => ProvidersOutcome::Unchanged,
        },
        ProvidersMode::JanusChecking { .. } => ProvidersOutcome::Unchanged,
        ProvidersMode::JanusResult { .. } => match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                state.mode = ProvidersMode::List;
                ProvidersOutcome::Changed
            }
            _ => ProvidersOutcome::Unchanged,
        },
        ProvidersMode::List => match key.code {
            KeyCode::Esc => ProvidersOutcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                if state.move_selection(false) {
                    ProvidersOutcome::Changed
                } else {
                    ProvidersOutcome::Unchanged
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.move_selection(true) {
                    ProvidersOutcome::Changed
                } else {
                    ProvidersOutcome::Unchanged
                }
            }
            KeyCode::Enter => {
                let Some(row) = state.rows.get(state.selected) else {
                    return ProvidersOutcome::Unchanged;
                };
                if row.disabled {
                    return ProvidersOutcome::Unchanged;
                }
                if row.provider_id == "xai" {
                    // xAI stays on the dedicated OAuth/default login flow.
                    return ProvidersOutcome::CloseWithAction(Action::Login);
                }
                if row.provider_id == "janus" {
                    // Janus is configured by URL + optional key, not a
                    // bare key: enter the guided setup flow.
                    state.mode = ProvidersMode::JanusBaseUrl {
                        value: JANUS_DEFAULT_BASE_URL.to_string(),
                        insecure_confirmation_required: false,
                    };
                    return ProvidersOutcome::Changed;
                }
                let replace = row.status != ProviderStatus::MissingKey;
                state.mode = ProvidersMode::EnteringKey {
                    provider_id: row.provider_id.clone(),
                    provider_name: row.provider_name.clone(),
                    buffer: String::new(),
                    replace,
                };
                ProvidersOutcome::Changed
            }
            KeyCode::Char('x') => {
                let Some(row) = state.rows.get(state.selected) else {
                    return ProvidersOutcome::Unchanged;
                };
                if row.provider_id != "xai" && row.status.has_stored_key() {
                    ProvidersOutcome::Action(Action::ClearProviderKey {
                        provider_id: row.provider_id.clone(),
                    })
                } else {
                    ProvidersOutcome::Unchanged
                }
            }
            KeyCode::Char('r') => {
                ProvidersOutcome::Action(Action::RefreshProviders { force: true })
            }
            _ => ProvidersOutcome::Unchanged,
        },
    }
}

fn status_color(theme: &Theme, status: ProviderStatus) -> ratatui::style::Color {
    match status {
        ProviderStatus::Configured => theme.accent_success,
        ProviderStatus::Environment => theme.accent_user,
        ProviderStatus::MissingKey => theme.warning,
        ProviderStatus::Unavailable => theme.gray_dim,
    }
}

/// Greedy word-wrap for plain notice text (warning/result lines).
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn refresh_status_label(raw: &str) -> Option<String> {
    match raw {
        "" | "fresh" => None,
        "refreshing" => Some("Refreshing provider catalog\u{2026}".to_string()),
        "cachedAfterError" => Some("Using cached provider catalog; refresh failed".to_string()),
        other => Some(format!("Catalog: {other}")),
    }
}

/// Render the providers modal overlay.
pub fn render_providers_overlay(
    buf: &mut Buffer,
    area: Rect,
    state: &mut ProvidersModalState,
    compact: bool,
    theme: &Theme,
) {
    let labels: &[&str] = match &state.mode {
        ProvidersMode::EnteringKey { .. } => &["Enter save", "Esc back"],
        ProvidersMode::JanusBaseUrl {
            insecure_confirmation_required: true,
            ..
        } => &["Enter confirm", "Esc edit URL"],
        ProvidersMode::JanusBaseUrl { .. } => &["Enter continue", "Esc back"],
        ProvidersMode::JanusApiKey { .. } => &["Enter set up", "Esc back"],
        ProvidersMode::JanusChecking { .. } => &["Esc close"],
        ProvidersMode::JanusResult { .. } => &["Enter/Esc back"],
        ProvidersMode::List => &[
            "\u{2191}/\u{2193} nav",
            "Enter select",
            "x clear key",
            "r refresh",
            "Esc close",
        ],
    };
    let shortcuts: Vec<Shortcut> = labels
        .iter()
        .map(|label| Shortcut {
            label,
            clickable: false,
            id: 0,
        })
        .collect();
    let modal_config = ModalWindowConfig {
        title: "Providers",
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing {
            width_pct: 0.60,
            max_width: 90,
            min_width: 44,
            v_margin: 4,
            h_pad: 2,
            v_pad: 1,
            footer_lines: 2,
        }
        .with_compact(compact),
        fold_info: None,
    };
    let Some(mca) = mw::render_modal_window(buf, area, &mut state.window, &modal_config, theme)
    else {
        return;
    };
    let content = mca.content;
    let mut y = content.y;
    let max_y = content.y + content.height;
    let dim = Style::default().fg(theme.gray_dim);

    match &state.mode {
        ProvidersMode::EnteringKey {
            provider_name,
            replace,
            ..
        } => {
            let verb = if *replace { "Replace" } else { "Enter" };
            let title = format!("{verb} API key for {provider_name}");
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        title,
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    )),
                    content.width,
                );
            }
            y += 2;
            if y < max_y {
                let masked = state.rendered_key();
                let shown = if masked.is_empty() {
                    Span::styled("(paste or type key)", dim)
                } else {
                    Span::styled(masked, Style::default().fg(theme.text_primary))
                };
                buf.set_line(content.x, y, &Line::from(shown), content.width);
            }
            y += 2;
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        "The key is stored locally and never displayed.",
                        dim,
                    )),
                    content.width,
                );
            }
        }
        ProvidersMode::JanusBaseUrl {
            value,
            insecure_confirmation_required,
        } => {
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        "Set up Janus (local): base URL",
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    )),
                    content.width,
                );
            }
            y += 2;
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        value.clone(),
                        Style::default().fg(theme.text_primary),
                    )),
                    content.width,
                );
            }
            y += 2;
            if *insecure_confirmation_required {
                for chunk in wrap_plain(JANUS_INSECURE_URL_WARNING, content.width as usize) {
                    if y >= max_y {
                        break;
                    }
                    buf.set_line(
                        content.x,
                        y,
                        &Line::from(Span::styled(chunk, Style::default().fg(theme.warning))),
                        content.width,
                    );
                    y += 1;
                }
                if y < max_y {
                    buf.set_line(
                        content.x,
                        y,
                        &Line::from(Span::styled("Enter confirm \u{2022} Esc edit URL", dim)),
                        content.width,
                    );
                }
            } else if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        "Enter to continue \u{2022} Esc to go back",
                        dim,
                    )),
                    content.width,
                );
            }
        }
        ProvidersMode::JanusApiKey { .. } => {
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        "Optional API key for Janus",
                        Style::default()
                            .fg(theme.text_primary)
                            .add_modifier(Modifier::BOLD),
                    )),
                    content.width,
                );
            }
            y += 2;
            if y < max_y {
                let masked = state.rendered_key();
                let shown = if masked.is_empty() {
                    Span::styled("(leave blank to skip)", dim)
                } else {
                    Span::styled(masked, Style::default().fg(theme.text_primary))
                };
                buf.set_line(content.x, y, &Line::from(shown), content.width);
            }
            y += 2;
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled("optional, starts with sk-janus-", dim)),
                    content.width,
                );
            }
        }
        ProvidersMode::JanusChecking { base_url } => {
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        format!("Checking Janus health at {base_url}"),
                        Style::default().fg(theme.text_primary),
                    )),
                    content.width,
                );
            }
        }
        ProvidersMode::JanusResult { message, .. } => {
            for chunk in wrap_plain(message, content.width as usize) {
                if y >= max_y {
                    break;
                }
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(chunk, Style::default().fg(theme.text_primary))),
                    content.width,
                );
                y += 1;
            }
            y += 1;
            if y < max_y {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled("Return to providers", dim)),
                    content.width,
                );
            }
        }
        ProvidersMode::List => {
            if state.loading {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled("Loading providers\u{2026}", dim)),
                    content.width,
                );
                return;
            }
            if let Some(err) = &state.error {
                buf.set_line(
                    content.x,
                    y,
                    &Line::from(Span::styled(
                        err.clone(),
                        Style::default().fg(theme.accent_error),
                    )),
                    content.width,
                );
                return;
            }
            let name_w = state
                .rows
                .iter()
                .map(|r| r.provider_name.len())
                .max()
                .unwrap_or(0)
                .max(8);
            for (i, row) in state.rows.iter().enumerate() {
                if y >= max_y {
                    break;
                }
                let selected = i == state.selected;
                let row_bg = if selected {
                    theme.bg_visual
                } else {
                    theme.bg_base
                };
                let row_rect = Rect {
                    x: content.x,
                    y,
                    width: content.width,
                    height: 1,
                };
                buf.set_style(row_rect, Style::default().bg(row_bg));
                let name_style = if row.disabled {
                    Style::default().fg(theme.gray_dim).bg(row_bg)
                } else {
                    Style::default()
                        .fg(theme.text_primary)
                        .bg(row_bg)
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        })
                };
                let status_style = Style::default()
                    .fg(if row.disabled {
                        theme.gray_dim
                    } else {
                        status_color(theme, row.status)
                    })
                    .bg(row_bg);
                let suffix = if row.disabled {
                    "  Unsupported protocol or authentication"
                } else {
                    ""
                };
                let line = Line::from(vec![
                    Span::styled(format!("{:<name_w$}  ", row.provider_name), name_style),
                    Span::styled(row.status.as_str(), status_style),
                    Span::styled(suffix, Style::default().fg(theme.gray_dim).bg(row_bg)),
                ]);
                buf.set_line(content.x, y, &line, content.width);
                y += 1;
            }
            if let Some(notice) = refresh_status_label(&state.refresh_status) {
                y += 1;
                if y < max_y {
                    buf.set_line(
                        content.x,
                        y,
                        &Line::from(Span::styled(notice, dim)),
                        content.width,
                    );
                }
            }
        }
    }
}
