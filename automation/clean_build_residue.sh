#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_ROOT="${CLEAN_BUILD_TARGET_ROOT:-$ROOT_DIR/app/rust/target}"

TRANSIENT_REPORTS=(
  "/tmp/truth_certification_local_ci_current.json"
  "/tmp/whole_page_semantic_current.json"
  "/tmp/clean_acceptance_bundle_current.json"
)

if [[ -d "$TARGET_ROOT" ]]; then
  rm -rf "$TARGET_ROOT"
  echo "removed build target root: $TARGET_ROOT"
else
  echo "build target root already clean: $TARGET_ROOT"
fi

for report_path in "${TRANSIENT_REPORTS[@]}"; do
  rm -f "$report_path"
done

echo "removed transient reports: ${TRANSIENT_REPORTS[*]}"
