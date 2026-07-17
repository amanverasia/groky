#!/usr/bin/env bash
# Refresh the committed models.dev raw document and regenerate the normalized
# catalog snapshot from it. Run this when you want to pick up new upstream
# providers/models; commit both files together.
set -euo pipefail
cd "$(dirname "$0")/.."

RAW=crates/codegen/xai-grok-catalog/data/models-dev-raw.json
OUT=crates/codegen/xai-grok-catalog/data/models-dev.json

curl -fsSL https://models.dev/api.json -o "$RAW"
cargo run -p xai-grok-catalog --features generator --bin generate_catalog -- \
  --input "$RAW" \
  --output "$OUT" "$@"
