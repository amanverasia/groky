//! ACP `_meta` parsing and session-map helpers.
//!
//! These lived in the (now removed) trace-upload turn module but are
//! generic agent/session utilities with no upload coupling.

/// Parse `_meta.agentProfile` as a JSON object or string name.
/// Returns `None` if absent or invalid.
pub(crate) fn parse_agent_profile_from_meta(
    meta: Option<&agent_client_protocol::Meta>,
) -> Option<xai_grok_agent::AgentDefinition> {
    let value = meta?.get("agentProfile")?;
    if value.is_object() {
        return match xai_grok_agent::AgentDefinition::from_json(value) {
            Ok(def) => {
                tracing::info!(
                    agent_name = % def.name,
                    "Using ACP agent profile from _meta.agentProfile (JSON object)"
                );
                Some(def)
            }
            Err(e) => {
                tracing::error!(
                    error = % e,
                    "Failed to parse _meta.agentProfile JSON object, falling back to default agent"
                );
                None
            }
        };
    }
    if let Some(name) = value.as_str() {
        tracing::info!(
            agent_name = % name, "Resolving agent from _meta.agentProfile (string name)"
        );
        return xai_grok_agent::discovery::by_name(name);
    }
    tracing::warn!(
        "Ignoring _meta.agentProfile: expected a JSON object or string, got {:?}",
        value
    );
    None
}
/// Parse `_meta.askUserQuestion` as a boolean.
///
/// `Some(false)` means the pager set `--no-ask-user`; the shell propagates
/// it to `AgentBuilder::with_ask_user_question_enabled(false)` so the tool
/// is stripped from the model's advertised tool list. `Some(true)` explicitly
/// enables the tool for this session. `None` means the field is absent — the
/// caller falls back to `AgentConfig::resolve_ask_user_question()` (default ON).
pub(crate) fn parse_ask_user_question_from_meta(
    meta: Option<&agent_client_protocol::Meta>,
) -> Option<bool> {
    let value = meta?.get("askUserQuestion")?;
    match value.as_bool() {
        Some(b) => Some(b),
        None => {
            tracing::warn!(
                "Ignoring _meta.askUserQuestion: expected a bool, got {:?}",
                value
            );
            None
        }
    }
}
/// Look up a session's model, falling back to the agent default.
pub(crate) fn lookup_session_model(
    sessions: &std::collections::HashMap<
        agent_client_protocol::SessionId,
        crate::session::SessionHandle,
    >,
    session_id: Option<&agent_client_protocol::SessionId>,
    default_model_id: &agent_client_protocol::ModelId,
) -> agent_client_protocol::ModelId {
    session_id
        .and_then(|sid| sessions.get(sid).map(|h| h.model_id.clone()))
        .unwrap_or_else(|| default_model_id.clone())
}
pub(crate) fn apply_yolo_mode_to_matching_sessions(
    sessions: &mut std::collections::HashMap<
        agent_client_protocol::SessionId,
        crate::session::SessionHandle,
    >,
    sender_id: Option<&str>,
    yolo_mode: bool,
) -> usize {
    let matches_sender = |h: &crate::session::SessionHandle| -> bool {
        sender_id.is_none() || h.origin_client.as_ref().map(|c| c.product.as_str()) == sender_id
    };
    let mut updated = 0;
    for handle in sessions.values_mut() {
        if matches_sender(handle) {
            handle.yolo_mode = yolo_mode;
            let _ = handle
                .cmd_tx
                .send(crate::session::SessionCommand::SetYoloMode { enabled: yolo_mode });
            updated += 1;
        }
    }
    updated
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_ask_user_question_returns_false_when_disabled() {
        let meta = serde_json::json!({ "askUserQuestion" : false });
        assert_eq!(
            parse_ask_user_question_from_meta(meta.as_object()),
            Some(false)
        );
    }
    #[test]
    fn parse_ask_user_question_returns_true_when_enabled() {
        let meta = serde_json::json!({ "askUserQuestion" : true });
        assert_eq!(
            parse_ask_user_question_from_meta(meta.as_object()),
            Some(true)
        );
    }
    #[test]
    fn parse_ask_user_question_returns_none_when_absent() {
        let meta = serde_json::json!({ "agentProfile" : "grok-build-plan" });
        assert_eq!(parse_ask_user_question_from_meta(meta.as_object()), None);
    }
    #[test]
    fn parse_ask_user_question_returns_none_for_empty_meta() {
        assert_eq!(parse_ask_user_question_from_meta(None), None);
    }
    /// Non-bool values are ignored (defensive: the shell falls back to the
    /// resolved default via `resolve_ask_user_question` rather than panicking
    /// on malformed input).
    #[test]
    fn parse_ask_user_question_ignores_non_bool() {
        let meta = serde_json::json!({ "askUserQuestion" : "no" });
        assert_eq!(parse_ask_user_question_from_meta(meta.as_object()), None);
    }
}
