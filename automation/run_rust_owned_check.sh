#!/usr/bin/env bash
set -euo pipefail

# Only these six established Rust-backed deep checks may execute here.
# The fast PR jobs do not claim that a Python source check ran them.
# Never delegate a check to another job without executing its real command.
case "$*" in
  "python3 automation/smoke_crawler_robots_policy.py"|\
  "bash automation/contract_verify.sh"|\
  "python3 automation/smoke_run_report_completeness.py"|\
  "python3 automation/smoke_rebuild_dependency_graph.py"|\
  "python3 automation/smoke_rebuild_queue_execution_path.py"|\
  "python3 automation/smoke_extraction_coverage.py") ;;
  *) echo "Unsupported Rust-backed CI check: $*" >&2; exit 2 ;;
esac

if [[ "${CI_RUST_CHECK_OWNER:-verify}" != "verify" ]]; then
  echo "Rust-backed checks cannot be delegated to another CI job" >&2
  exit 2
fi
exec "$@"
