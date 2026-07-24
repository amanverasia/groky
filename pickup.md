# Groky cross-machine handoff

> **Authoritative branch:** `main`
>
> **Remote:** `https://github.com/amanverasia/groky.git`
>
> **Current merged head:** `1581eb3ff6593fe5bf1801d50edc8eafa30d1ca3`
>
> **Release metadata:** pager and pager-bin are `0.1.1`
>
> **Release status:** prepared, not tagged or published

## What has shipped to `main`

Merged PRs in this work sequence:

- #9 — portable generic ARM Linux builds, credential-less first run, inert
  local-only updater, canonical Groky branding, checksummed installers, and
  required general CI.
- #10 — v0.1.1 README and release-facing package preparation.
- #11 — post-merge plan/TODO/issue/handoff reconciliation.
- #12 — optional tracing target filters no longer globally suppress sibling
  tracing layers.
- #13 — live `SamplerConfig` is no longer generically serializable; diagnostics
  use a sanitized internal snapshot and sanitized endpoint identity.
- #14 — canonical dynamic-provider URL identity, same-origin path-only endpoint
  overrides, ordered environment credential names, and origin-scoped dynamic
  cache behavior.
- #15 — transactional dynamic-provider mutation/composition and real inference
  credential/wire-contract coverage.
- #16 — strict `[dynamic_provider.<id>]` TOML parsing.
- #17 — startup activation with static/fresh-cache hydration and nonblocking
  stale discovery.
- #18 — atomic live reload across dynamic provider state and ModelsManager
  publication, with rollback and watcher coverage.
- #19 — accurate Janus modal footer shortcuts and route tests.
- #20 — removal of the unused plaintext provider session-key cache and obsolete
  resolution abstractions; provider order is stored → environment → explicit
  model, while xAI legacy session behavior remains isolated.

Current `main` and `origin/main` are synchronized at `1581eb3`.

## Verification already completed

- Required GitHub `general-ci` jobs (`validate`, `test`, `installer`) passed on
  every merged PR.
- Provider catalog checks passed where relevant.
- Focused local tests covered tracing, sampler secret redaction, dynamic config,
  cache origin, provider inference, startup/reload, Janus modal behavior, and
  provider credential precedence.
- Release-preparation build reported `groky 0.1.1`; updater and installer E2E
  tests passed before PR #10.

## Pending curated work

Use `TODO.md` and `ISSUES.md` as the canonical current backlog. The next planned
work is selective security hardening, one focused branch/PR at a time:

1. `security/web-fetch-ssrf-boundaries`
   - default-deny non-public destinations;
   - explicit literal-loopback opt-in;
   - redirect-hop and DNS-rebinding protections.
2. `security/plugin-git-operand-validation`
   - validate Git URL/ref/SHA operands before spawn/filesystem mutation;
   - option terminators and immutable/unattended pin policy.
3. `security/workspace-lsp-trust-default-deny`
   - propagate verified folder trust to project LSP launch.
4. Reviewed permission/sensitive-file/provider-endpoint/secure-file hardening,
   each as a separate design and PR.

Do not merge `upstream/main` wholesale. Preserve Groky's no-phone-home,
credential-less startup, ARM portability, and provider credential isolation.

## External release gates still pending

- Manual clean-home startup with no credentials under syscall-level network
  tracing; verify normal welcome/passive hint and no unexpected outbound
  connection.
- Explicit authorization to create and push `v0.1.1`, then monitor release
  publication and verify Linux x86_64/aarch64 artifacts and checksums.
- Real ARMv8/Ampere A1 installation and no-SIGILL smoke.
- groky.dev hosting/DNS deployment.
- macOS artifact restoration and promoted Windows support decisions.

No local or remote `v0.1.1` tag/release exists at this handoff.

## Resource policy

Shared Cargo target:

```text
/home/amanverasia/.cache/groky-cargo-target
```

Latest measured size: `104,535,400,880` bytes (~103 GB).

- Run one Cargo command at a time.
- Use `CARGO_BUILD_JOBS=2` and `nice -n 10`.
- Pause at 115,000,000,000 bytes.
- Never exceed 120,000,000,000 bytes.
- Do not delete caches/artifacts without explicit approval.
- Safe low-impact cleanup previously used: delete only
  `debug/incremental` and `release/incremental`.

## Resume checklist on the other machine

```bash
git clone https://github.com/amanverasia/groky.git
cd groky
git switch main
git pull --ff-only origin main
git rev-parse HEAD   # expected 1581eb3ff6593fe5bf1801d50edc8eafa30d1ca3
```

Then read:

```text
pickup.md
TODO.md
ISSUES.md
docs/superpowers/plans/2026-07-17-portable-arm-no-login-no-phone-home.md
docs/superpowers/plans/2026-07-22-v0.1.1-release-prep.md
docs/releasing.md
```

Start the next focused branch from current green `origin/main`.
