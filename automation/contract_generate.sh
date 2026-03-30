#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

python3 automation/gen_proto_contracts.py --out automation/PROTO_CONTRACTS.md
echo "OK: contract generation completed"
