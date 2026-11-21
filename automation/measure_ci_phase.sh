#!/usr/bin/env bash
# Preserve the phase's actual exit status while publishing machine-readable
# elapsed time. This is observation only: it never exempts a failed gate.
set -euo pipefail

if [[ $# -lt 2 || ! "$1" =~ ^(contracts|product|integration|truth_worker_build|truth_outbox_build|truth_transport|truth_suite|semantic_fixture)$ ]]; then
  echo 'usage: measure_ci_phase.sh <known phase> <command...>' >&2
  exit 2
fi
phase="$1"
shift
report="${CI_TIMING_REPORT_PATH:-}"
if [[ -z "$report" && "${GITHUB_ACTIONS:-}" == "true" ]]; then
  report='automation/reports/ci_phase_timings.jsonl'
fi
source_sha="${CI_SOURCE_SHA:-${GITHUB_SHA:-unknown}}"
# A runner may synchronize its wall clock backwards while a phase is running.
# Measure elapsed time from a monotonic clock, not GNU date/EPOCHREALTIME.
monotonic_ms() {
  python3 -c 'import time; print(time.monotonic_ns() // 1_000_000)'
}
start_ms="$(monotonic_ms)"

if "$@"; then
  result=0
else
  result=$?
fi

end_ms="$(monotonic_ms)"
elapsed_ms="$((end_ms - start_ms))"
if [[ -n "$report" ]]; then
  mkdir -p "$(dirname "$report")"
  printf '{"phase":"%s","source_sha":"%s","elapsed_ms":%s,"exit_code":%s}\n' \
    "$phase" "$source_sha" "$elapsed_ms" "$result" >> "$report"
fi
echo "CI_PHASE_TIME phase=$phase elapsed_ms=$elapsed_ms exit_code=$result source_sha=$source_sha"
exit "$result"
