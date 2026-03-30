#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

echo "[DEPRECATED] python_bridge crate removed from runtime architecture."
echo "Building Rust primitives gRPC service instead (single source of truth for hash/id/url_norm)."

cargo build -p primitives_svc --release
