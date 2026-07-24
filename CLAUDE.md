# Groky contributor instructions

## Project invariants

- Groky is a privacy/local-first fork. Do not reintroduce networked telemetry,
  automatic updater downloads, upstream binary replacement, or a forced default
  browser-login gate.
- User-visible command/config naming is `groky`, `GROKY_*`, and `~/.groky`.
  Preserve legacy identifiers only where compatibility, protocol, or migration
  code explicitly requires them (`GROK_HOME`, selected `GROK_*`, `grok.com`,
  `xai-grok-*` crate names).
- The updater is an inert local-only compatibility facade. Updates are performed
  explicitly by re-running the checksummed installer.
- Linux aarch64 release builds must stay on a generic ARMv8-compatible CPU
  baseline; do not add host-specific `target-cpu` flags.
- Provider/model snapshots and caches must remain secret-free. Provider
  credentials resolve at request time and must never fall back to unrelated xAI
  session credentials or endpoints.
- Dynamic-provider endpoint changes must not reuse discovered models from a
  different canonical base URL. Endpoint overrides remain same-origin,
  path-only, query-free, and fragment-free.

## Build and verification discipline

- Prefer targeted crate commands; full-workspace builds are expensive.
- Run local Cargo commands serially with:

  ```bash
  CARGO_BUILD_JOBS=2 \
  CARGO_TARGET_DIR=/home/amanverasia/.cache/groky-cargo-target \
  nice -n 10 cargo ...
  ```

  On another machine, use an equivalent local target path.
- Measure build storage before and after expensive work. On the original
  development machine, pause at 115 GB and never exceed 120 GB. Do not delete
  caches or artifacts without approval. The least disruptive cleanup is only
  Cargo's `debug/incremental` and `release/incremental` directories.
- Run relevant focused tests first, then formatting, diff whitespace, affected
  crate checks/Clippy, and `bin/check-no-network-telemetry.sh`.
- Every substantive change goes through a focused branch and PR. Merge only
  after required CI is green. Do not merge `upstream/main` wholesale; adapt
  reviewed upstream fixes in isolated PRs while preserving the invariants above.

## Project records

- `TODO.md` is the curated current backlog.
- `ISSUES.md` contains currently verified issues, not every imported source TODO.
- `pickup.md` is the current cross-machine/session handoff.
- Historical implementation plans under `docs/superpowers/plans/` are evidence;
  do not treat every old unchecked checkbox as current work without reconciling
  it against merged PRs and tests.
- Release procedure and external gates are documented in `docs/releasing.md` and
  the active release plans. Never claim a release, hardware check, or website
  deployment complete until it has actually occurred.
