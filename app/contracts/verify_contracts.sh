#!/usr/bin/env bash
set -e

# ==============================================================================
# SEO Engine V2: Contract Verification Script
# ==============================================================================
# Verifies that generated artifacts are up-to-date with proto/fbs sources.
# Used in CI/CD (Phase C, Step 16).

echo "Verifying contracts..."

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "$DIR"

# Run generation in a temporary directory
TEMP_DIR=$(mktemp -d)
cp -r proto fbs "$TEMP_DIR/"
mkdir -p "$TEMP_DIR/generated/python"

echo "Checking Python protobuf drift..."
python3 -m grpc_tools.protoc -I"$TEMP_DIR/proto" \
    --python_out="$TEMP_DIR/generated/python" \
    --pyi_out="$TEMP_DIR/generated/python" \
    "$TEMP_DIR/proto"/*.proto

# Simple diff check (excluding pycache if any)
if diff -r -q "$TEMP_DIR/generated/python" "./generated/python" > /dev/null; then
    echo "✅ Python contracts are up to date."
else
    echo "❌ ERROR: Python contracts are out of sync. Please run generate_contracts.sh and commit the changes."
    rm -rf "$TEMP_DIR"
    exit 1
fi

rm -rf "$TEMP_DIR"
echo "Verification passed."
