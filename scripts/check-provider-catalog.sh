#!/usr/bin/env bash
# Deterministic CI check: regenerate the catalog snapshot from the COMMITTED
# raw models.dev document and verify the committed snapshot matches byte-for-
# byte. No network access; never flakes on upstream models.dev publishes.
#
# To check freshness against live models.dev instead (advisory), run:
#   scripts/update-provider-catalog.sh --check
set -euo pipefail
cd "$(dirname "$0")/.."

cargo run -p xai-grok-catalog --bin generate_catalog -- \
  --input crates/codegen/xai-grok-catalog/data/models-dev-raw.json \
  --output crates/codegen/xai-grok-catalog/data/models-dev.json \
  --check
