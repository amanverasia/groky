# Providers

How the shell discovers model providers, where API keys live, and how the
dynamic-provider (Janus) flow works.

## Provider catalog

The shell composes its provider/model list from layered sources:

- **Bundled snapshot.** A models.dev catalog snapshot is embedded in the
  binary, so the provider list works offline out of the box.
- **On-disk cache.** `$GROK_HOME/provider_catalog.json` caches the upstream
  catalog (`https://models.dev/api.json`) with 24-hour freshness. Opening the
  model picker refreshes only when the cache is stale; the explicit refresh
  key (`r`) in `/providers` always re-fetches.
- **Dynamic providers.** Runtime-registered providers (today: Janus) layer
  their discovered models on top of the bundled+cache catalog.
- **Config patches.** `[provider.*]` / `[model.*]` overrides from config
  apply last, during composition.

### The `/providers` UI

`/providers` lists every catalog provider, shows whether a credential is
available and where it comes from, and lets you:

- store an API key for a provider (masked entry),
- clear a stored key,
- force a catalog refresh (`r`),
- set up the Janus local provider.

### Credentials

Provider API keys are stored in `$GROK_HOME/auth.json` under a
`provider::<id>` scope. The file is written atomically and kept owner-only
(`0600`); a failed write rolls back without loosening permissions.

Credential resolution precedence, at request time:

1. **Session key** — set for the current process only, never persisted.
2. **Stored key** — the `provider::<id>` entry in `auth.json`.
3. **Environment variable** — the provider's documented env var(s) from the
   catalog (e.g. `OPENAI_API_KEY`).

Dynamic providers declare no environment variable names, so for them the
effective order is session > stored. For providers marked `unauthenticated`
a key is *optional*, not forbidden: if a key exists it is sent (discovery,
health checks, and inference all follow this rule); if none exists, requests
go out without an `Authorization` header.

## Janus

Janus is a locally hosted OpenAI-compatible gateway. The shell ships a
built-in preset for it:

| Setting             | Value                              |
| ------------------- | ---------------------------------- |
| Provider id         | `janus`                            |
| Protocol            | OpenAI chat completions            |
| Default base URL    | `http://127.0.0.1:20128/v1`        |
| Health check        | `GET /v1/health`                   |
| Model discovery     | `GET /v1/models`                   |
| API key             | Optional (hint: starts `sk-janus-`) |

### Setup flow

From `/providers`, choose the Janus row:

1. **Base URL** — edit or accept the default. If the URL uses plain HTTP on
   a non-loopback host, the UI warns ("This URL sends prompts and
   credentials over plain HTTP...") and requires explicit confirmation,
   which sets `allow_insecure_http` for this provider.
2. **API key (optional)** — leave empty to keep any previously stored key
   unchanged; entering one stores it under `provider::janus` in `auth.json`.
3. **Health check** — the shell probes `/v1/health`, then discovers models
   from `/v1/models` and reports the outcome (ready with N models, healthy
   but empty, or an actionable, secret-free failure message).

### Model IDs are preserved exactly

Discovered Janus model IDs are kept verbatim — `openai/gpt-4o`,
`best-effort`, and other combo names are not rewritten. In the model picker
they appear qualified as `janus/<id>` (e.g. `janus/openai/gpt-4o`), but the
upstream request always carries the original unprefixed ID. When a
discovered ID exactly matches a bundled-catalog model, its metadata (context
window, reasoning support, cost) enriches the entry.

### Persistence

Successful setup writes `$GROK_HOME/dynamic_providers.json`, which is
deliberately secret-free — it stores only the provider `id`, `base_url`, and
`allow_insecure_http` flag. Keys live in `auth.json`; discovered models live
in the dynamic cache. On startup the shell re-registers providers from this
file (currently only the `janus` id is recognized; a corrupt or
unknown-schema file is logged and ignored, never fatal).

## Security policy for dynamic endpoints

All dynamic-provider URLs (base, models, health) are validated **before**
the provider is registered, so a provider cannot pass discovery and later
send prompts or credentials to a disallowed inference URL:

- **HTTPS is always allowed.** Plain HTTP is allowed only for loopback
  addresses (or the literal host `localhost`) unless the user explicitly
  opted in via the setup flow (`allow_insecure_http`).
- **Embedded userinfo (`http://user:pass@...`) is always rejected**, and
  URLs are userinfo-redacted before they can appear in any error message or
  log line.
- **Redirects are followed manually, at most 5 hops**, and the bearer
  credential is stripped on any cross-origin hop.
- **Discovery responses are bounded**: 2 MiB body cap, at most 2000 models,
  with per-field length caps; discovery times out after 10 s, health checks
  after 3 s.
- Unauthenticated providers send a key only if one is stored; the key text
  never appears in logs, errors, or on-disk metadata.

## Dynamic model cache semantics

Discovered models are cached (secret-free: model IDs and names only) in
`$GROK_HOME/dynamic_models_cache.json`, written atomically:

- **Successful discovery replaces the provider's cache entry** — including a
  successful *empty* list, which the UI reports as "Janus is healthy but
  returned no models."
- **Failed refresh never removes cached models.** The last-known-good list
  (plus any statically declared models) is published instead, regardless of
  its age, and the event is marked as served-from-cache.
- **Staleness gate**: background refresh runs when a provider's cache entry
  is missing or older than 24 h. The explicit refresh key (`r`) in
  `/providers` forces re-discovery for all dynamic providers. At most 4
  refreshes run concurrently, and per-provider refreshes coalesce.

## Known limitations

- **Generic `[provider.<id>]` TOML-configured dynamic providers are not yet
  wired.** The catalog crate can validate and deserialize dynamic-provider
  TOML (`discover = true`, static `models`, endpoint overrides), and the
  shell exposes a programmatic `ProviderCatalogAdapter::configure_dynamic`
  API, but the shell's config loading does not yet register dynamic
  providers declared in `config.toml`. Today the only end-to-end dynamic
  provider is the Janus preset via `/providers`.
- **A dynamic provider that reuses a catalog provider id replaces that
  catalog entry** in the composed snapshot. Because dynamic providers carry
  no env-var names, environment-variable credential resolution for the
  shadowed catalog provider stops working.
- **The dynamic cache is keyed by provider id only.** After changing a
  provider's base URL, previously discovered models are still served until
  the next successful refresh against the new URL.
