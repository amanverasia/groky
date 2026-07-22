//! Generic startup warnings displayed on the welcome screen.
//!
//! Any subsystem (terminal diagnostics, auth, config migration, etc.) can
//! produce [`StartupWarning`]s.

/// A non-fatal startup warning from any subsystem.
///
/// This is a **display contract only** -- the subsystem formats the message
/// and optional action hint. Detailed diagnostics (fix commands, config paths)
/// live in the subsystem-specific slash commands (e.g. `/terminal-setup`).
#[derive(Debug, Clone)]
pub struct StartupWarning {
    /// Severity controls rendering color (yellow for warnings, dim for info).
    pub severity: WarningSeverity,
    /// Short, user-facing message (fits in ~60 columns).
    pub message: String,
    /// Optional action hint (e.g. "run /terminal-setup").
    pub action: Option<String>,
}

/// Severity level for startup warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningSeverity {
    /// Rendered in warning color (yellow). Something is misconfigured.
    Warning,
    /// Rendered in dim/gray. Informational, not actionable.
    Info,
}

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
