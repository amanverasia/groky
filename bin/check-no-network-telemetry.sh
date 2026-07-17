#!/usr/bin/env bash
# Guard: no network-telemetry code, config, or dependencies may return.
# Update/announcement networking is intentionally retained (asserted below).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Forbidden source symbols. Two files are excluded precisely because they are
# negative-assertion guard tests that must name the forbidden symbols:
# - xai-file-utils/tests/no_trace_upload_api.rs asserts upload_session_metadata
#   and spawn_trace_upload do NOT appear in product sources.
# - xai-tool-protocol/src/methods.rs has a test asserting the *.donate wire
#   methods are NOT part of the protocol.
forbidden_source='api\.mixpanel\.com|GROK_EXTERNAL_OTEL|GROK_INTERNAL_OTLP|GROK_WORKSPACE_OTLP_ENDPOINT|SENTRY_DSN|traces\.donate|logs\.donate|metrics\.donate|spawn_trace_upload|upload_session_metadata|OTEL_EXPORTER_OTLP'
if rg -n --glob '*.rs' --glob '*.toml' \
    --glob '!crates/codegen/xai-file-utils/tests/no_trace_upload_api.rs' \
    --glob '!crates/common/xai-tool-protocol/src/methods.rs' \
    "$forbidden_source" crates prod Cargo.toml; then
  echo "network telemetry source survived" >&2; exit 1
fi

forbidden_manifest='^\s*(sentry|xai-mixpanel|opentelemetry(-otlp|-http|-proto|_sdk)?|tracing-opentelemetry|fastrace-(opentelemetry|reqwest|tonic))\s*='
if rg -n --glob 'Cargo.toml' "$forbidden_manifest" Cargo.toml crates prod; then
  echo "network telemetry manifest entry survived" >&2; exit 1
fi

if rg -n '^name = "(sentry|opentelemetry|opentelemetry-otlp|opentelemetry-proto|opentelemetry-http|opentelemetry_sdk|tracing-opentelemetry|fastrace-opentelemetry|xai-mixpanel)"' Cargo.lock; then
  echo "network telemetry dependency survived in lockfile" >&2; exit 1
fi

# Product networking intentionally retained.
rg -q 'xai-grok-update' Cargo.toml
rg -q 'xai-grok-announcements' Cargo.toml
echo "no network telemetry found"
