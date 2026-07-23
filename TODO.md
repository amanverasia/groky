# Curated Groky backlog

Completed implementation history is preserved in `docs/superpowers/plans/`.
This file lists only currently pending project work; broad imported source
TODO/FIXME comments are not part of this backlog.

## Correctness and secret hardening

- [ ] Fix `TargetFilterLayer` so optional sampling/instrumentation layers do not
      globally suppress unrelated tracing events. See `ISSUES.md`.
- [ ] Remove generic serialization from live `SamplerConfig` and provide only an
      explicit secret-free diagnostics representation if needed. See
      `ISSUES.md`.

## Dynamic providers and Janus

- [ ] Add generic `[dynamic_provider.<id>]` TOML configuration without changing
      existing `[provider.<id>]` catalog-override semantics.
- [ ] Reject dynamic IDs that collide with bundled providers or reserved
      `xai`/`janus` identities.
- [ ] Carry configured environment-variable names through provider composition
      and resolve credentials from the effective post-override provider at
      inference time; never fall back to xAI/session credentials.
- [ ] Scope discovered-model cache reuse to provider ID plus canonical validated
      base URL so endpoint changes cannot publish stale models from an old
      origin.
- [ ] Add atomic startup/reload registration: invalid reload retains the
      last-known-good provider set and emits secret-free diagnostics.
- [ ] Give each Janus setup/result modal state accurate Enter/Esc footer labels.
- [ ] Remove unused plaintext `ProviderCatalogAdapter::session_keys` and
      `set_session_key`; supported credentials remain stored provider keys and
      configured environment references.

## Selective security adaptations

Do not merge upstream sync commits wholesale. Port and review these as focused
local changes:

- [ ] Default-deny non-public `web_fetch` destinations, with explicit literal
      loopback opt-in and redirect/DNS-rebinding protections.
- [ ] Validate plugin Git URL/ref/SHA operands and terminate Git options before
      user/config-controlled values.
- [ ] Default project LSP trust to false and propagate only verified folder
      trust into workspace-server launch.
- [ ] Harden permission auto-approval against execution-capable `rg --pre`,
      `env -S`, dangerous `kubectl`/`ps`, and recursive command construction.
- [ ] Enforce a reviewed cross-platform sensitive-file policy for direct edits
      and shell writes.
- [ ] Scope provider/session credentials to their exact approved endpoint.
- [ ] Share owner-only secure-file writes across provider/auth/MCP/crash
      sensitive artifacts.

## External release and product gates

- [ ] Manual clean-home no-credential/no-network startup smoke under syscall
      tracing.
- [ ] Create/push `v0.1.1` and verify published Linux artifacts/checksums only
      after explicit release authorization.
- [ ] Verify the published aarch64 artifact on real ARMv8/Ampere hardware with
      no SIGILL.
- [ ] Deploy groky.dev documentation/installer redirect when hosting and DNS are
      available.
- [ ] Diagnose and restore macOS release artifacts.
- [ ] Decide and validate promoted Windows release support (currently
      best-effort/untested).
