#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

python3 automation/gen_proto_contracts.py --out automation/PROTO_CONTRACTS.md --check

# Also validate proto compilation path by checking contracts + infrastructure crates.
(cd app/rust && cargo check -q -p contracts -p infrastructure)

echo "OK: contract verification passed"
