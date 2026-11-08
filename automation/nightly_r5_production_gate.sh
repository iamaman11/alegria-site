#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

bash automation/e2e_supersite_certification.sh
python3 automation/check_e2e_product_verdict.py

# Production acceptance must compare the deployable snapshot with the
# independently served public HTTPS bytes, never just PostgreSQL statuses.
if [[ -z "${ALEGRIA_CERT_PUBLIC_OUTPUT_DIR:-}" ]]; then
  echo "PUBLIC_HTTPS_READBACK: FAILED - ALEGRIA_CERT_PUBLIC_OUTPUT_DIR is required" >&2
  exit 1
fi
python3 automation/check_public_static_output.py "$ALEGRIA_CERT_PUBLIC_OUTPUT_DIR"
python3 automation/check_public_http_readback.py "$ALEGRIA_CERT_PUBLIC_OUTPUT_DIR" \
  --report docs/runs/public_https_readback_evidence.json
