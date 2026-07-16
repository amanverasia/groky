//! Integration tests for the provider-aware `/model` picker (Task 12).
//!
//! Lives as an integration test because the pager's in-crate unit test
//! harness does not currently compile (pre-existing breakage); the public
//! lib surface is exercised instead.

use std::sync::Arc;

use agent_client_protocol as acp;
use xai_grok_pager::acp::model_state::ModelState;
use xai_grok_pager::providers::catalog_notice_for_status;
use xai_grok_pager::slash::commands::model::{build_model_items, scored_model_items};

fn provider_model(
    id: &str,
    name: &str,
    provider_id: &str,
    provider_name: &str,
    context: u64,
    reasoning: bool,
) -> (acp::ModelId, acp::ModelInfo) {
    let model_id = acp::ModelId::new(Arc::from(id));
    let mut meta = serde_json::Map::new();
    meta.insert("providerId".into(), provider_id.into());
    meta.insert("providerName".into(), provider_name.into());
    meta.insert("totalContextTokens".into(), context.into());
    if reasoning {
        meta.insert("supportsReasoningEffort".into(), true.into());
    }
    let info = acp::ModelInfo::new(model_id.clone(), name.to_string()).meta(Some(meta));
    (model_id, info)
}

/// Three models across two providers, inserted deliberately out of the
/// expected output order. `current` selects which model is active.
fn provider_model_state(current: &str) -> ModelState {
    let mut state = ModelState::default();
    for (id, info) in [
        provider_model(
            "openai/gpt-5-mini",
            "GPT-5 Mini",
            "openai",
            "OpenAI",
            400_000,
            false,
        ),
        provider_model("openai/gpt-5", "GPT-5", "openai", "OpenAI", 400_000, true),
        provider_model(
            "anthropic/claude-sonnet-4",
            "Claude Sonnet 4",
            "anthropic",
            "Anthropic",
            200_000,
            true,
        ),
    ] {
        state.available.insert(id, info);
    }
    state.current = Some(acp::ModelId::new(Arc::from(current)));
    state
}

#[test]
fn empty_query_orders_current_then_provider_and_model_id() {
    let items = build_model_items(&provider_model_state("anthropic/claude-sonnet-4"));
    assert_eq!(
        items
            .iter()
            .map(|i| i.insert_text.trim())
            .collect::<Vec<_>>(),
        ["Claude Sonnet 4", "GPT-5", "GPT-5 Mini"]
    );
}

#[test]
fn empty_query_puts_current_first_even_when_provider_sorts_later() {
    let items = build_model_items(&provider_model_state("openai/gpt-5"));
    assert_eq!(items[0].insert_text.trim(), "GPT-5");
    // Remaining rows are (provider_id, model_id) ascending.
    assert_eq!(
        items[1..]
            .iter()
            .map(|i| i.insert_text.trim())
            .collect::<Vec<_>>(),
        ["Claude Sonnet 4", "GPT-5 Mini"]
    );
}

#[test]
fn direct_provider_model_match_ranks_above_display_name_proxy() {
    let state = provider_model_state("anthropic/claude-sonnet-4");
    let items = scored_model_items(&state, "openai/gpt-5");
    assert!(!items.is_empty());
    assert_eq!(items[0].model_id.as_deref(), Some("openai/gpt-5"));
    assert!(
        items[0].explicit_score > items[1].explicit_score,
        "qualified exact match must outrank fuzzy proxies: {:?} vs {:?}",
        items[0].explicit_score,
        items[1].explicit_score
    );
}

#[test]
fn bare_model_id_and_provider_id_direct_matches_rank_first() {
    let state = provider_model_state("anthropic/claude-sonnet-4");
    // Bare model id (already provider-qualified in this catalog).
    let by_model = scored_model_items(&state, "openai/gpt-5-mini");
    assert_eq!(by_model[0].model_id.as_deref(), Some("openai/gpt-5-mini"));
    // Provider id matches every openai row ahead of anthropic rows.
    let by_provider = scored_model_items(&state, "anthropic");
    assert_eq!(
        by_provider[0].model_id.as_deref(),
        Some("anthropic/claude-sonnet-4")
    );
}

#[test]
fn row_contains_provider_context_and_reasoning() {
    let items = build_model_items(&provider_model_state("openai/gpt-5"));
    let row = items
        .into_iter()
        .find(|i| i.model_id.as_deref() == Some("openai/gpt-5"))
        .expect("gpt-5 row");
    assert!(row.display.contains("OpenAI"), "display: {}", row.display);
    assert!(
        row.description.contains("openai/gpt-5"),
        "description: {}",
        row.description
    );
    assert!(
        row.description.contains("400k context"),
        "description: {}",
        row.description
    );
    assert!(
        row.description.contains("reasoning"),
        "description: {}",
        row.description
    );
    // Insertion text is unchanged: display name (+ trailing space for
    // reasoning models to chain into the effort sub-menu).
    assert_eq!(row.insert_text, "GPT-5 ");
    // Match text carries newline-separated provider id, provider/model,
    // model id, and display name.
    let lines: Vec<&str> = row.match_text.lines().collect();
    assert!(lines.contains(&"openai"), "match_text: {}", row.match_text);
    assert!(
        lines.contains(&"openai/gpt-5"),
        "match_text: {}",
        row.match_text
    );
    assert!(lines.contains(&"GPT-5"), "match_text: {}", row.match_text);
}

#[test]
fn non_provider_models_render_without_provider_decoration() {
    let mut state = ModelState::default();
    let id = acp::ModelId::new(Arc::from("grok-4.5"));
    state
        .available
        .insert(id.clone(), acp::ModelInfo::new(id, "Grok 4.5".to_string()));
    let items = build_model_items(&state);
    assert_eq!(items[0].display, "Grok 4.5");
    assert_eq!(items[0].model_id.as_deref(), Some("grok-4.5"));
    assert_eq!(items[0].explicit_score, None);
}

#[test]
fn background_update_keeps_current_and_reports_cached_notice() {
    let mut state = provider_model_state("openai/gpt-5");
    let refreshed = provider_model_state("openai/gpt-5");
    state.apply_catalog_update(
        refreshed.available.clone(),
        refreshed.current.clone(),
        catalog_notice_for_status("cachedAfterError"),
    );
    assert_eq!(state.current_model_id_str(), Some("openai/gpt-5"));
    assert_eq!(
        state.catalog_notice.as_deref(),
        Some("Using cached provider catalog; refresh failed")
    );
}

#[test]
fn catalog_notice_maps_refresh_status_strings() {
    assert_eq!(catalog_notice_for_status("fresh"), None);
    assert_eq!(
        catalog_notice_for_status("refreshing").as_deref(),
        Some("Refreshing provider catalog")
    );
    assert_eq!(
        catalog_notice_for_status("stale").as_deref(),
        Some("Refreshing provider catalog")
    );
    assert_eq!(
        catalog_notice_for_status("cachedAfterError").as_deref(),
        Some("Using cached provider catalog; refresh failed")
    );
}
