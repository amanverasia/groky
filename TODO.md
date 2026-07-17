# TODO

## Plan 3: Dynamic Providers and Janus (pending)

Execute `docs/superpowers/plans/2026-07-16-dynamic-providers-and-janus.md` (12 tasks)
using subagent-driven development, on a branch created from `feature/provider-catalog`.

Suggested kickoff prompt for a fresh session:

> Execute docs/superpowers/plans/2026-07-16-dynamic-providers-and-janus.md on a
> branch from feature/provider-catalog using subagent-driven development; first
> commit fixes the unconditional refresh on picker open.

### Scope (approved spec: docs/superpowers/specs/2026-07-16-telemetry-and-model-providers-design.md)

- Generic OpenAI-compatible `GET /models` discovery for user-configured providers
  (`discover = true` under `[provider.<id>]`), bounded parsing (2 MiB body,
  2000 models, field-length caps), static + discovered model merge.
- URL security policy: HTTPS required; plain HTTP loopback-only unless
  `allow_insecure_http = true`; manual redirects (max 5) that strip the bearer
  credential on cross-origin hops.
- Secret-free last-known-good dynamic model cache (`~/.grok/dynamic_models.json`),
  24h freshness; failed refresh never removes cached models.
- Janus preset: id `janus`, base `http://127.0.0.1:20128/v1`, chat_completions,
  health `GET /v1/health`, discovery `GET /v1/models`, optional key with
  `sk-janus-` hint; setup flow in `/providers`; exact model IDs preserved
  (`openai/gpt-4o`, combo names like `best-effort`); metadata enrichment from
  the bundled catalog by exact ID match.
- Before provider keys reach the sampler: fix raw `api_key` Debug/tracing
  leakage in `xai-grok-sampler` (plan Task 1).
- End-to-end Janus test: discovery -> model selection -> chat completion with
  correct bearer and unchanged model ID; PTY setup smoke test.
- Docs: `docs/configuration/providers.md`.

### First commit on the Plan 3 branch (from Plan 2 final audit)

1. `Effect::RefreshProviders` fired on model-picker open must respect the 24h
   staleness gate (`refresh_if_stale`) instead of unconditionally calling
   `adapter.refresh()`; keep unconditional refresh only for the explicit
   refresh key in `/providers`.

### Carry-over minor findings (fold into Plan 3 tasks where natural)

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
