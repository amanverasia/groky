#!/usr/bin/env bash
set -euo pipefail
cargo run -p xai-grok-catalog --features generator --bin generate_catalog -- \
  --fetch https://models.dev/api.json \
  --output crates/codegen/xai-grok-catalog/data/models-dev.json "$@"
