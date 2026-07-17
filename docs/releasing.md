# Releasing groky

Releases are tag-driven. Pushing a tag matching `v*` runs
[`.github/workflows/release.yml`](../.github/workflows/release.yml), which
builds `groky` natively on four runners, packages tarballs with checksums,
and publishes a GitHub Release.

## Targets

| Runner | Target triple |
|---|---|
| `ubuntu-latest` | `x86_64-unknown-linux-gnu` |
| `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` |

macOS targets (`x86_64-apple-darwin` on `macos-13`, `aarch64-apple-darwin` on
`macos-latest`) are disabled in the workflow for now — the runners were
excessively slow/hung on the first release attempt. Re-enable the commented
matrix entries and the Darwin branch in `install.sh` once diagnosed.

Artifacts: `groky-<version>-<target>.tar.gz` (contains `groky`, `LICENSE`,
`THIRD-PARTY-NOTICES`), a `.sha256` per tarball, and a combined `SHA256SUMS`.

## Cutting a release

1. **Bump the version** in `crates/codegen/xai-grok-pager-bin/Cargo.toml` AND
   `crates/codegen/xai-grok-pager/Cargo.toml` (the `--version` string is
   compiled from the pager lib's package version). The workflow's smoke test
   fails the build if the tag's base version does not match what the binary
   reports. Commit and merge to `main`.
2. **Dry run (optional):** trigger the `release` workflow manually
   (`workflow_dispatch`) — it builds all four artifacts and uploads them to
   the run without publishing anything.
3. **Release candidate (recommended for significant releases):**
   `git tag v0.1.0-rc.1 && git push origin v0.1.0-rc.1`. Pre-release tags
   pass the smoke test as long as the base version matches (a `0.1.0` binary
   is accepted for `v0.1.0-rc.1`). Verify the artifacts and the installer:
   `GROKY_VERSION=v0.1.0-rc.1 bash install.sh`.
4. **Final:** `git tag v0.1.0 && git push origin v0.1.0`.

The GitHub Release is created with auto-generated notes; edit them afterwards
if needed.

## Installer

[`install.sh`](../install.sh) at the repo root downloads the right tarball
for the host OS/arch from the latest (or `GROKY_VERSION`-pinned) release,
verifies its sha256, and installs to `~/.local/bin` (`GROKY_INSTALL_DIR` to
override). Test it locally against a fake release layout with
[`scripts/test-install.sh`](../scripts/test-install.sh); lint with
`shellcheck install.sh`.

When groky.dev is live it should serve or redirect
`https://groky.dev/install.sh` to the raw `main` copy of this script.

## Version string notes

- `groky --version` reports the pager package version plus the build commit,
  e.g. `groky 0.1.0 (44524d3) [alpha]`.
- The internal `xai-grok-version` crate (used for API client-version headers
  and update-channel logic) deliberately stays on upstream's numbering and is
  not part of the release version.
