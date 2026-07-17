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
use crate::providers::{ProviderListResponse, ProviderRowView, ProviderStatus, SecretKey};
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

    /// Append text to the key buffer (typing or paste). No-op in list mode.
    pub fn insert_str(&mut self, s: &str) {
        if let ProvidersMode::EnteringKey { buffer, .. } = &mut self.mode {
            buffer.push_str(s);
        }
    }

    /// Masked rendering of the key buffer: one `*` per character.
    pub fn rendered_key(&self) -> String {
        match &self.mode {
            ProvidersMode::EnteringKey { buffer, .. } => "*".repeat(buffer.chars().count()),
            ProvidersMode::List => String::new(),
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
                self.rows = crate::providers::provider_rows(&list);
                self.refresh_status = list.refresh_status;
                self.error = None;
                self.clamp_selection();
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Apply a `x.ai/providers/update` broadcast (replacement rows).
    pub fn apply_update(&mut self, rows: Vec<ProviderRowView>) {
        self.loading = false;
        self.rows = rows;
        self.clamp_selection();
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

/// Keyboard handling for the providers modal (chrome Esc/close is routed
/// by the caller first; this sees everything else).
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
    let entering = matches!(state.mode, ProvidersMode::EnteringKey { .. });
    let shortcuts: Vec<Shortcut> = if entering {
        vec![
            Shortcut {
                label: "Enter save",
                clickable: false,
                id: 0,
            },
            Shortcut {
                label: "Esc back",
                clickable: false,
                id: 0,
            },
        ]
    } else {
        vec![
            Shortcut {
                label: "\u{2191}/\u{2193} nav",
                clickable: false,
                id: 0,
            },
            Shortcut {
                label: "Enter select",
                clickable: false,
                id: 0,
            },
            Shortcut {
                label: "x clear key",
                clickable: false,
                id: 0,
            },
            Shortcut {
                label: "r refresh",
                clickable: false,
                id: 0,
            },
            Shortcut {
                label: "Esc close",
                clickable: false,
                id: 0,
            },
        ]
    };
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
