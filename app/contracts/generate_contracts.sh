#!/usr/bin/env bash
set -e

# ==============================================================================
# SEO Engine V2: Contract Generation Script
# ==============================================================================
# 1. Generates Python protobuf classes
# 2. Compiles FlatBuffers schemas

echo "Starting contract generation..."

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "$DIR"

# Directories
PROTO_DIR="./proto"
FBS_DIR="./fbs"
OUT_PYTHON="./generated/python"
OUT_RUST="./generated/rust"

mkdir -p "$OUT_PYTHON" "$OUT_RUST"

# 1. Generate Python Protobuf
# Requires grpcio-tools: pip install grpcio-tools
echo "Generating Python Protobuf..."
python3 -m grpc_tools.protoc -I"$PROTO_DIR" \
    --python_out="$OUT_PYTHON" \
    --pyi_out="$OUT_PYTHON" \
    "$PROTO_DIR"/*.proto

# Note: Rust Protobuf generation is handled by `prost-build` inside `build.rs` 
# of the app/rust/crates/contracts crate.
# This script focuses on Python and general FlatBuffers.

# 2. Compile FlatBuffers (if flatc is installed)
if command -v flatc &> /dev/null; then
    echo "Generating FlatBuffers for Python..."
    flatc --python -o "$OUT_PYTHON" "$FBS_DIR"/*.fbs
    
    echo "Generating FlatBuffers for Rust..."
    flatc --rust -o "$OUT_RUST" "$FBS_DIR"/*.fbs
else
    echo "WARNING: 'flatc' not found in PATH. Skipping FlatBuffers compilation."
    echo "Please install flatbuffers compiler to generate FBS schemas."
fi

echo "Contract generation complete."
