#!/usr/bin/env bash
# Pin the two native build tools without a full apt index refresh on each runner.
# The release hashes are for the exact Linux x86_64 assets, not their tags.
set -euo pipefail

if [[ -z "${RUNNER_TEMP:-}" || -z "${GITHUB_PATH:-}" || -z "${GITHUB_ENV:-}" ]]; then
  echo 'CI native tools require the GitHub Actions environment' >&2
  exit 1
fi

tool_dir="$RUNNER_TEMP/alegria-ci-native-tools"
mkdir -p "$tool_dir"

fetch_verified() {
  local url="$1" sha256="$2" archive="$3"
  curl --fail --location --silent --show-error --retry 3 --max-time 90 \
    --output "$archive" "$url"
  printf '%s  %s\n' "$sha256" "$archive" | sha256sum --check --status
}

for tool in "$@"; do
  case "$tool" in
    protoc)
      archive="$tool_dir/protoc.zip"
      fetch_verified \
        'https://github.com/protocolbuffers/protobuf/releases/download/v21.12/protoc-21.12-linux-x86_64.zip' \
        '3a4c1e5f2516c639d3079b1586e703fc7bcfa2136d58bda24d1d54f949c315e8' \
        "$archive"
      mkdir -p "$tool_dir/protobuf"
      unzip -q "$archive" 'bin/protoc' 'include/*' -d "$tool_dir/protobuf"
      chmod 755 "$tool_dir/protobuf/bin/protoc"
      printf '%s\n' "$tool_dir/protobuf/bin" >> "$GITHUB_PATH"
      printf 'PROTOC_INCLUDE=%s\n' "$tool_dir/protobuf/include" >> "$GITHUB_ENV"
      "$tool_dir/protobuf/bin/protoc" --version
      ;;
    ripgrep)
      archive="$tool_dir/ripgrep.tar.gz"
      fetch_verified \
        'https://github.com/BurntSushi/ripgrep/releases/download/16.0.0/ripgrep-16.0.0-x86_64-unknown-linux-musl.tar.gz' \
        '54268ab4643c00a290a29b0086066d64b3f56236dd805ec64629a86f9bd29fd0' \
        "$archive"
      mkdir -p "$tool_dir/ripgrep"
      tar -xzf "$archive" -C "$tool_dir/ripgrep" --strip-components=1 \
        'ripgrep-16.0.0-x86_64-unknown-linux-musl/rg'
      printf '%s\n' "$tool_dir/ripgrep" >> "$GITHUB_PATH"
      "$tool_dir/ripgrep/rg" --version
      ;;
    *)
      echo "Unsupported CI native tool: $tool" >&2
      exit 1
      ;;
  esac
done
