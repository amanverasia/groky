//! Strict parsing for user-declared `[dynamic_provider.<id>]` TOML entries.
//!
//! Unlike the legacy `[provider.<id>]` static-override section, this section is
//! all-or-nothing: malformed shape, unknown fields, or invalid values reject the
//! entire config. The table key is the sole source of each provider ID.

use indexmap::IndexMap;
use xai_grok_catalog::DynamicProviderConfig;

/// Parses `[dynamic_provider.<id>]` entries strictly.
///
/// The table key supplies `DynamicProviderConfig::id`; an `id` field inside an
/// entry is rejected rather than allowed to duplicate or override that key.
pub(crate) fn parse_dynamic_providers(
    raw_config: &toml::Value,
) -> Result<IndexMap<String, DynamicProviderConfig>, String> {
    let Some(section) = raw_config.get("dynamic_provider") else {
        return Ok(IndexMap::new());
    };
    let table = section.as_table().ok_or_else(|| {
        format!(
            "`dynamic_provider` must be a table of [dynamic_provider.<id>] entries, got {}",
            section.type_str()
        )
    })?;

    let mut providers = IndexMap::with_capacity(table.len());
    for (id, value) in table {
        let entry = value.as_table().ok_or_else(|| {
            format!(
                "`dynamic_provider.{id}` must be a table, got {}",
                value.type_str()
            )
        })?;
        if entry.contains_key("id") {
            return Err(format!(
                "`dynamic_provider.{id}` must not contain `id`; the table key supplies the provider ID"
            ));
        }

        let mut entry = entry.clone();
        entry.insert("id".to_owned(), toml::Value::String(id.clone()));
        let provider = toml::Value::Table(entry)
            .try_into::<DynamicProviderConfig>()
            .map_err(|error| format!("invalid `dynamic_provider.{id}`: {error}"))?;
        providers.insert(id.clone(), provider);
    }
    Ok(providers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xai_grok_catalog::Protocol;

    fn parse_cfg(toml_str: &str) -> crate::agent::config::Config {
        let raw: toml::Value = toml::from_str(toml_str).unwrap();
        crate::agent::config::Config::new_from_toml_cfg(&raw).expect("config should parse")
    }

    fn assert_config_error(toml_str: &str, expected: &str) {
        let raw: toml::Value = toml::from_str(toml_str).unwrap();
        let error = crate::agent::config::Config::new_from_toml_cfg(&raw)
            .expect_err("config must be rejected");
        assert!(error.contains(expected), "{error}");
    }

    #[test]
    fn parses_dynamic_provider_with_static_models() {
        let config = parse_cfg(
            r#"
            [dynamic_provider.gateway]
            name = "Gateway"
            base_url = "https://gateway.example/v1"
            env_vars = ["GATEWAY_TOKEN", "GATEWAY_TOKEN_FALLBACK"]
            api_backend = "responses"
            discover = true

            [dynamic_provider.gateway.models."org/model"]
            name = "Model"
            api_backend = "chat_completions"
            context_window = 128000
            reasoning = true
            "#,
        );

        let provider = config.dynamic_providers.get("gateway").unwrap();
        assert_eq!(provider.id.as_str(), "gateway");
        assert_eq!(
            provider.env_vars,
            ["GATEWAY_TOKEN", "GATEWAY_TOKEN_FALLBACK"]
        );
        assert_eq!(provider.protocol, Protocol::Responses);
        assert!(provider.discover);
        let model = provider
            .models
            .iter()
            .find_map(|(id, model)| (id.as_str() == "org/model").then_some(model))
            .unwrap();
        assert_eq!(model.protocol, Some(Protocol::ChatCompletions));
        assert_eq!(model.context_window, Some(128000));
        assert_eq!(model.reasoning, Some(true));
    }

    #[test]
    fn rejects_bad_section_and_entry_shapes() {
        assert_config_error(
            "dynamic_provider = false",
            "`dynamic_provider` must be a table",
        );
        assert_config_error(
            r#"
            [dynamic_provider]
            gateway = false
            "#,
            "`dynamic_provider.gateway` must be a table",
        );
    }

    #[test]
    fn rejects_inner_id() {
        assert_config_error(
            r#"
            [dynamic_provider.gateway]
            id = "other"
            name = "Gateway"
            base_url = "https://gateway.example/v1"
            "#,
            "must not contain `id`",
        );
    }

    #[test]
    fn rejects_missing_unknown_and_invalid_values() {
        assert_config_error(
            r#"
            [dynamic_provider.gateway]
            base_url = "https://gateway.example/v1"
            "#,
            "missing field `name`",
        );
        assert_config_error(
            r#"
            [dynamic_provider.gateway]
            name = "Gateway"
            base_url = "https://gateway.example/v1"
            typo = true
            "#,
            "unknown field `typo`",
        );
        assert_config_error(
            r#"
            [dynamic_provider.gateway]
            name = "Gateway"
            base_url = "not a URL"
            "#,
            "invalid base URL",
        );
    }

    #[test]
    fn provider_overrides_remain_lenient_and_are_not_dynamic_providers() {
        let config = parse_cfg(
            r#"
            [provider.existing]
            name = "Static override"
            env_key = "EXISTING_TOKEN"
            ignored_by_static_override = true
            "#,
        );

        assert!(config.dynamic_providers.is_empty());
        let override_config = config.config_providers.get("existing").unwrap();
        assert_eq!(override_config.name.as_deref(), Some("Static override"));
        assert_eq!(
            override_config.env_key.as_ref().unwrap().primary(),
            Some("EXISTING_TOKEN")
        );
        assert!(
            config
                .provider_override_warnings
                .iter()
                .any(|warning| { warning.field.as_deref() == Some("ignored_by_static_override") })
        );
    }
}
