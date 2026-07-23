# Current Groky handoff

> **Baseline:** `main` after PR #10 (`ccc8321`)
>
> **Release metadata:** pager and pager-bin are `0.1.1`
>
> **Release status:** prepared but not tagged or published

## Completed

- PR #9 merged portable generic-aarch64 builds, credential-less first run,
  local-only updater behavior, canonical `groky` / `~/.groky` branding,
  checksummed installers, and required general CI.
- PR #10 merged the v0.1.1 README and release-facing version preparation.
- Required PR and merged-main `general-ci` jobs (`validate`, `test`, and
  `installer`) passed.
- Local release build reported `groky 0.1.1`; updater tests and installer E2E
  passed during release preparation.

## Active engineering backlog

The canonical current lists are `ISSUES.md` and `TODO.md`.

Immediate focused branches, in order:

1. isolate optional tracing target filters so sibling logs are not suppressed;
2. make live `SamplerConfig` non-serializable and add a safe diagnostic view;
3. finish generic dynamic-provider configuration, composition, reload, cache
   origin scoping, and Janus footer behavior;
4. remove unused plaintext provider `session_keys`;
5. selectively adapt reviewed upstream security improvements (web-fetch SSRF,
   plugin Git operands, LSP trust, permission/sensitive-file/endpoint and
   secure-file hardening).

Do not merge `upstream/main` wholesale; preserve Groky's no-phone-home,
no-forced-login, ARM portability, and provider credential-isolation policies.

## External gates still pending

- Manual scratch-home, credential-less startup under syscall-level network
  tracing; confirm normal welcome/passive credentials hint and no unexpected
  outbound connection.
- Explicit authorization to create and push `v0.1.1`, followed by release
  workflow and artifact/checksum verification.
- Real ARMv8/Ampere A1 installation and no-SIGILL smoke.
- groky.dev hosting/DNS deployment.
- macOS release artifacts and promoted Windows support.

## Resource policy

Use one Cargo command at a time with two jobs and low priority. The shared
Cargo target is `/home/amanverasia/.cache/groky-cargo-target`: pause at 115 GB,
never exceed 120 GB, and do not delete caches/artifacts without explicit
approval.
