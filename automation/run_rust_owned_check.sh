#!/usr/bin/env bash
set -euo pipefail

# Only these six pre-existing Rust-backed checks may be delegated to their
# explicitly required CI owners. Never accept an arbitrary command or job.
case "$*" in
  "python3 automation/smoke_crawler_robots_policy.py"|\
  "bash automation/contract_verify.sh"|\
  "python3 automation/smoke_run_report_completeness.py") source_job="verify"; target_owner="rust-contract-smokes" ;;
  "python3 automation/smoke_rebuild_dependency_graph.py"|\
  "python3 automation/smoke_rebuild_queue_execution_path.py"|\
  "python3 automation/smoke_extraction_coverage.py") source_job="product-truth"; target_owner="architecture-and-rust" ;;
  *) echo "Unsupported Rust-backed CI check: $*" >&2; exit 2 ;;
esac

case "${CI_RUST_CHECK_OWNER:-verify}" in
  verify)
    exec "$@"
    ;;
  architecture-and-rust|rust-contract-smokes)
    # Local 'ci_verify.sh all' still runs every test. The only exception is
    # the exact GitHub source job, whose gate requires the selected owner PASS.
    if [[ "${CI_RUST_CHECK_OWNER:-}" != "$target_owner" ||
          "${GITHUB_ACTIONS:-}" != "true" || "${GITHUB_JOB:-}" != "$source_job" ||
          ! "${CI_SOURCE_SHA:-}" =~ ^[0-9a-f]{40}$ ]]; then
      echo "Rust check delegation requires exact-source GitHub $source_job" >&2
      exit 2
    fi
    echo "CI_RUST_CHECK_DELEGATED owner=$target_owner check=$2 source_sha=$CI_SOURCE_SHA"
    ;;
  *)
    echo "Unknown Rust check owner: $CI_RUST_CHECK_OWNER" >&2
    exit 2
    ;;
esac
