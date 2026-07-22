# Portable ARM Builds, No Login, and No Phone-Home

## Summary

groky v0.1.0 crashes with SIGILL on common aarch64 hardware, blocks first-time users behind a grok.com login screen, and ships upstream xAI's auto-updater, which checks `x.ai` at startup and can silently replace groky with the upstream `grok` binary. This design fixes all three for v0.1.1:

1. Build aarch64 Linux release binaries for `target-cpu=generic` so they run on any ARMv8 machine.
2. Neuter `xai-grok-update` into a local-only facade (the same pattern used for `xai-grok-telemetry`), eliminating all update-related network traffic.
3. Remove the interactive grok.com login from the advertised auth methods so a credential-less first run lands directly in the app.

This revises a decision in the 2026-07-16 telemetry design, which kept update checks as "product features rather than telemetry." Field experience shows the updater phones xAI infrastructure by default and would replace groky with upstream grok once xAI's channel pointer advances past the lockstepped version (`0.2.101`). That is incompatible with groky's no-phone-home policy.

## Goals

- Release binaries run on all ARMv8-A aarch64 Linux hosts (e.g., Oracle Ampere A1 / Neoverse N1), not just ARMv9 machines.
- A release-build groky makes zero network requests unless the user configures a model provider; traffic to a user-configured provider endpoint is the product's function and is unaffected.
- First run with no credentials shows no login screen and no quit-or-login gate; the user lands in the app with a passive hint about configuring credentials.
- Users with existing credentials (`XAI_API_KEY`, cached token, BYOK provider keys) see no behavior change.
- Guard rails in CI prevent the upstream sync from silently reintroducing the `target-cpu` flag or updater networking.

## Non-Goals

- Re-pointing the updater at groky's own GitHub releases. Users update via `install.sh`; the README documents this.
- Removing the OAuth/login code from the tree. It becomes unreachable from advertised auth methods; deleting it would inflate the upstream-sync diff for no behavioral gain.
- Disabling the `/v1/settings` prefetch for authenticated users. It only contacts the endpoint the user configured.
- macOS release builds (still disabled pending runner diagnosis; see README Roadmap).

## Design

### 1. Portable aarch64 builds

Root cause of the SIGILL: `.cargo/config.toml` sets `target-cpu=neoverse-v2` (ARMv9-A) for `aarch64-unknown-linux-gnu`, inherited from upstream's server-tuned build. GitHub's `ubuntu-24.04-arm` runners (Cobalt 100, Neoverse N2, ARMv9) execute the resulting instructions, so the release smoke test passes, but ARMv8 hosts such as Ampere A1 (Neoverse N1) trap immediately.

Changes:

- `.cargo/config.toml` `[target.aarch64-unknown-linux-gnu]`: replace `target-cpu=neoverse-v2` with `target-cpu=generic`, matching the existing `aarch64-unknown-linux-musl` section.
- `release.yml`: add a guard step that fails the build if `.cargo/config.toml` contains a `target-cpu` value other than `generic` for any `*-unknown-linux-*` target. This protects against upstream-sync regressions.
- Bump `xai-grok-pager-bin` to 0.1.1 before tagging `v0.1.1` (the smoke test enforces tag/binary agreement).

Verification is empirical: after tagging, install on the affected Ampere A1 host and confirm `groky --version` runs. Optionally pre-verify with `qemu-aarch64 -cpu neoverse-n1` where available.

### 2. Updater becomes a local-only facade

`xai-grok-update` is a direct dependency of `xai-grok-pager-bin` and, in release builds, checks `https://x.ai/cli` (fallback: xAI's GCS bucket) in the background on every start, writes `cli.auto_update = true` into the user's config on first run, and can download and symlink-swap the upstream `grok` binary into `~/.groky/bin`.

Following the `xai-grok-telemetry` precedent ("local-only compatibility facade"), the crate keeps its public API so call sites in `main.rs` compile unchanged, but:

- Version fetching performs no I/O and reports the current version as latest (no update available). The `x.ai`, GCS, npm, and `xai-org-shared/grok-build` endpoints are removed from the compiled code.
- No downloads, no writes to `~/.groky/downloads`, no symlink management, no config mutation (`cli.auto_update` is never written).
- `enforce_minimum_version_or_exit` retains its existing no-config fast path and never reaches the network.
- Update-notification UI paths receive "no update available" and therefore render nothing.

`bin/check-no-network-telemetry.sh` changes from protecting the updater to policing it:

- Remove the assertions that `xai-grok-update` retains networking.
- Add checks that forbid the upstream update endpoints (`x.ai/cli`, `storage.googleapis.com/grok-build-public-artifacts`, `@xai-official/grok`, `xai-org-shared/grok-build`) from appearing in the updater's non-test source.
- The script continues to assert the crate itself remains present, preserving the compatibility-facade approach across upstream syncs.

### 3. Credential-less first run lands in the app

Today, `build_unpinned` in `xai-grok-shell/src/agent/auth_method.rs` always appends an interactive grok.com/OIDC auth method; the pager classifies it as `needs_login` and renders a blocking "Login with grok.com / Quit" screen with no third option.

Changes:

- `auth_method.rs`: stop appending the interactive login method. Advertised methods become exactly: `XAI_API_KEY` env, cached token (when present), and BYOK provider keys (when present).
- With no advertised method requiring interactive login, `needs_login` in `xai-grok-pager/src/acp/mod.rs` is false, so the blocking welcome gate never activates. The welcome view shows a passive hint instead: set `XAI_API_KEY` or configure a provider via `/providers`.
- The OAuth flow implementation stays in the tree but is unreachable from advertised methods.
- Existing test `shell_built_auth_methods_for_byok_user_skip_login_screen` must keep passing; users with any existing credentials see no change.

## Testing

Test-driven throughout; new failing tests precede each change:

- Updater facade: version check performs no network I/O (no reqwest/hyper types in the code path; assert via unit tests that the result is "up to date" and that no config mutation occurs).
- Auth methods: with empty credentials, the advertised method list contains no interactive-login entry and `needs_login` is false.
- Welcome view: credential-less startup reaches the normal app view with the hint rendered, not the login menu.
- Guard script: runs clean after the flip; a fixture-level check confirms it fails when an upstream endpoint string is reintroduced into the updater.
- CI: existing workspace tests, `bin/check-no-network-telemetry.sh`, and the release smoke test all pass; the new `target-cpu` guard step passes.

## Release

Ship as v0.1.1: bump version, tag, verify release assets and checksums, then confirm on the Ampere A1 host that the binary runs and that a credential-less first start makes no network requests (verifiable with `strace -f -e trace=network` or by watching for connections).
