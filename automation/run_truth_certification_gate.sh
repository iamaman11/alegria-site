#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_PATH="${TRUTH_CERT_REPORT_PATH:-/tmp/truth_certification_local_ci_current.json}"
LOG_PATH="$(mktemp /tmp/truth-cert-gate.XXXXXX.log)"
TARGET_ROOT="${TRUTH_CERT_TARGET_ROOT:-$ROOT_DIR/app/rust/target}"
NESTED_TARGET_ROOT="${TRUTH_CERT_NESTED_TARGET_ROOT:-$TARGET_ROOT/integration-harness-nested}"
SUITE_TARGET_DIR="${TRUTH_CERT_SUITE_TARGET_DIR:-$TARGET_ROOT/integration-harness-suite}"
# Reuse compiled dependencies between the real Worker binaries, without
# sharing their target with the integration harness running those binaries.
# Runtime processes, binary identities and the disposable DB remain separate.
WORKERS_TARGET_DIR="$NESTED_TARGET_ROOT/workers"
TEMPORAL_BIN="$WORKERS_TARGET_DIR/debug/temporal_worker"
OUTBOX_BIN="$WORKERS_TARGET_DIR/debug/outbox_worker"

if [[ "$(realpath -m "$WORKERS_TARGET_DIR")" == "$(realpath -m "$SUITE_TARGET_DIR")" ]]; then
  echo "Truth Worker and integration harness Cargo target directories must differ" >&2
  exit 2
fi

cd "$ROOT_DIR"

rm -f "$REPORT_PATH"

bash automation/measure_ci_phase.sh truth_worker_build cargo build --manifest-path app/rust/Cargo.toml \
  --target-dir "$WORKERS_TARGET_DIR" \
  -p temporal_worker \
  --bin temporal_worker

bash automation/measure_ci_phase.sh truth_outbox_build cargo build --manifest-path app/rust/Cargo.toml \
  --target-dir "$WORKERS_TARGET_DIR" \
  -p outbox_worker \
  --bin outbox_worker

for worker_bin in "$TEMPORAL_BIN" "$OUTBOX_BIN"; do
  if [[ ! -x "$worker_bin" ]]; then
    echo "Missing or non-executable Truth Worker binary: $worker_bin" >&2
    exit 1
  fi
done
echo "CI_TRUTH_WORKER_BINARIES temporal=$TEMPORAL_BIN outbox=$OUTBOX_BIN"

# Prove both Qdrant transports on a real container before running the
# multi-fixture truth certification. A gRPC-only harness cannot validate the
# HTTP REST endpoint used by projection_materialize_adapter.
bash automation/measure_ci_phase.sh truth_transport cargo test --manifest-path app/rust/Cargo.toml \
  --target-dir "$SUITE_TARGET_DIR" \
  -p integration_harness \
  --features e2e \
  --lib \
  qdrant_rest_outbox_upsert_is_visible_to_grpc_retrieval \
  -- --nocapture

run_suite() {
  rm -f "$LOG_PATH"
  INTEGRATION_TEMPORAL_WORKER_BIN="$TEMPORAL_BIN" \
  INTEGRATION_OUTBOX_WORKER_BIN="$OUTBOX_BIN" \
  TRUTH_CERT_REPORT_PATH="$REPORT_PATH" \
  bash automation/measure_ci_phase.sh truth_suite cargo test --manifest-path app/rust/Cargo.toml \
    --target-dir "$SUITE_TARGET_DIR" \
    -p integration_harness \
    --features e2e \
    --lib \
    truth_certification_suite_matches_fixture_expectations \
    -- --nocapture 2>&1 | tee "$LOG_PATH"
}

is_transient_infra_failure() {
  grep -Eq \
    "probe (neo4j|qdrant|temporal) harness failed|Connection reset by peer|bind text stub listener failed" \
    "$LOG_PATH"
}

if ! run_suite; then
  if is_transient_infra_failure; then
    echo "truth certification gate hit transient infra bootstrap failure; retrying once" >&2
    sleep 3
    run_suite
  else
    exit 1
  fi
fi

python3 automation/check_truth_certification_regression.py "$REPORT_PATH"
