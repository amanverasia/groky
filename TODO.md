# TODO

## Plan 3: Dynamic Providers and Janus — DONE

Executed `docs/superpowers/plans/2026-07-16-dynamic-providers-and-janus.md`
(12 tasks) on `feature/dynamic-providers`. Landed: bounded dynamic-provider
config + discovery in `xai-grok-catalog` (URL security policy, manual
redirects with cross-origin credential stripping, 2 MiB / 2000-model bounds),
secret-free last-known-good dynamic model cache, Janus preset + setup flow
(`/providers` → `x.ai/providers/setup_janus`, persisted in secret-free
`$GROK_HOME/dynamic_providers.json`), sampler credential redaction, refresh
orchestration (24h staleness gate, forced refresh, in-flight coalescing),
end-to-end Janus tests, and `docs/configuration/providers.md`.

## Plan 3 follow-ups (not landed)

- Generic `[provider.<id>]` TOML-configured dynamic providers are not wired
  into the shell's config loading: the catalog crate validates/deserializes
  `DynamicProviderConfig` from TOML and the shell exposes
  `ProviderCatalogAdapter::configure_dynamic`, but nothing registers dynamic
  providers declared in `config.toml`. Janus (via `/providers`) is the only
  end-to-end dynamic provider today. See "Known limitations" in
  `docs/configuration/providers.md`.
- Providers modal: the `JanusResult` screen relies on Enter/Esc but the modal
  footer shortcut labels do not describe them.
- Dynamic model cache is keyed by provider id only; after a base-URL change,
  stale models from the old URL are served until the next successful refresh.
- A dynamic provider reusing a catalog provider id replaces that catalog
  entry, dropping its env-var credential resolution.
- Inference-time env-var fallback (`catalog_provider_env_credential`) uses the
  embedded catalog only; stamp `entry.env_key` from the post-override provider
  during composition so runtime-refreshed providers resolve correctly.
- CI catalog check fetches live models.dev (nondeterministic); consider
  checking against a committed raw input instead.
- `ProviderCatalogAdapter` doc claims "holds no secrets" while `session_keys`
  holds plaintext; wire or remove `set_session_key` (currently no production
  caller).
- Pre-existing broken lib test targets (`xai-grok-shell` ~31 errors,
  `xai-grok-pager` ~169 errors) in the published snapshot: repairing them
  would let the deferred in-crate unit tests run.
- Pre-existing bug: enabling `GROK_LOG_SAMPLING` suppresses all other tracing
  process-wide (`TargetFilterLayer::enabled` global veto,
  xai-grok-telemetry/src/instrumentation.rs); use per-layer `Filtered`.

## Other pending decisions

- Merge `feature/remove-telemetry` and `feature/provider-catalog` into `main`.
- Wire `bin/check-no-network-telemetry.sh` into CI once CI exists for the fork.
