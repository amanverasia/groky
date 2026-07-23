# Current verified issues

Verified against `main` after PR #10 (`ccc8321`). This is the curated Groky
issue register, not a whole-repository audit. Historical findings resolved by
PRs #9 and #10 are recorded below for provenance.

## P1 — Optional tracing layers globally suppress unrelated events

**Status:** Confirmed

**Priority:** P1

`TargetFilterLayer::enabled` in
`crates/codegen/xai-grok-telemetry/src/instrumentation.rs` returns `false` for
nonmatching targets. In a composed `tracing-subscriber` stack that is a global
veto, so enabling sampling or instrumentation can suppress sibling logging
layers.

Acceptance criteria:

- [ ] Target mismatch never returns a global `false` from shared callsite
      interest (`enabled`/`register_callsite`).
- [ ] The optional layer forwards only matching events and span lifecycle data.
- [ ] A composed-subscriber test proves the normal sink receives unrelated
      events while the optional sink receives only its target.
- [ ] Sampling, instrumentation-log, and Chrome modes have focused regressions.
- [ ] One sampling-enabled run proves ordinary diagnostics and
      `sampling.jsonl` both survive.

## P3 hardening — `SamplerConfig` can serialize credentials

**Status:** Confirmed dormant hardening; no active production leak found

**Priority:** P3

`SamplerConfig` derives `Serialize` while containing `api_key` and arbitrary
`extra_headers` values. Its manual `Debug` is redacted, but generic serde
serialization is a latent credential-exfiltration API.

Acceptance criteria:

- [ ] The live credential-bearing `SamplerConfig` is not generically
      serializable, unless an independently verified compatibility requirement
      proves this is necessary.
- [ ] Any needed serialized diagnostics use an explicit safe snapshot that
      omits secret values and cannot reconstruct a live credential-bearing
      config.
- [ ] API-key, authorization-header, and proxy-token canaries never occur in
      Debug or supported serialized diagnostics.
- [ ] Existing structural serde tests are migrated to the deliberate safe
      representation or direct construction.

## Resolved by PR #9 / PR #10

- Upstream auto-updater replacement risk: replaced with a local-only inert
  facade; explicit installer reruns are the update path.
- Shell library test drift: repaired and included in required general CI.
- Missing general pull-request CI: resolved by `general-ci`.
- Pager `grok` branding assertions and user guidance: updated to `groky` while
  preserving compatibility identifiers.
- Workspace formatting failures: resolved.
- Portable release metadata and README: prepared as `0.1.1` by PR #10.

See `TODO.md` for curated provider and security follow-ups. External release,
ARM-hardware, and website gates are documented in `pickup.md` and the release
plans.
