# groky Releases, Rename, And Installer — Design

Date: 2026-07-17. Approved by user (options chosen in brainstorming).

## Goal

Ship versioned prebuilt binaries named `groky` for Linux/macOS (x86_64 +
aarch64) via tag-driven GitHub Releases, installable with a one-line curl
script, with configuration under `~/.groky`.

## Decisions (user-approved)

- Binary and brand: **groky everywhere** — installed command `groky`, config
  under `~/.groky`.
- Platforms: Linux x86_64 + aarch64, macOS x86_64 + aarch64. Windows stays
  best-effort/unsupported.
- Versioning: fork semver starting at **v0.1.0**, tag-driven
  (`git tag v0.1.0` → CI builds and publishes). Upstream's crate numbering is
  abandoned for the shipped version.
- Installer: `install.sh` at the repo root, fetched via
  `https://raw.githubusercontent.com/amanverasia/groky/main/install.sh`;
  groky.dev will later serve/redirect to the same file. Installs to
  `~/.local/bin`, no sudo.
- Rename depth: shallow. Internal crate names (`xai-grok-*`) are unchanged to
  keep upstream merges tractable. `~/.groky` is the new default home with
  `GROKY_HOME` primary, `GROK_HOME` legacy fallback, and automatic one-time
  migration (copy, never delete) from `~/.grok`.

## Components

### 1. Binary rename + version

- `crates/codegen/xai-grok-pager-bin/Cargo.toml`: add `[[bin]] name = "groky"`
  (path to existing main.rs) and set `version = "0.1.0"`. The package name
  stays `xai-grok-pager-bin`.
- Update every reference to the old artifact name that matters for builds and
  tests: PTY harness and test-support binary resolution
  (`CARGO_BIN_EXE_xai-grok-pager` → `CARGO_BIN_EXE_groky`, plus their
  target-dir fallbacks), pty_scenario/scroll_matrix docs, README build
  snippet. `--version` output reports `groky 0.1.0`.

### 2. Config home: `~/.groky` + migration

- `xai-grok-config/src/paths.rs`: resolution order becomes `GROKY_HOME` →
  `GROK_HOME` (legacy) → `~/.groky`.
- One-time migration in the same resolver: when the resolved default
  (`~/.groky`) does not exist and legacy `~/.grok` does, recursively copy
  `~/.grok` → `~/.groky` preserving permissions (auth.json stays 0600),
  then proceed. Never deletes or writes to `~/.grok`. Migration failure falls
  back to creating a fresh `~/.groky` (warn, don't crash).
- The standalone duplicate resolver `xai_fast_worktree::db::resolve_grok_home`
  gets the same precedence + default (without migration logic, or reusing a
  tiny shared helper if trivial).
- Tests cover: precedence order, migration copy (incl. 0600 preservation),
  migration skipped when `~/.groky` exists, env overrides bypass migration.

### 3. Release workflow (`.github/workflows/release.yml`)

- Triggers: push of tag `v*`; `workflow_dispatch` for dry runs (build +
  upload artifacts to the run, skip release creation).
- Matrix (native runners, no cross-compilation):
  - `ubuntu-latest` → `x86_64-unknown-linux-gnu`
  - `ubuntu-24.04-arm` → `aarch64-unknown-linux-gnu`
  - `macos-13` → `x86_64-apple-darwin`
  - `macos-latest` → `aarch64-apple-darwin`
- Each job: `cargo build --release -p xai-grok-pager-bin`, smoke test
  `./groky --version`, package `groky-<version>-<target>.tar.gz` containing
  `groky`, `LICENSE`, `THIRD-PARTY-NOTICES`; emit `<tarball>.sha256`.
- Publish job (tag pushes only): assembles `SHA256SUMS`, creates the GitHub
  Release with generated notes and uploads all artifacts.

### 4. Installer (`install.sh`)

- Usage: `curl -fsSL https://raw.githubusercontent.com/amanverasia/groky/main/install.sh | bash`.
- Behavior: detect OS (Linux/Darwin) + arch (x86_64/aarch64|arm64); resolve
  version from `GROKY_VERSION` (tag, e.g. `v0.1.0`) or latest release via the
  GitHub API; download tarball + `.sha256`; verify checksum (`sha256sum` or
  `shasum -a 256`); extract and install `groky` to `GROKY_INSTALL_DIR`
  (default `~/.local/bin`); `chmod +x`; warn when the install dir is not on
  `PATH`. Idempotent — re-running upgrades in place. No sudo. Fails loudly on
  unsupported platform, missing tools (`curl`, `tar`), checksum mismatch, or
  missing release asset.
- Test seams: `GROKY_DOWNLOAD_BASE` override so CI/local tests can point at a
  local fixture server instead of GitHub; script passes `shellcheck`.

### 5. Docs + first release

- README: installer becomes the primary install path (remove "coming soon"),
  build-from-source snippet updated to the `groky` binary name, roadmap
  checks off releases/installer (website + CI breadth remain).
- Release procedure documented in `docs/releasing.md` (tag, what CI does,
  how to verify).
- Rollout: tag `v0.1.0-rc.1` first; verify all four artifacts and the
  installer end-to-end on Linux; then tag `v0.1.0`.

## Error handling

- Migration: copy failures warn and fall back to fresh `~/.groky`.
- Workflow: any matrix job failure aborts the release; artifacts from dry
  runs never publish.
- Installer: `set -euo pipefail`; every failure path prints an actionable
  message; checksum mismatch deletes the downloaded file.

## Out of scope

- groky.dev hosting (later; the script URL is designed to be fronted by it).
- Windows builds, package managers (brew/apt), self-update, deep crate
  renames, changelog automation.
