#!/usr/bin/env python3
"""Read-only, exact-run PR CI critical-path evidence at the existing green gate.

This is diagnostic evidence, not a release gate or a substitute for the
three independently verified event-to-green measurements required by #94.
All required real tests have already completed before this script runs.
"""

from __future__ import annotations

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import urllib.error
import urllib.request


# Every row is an existing source-attested runner and its actual predecessors.
# No invented build/test durations and no change to the workflow's `needs`.
E2E_PATHS = {
    "manual-temporal": (
        "rust-temporal-legacy-e2e (temporal_manual)",
        ("rust-e2e-temporal-harness", "rust-e2e-legacy-binaries"),
    ),
    "native-qdrant": (
        "rust-qdrant-temporal-e2e (temporal_qdrant)",
        (
            "rust-e2e-qdrant-temporal-harness",
            "rust-e2e-qdrant-effects-binaries",
            "rust-e2e-v3-binaries",
        ),
    ),
    "outbox": (
        "rust-outbox-e2e (outbox)",
        ("rust-e2e-outbox-harness", "rust-e2e-legacy-binaries"),
    ),
}


def timestamp(value: str | None) -> datetime | None:
    if not value:
        return None
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("GitHub job timestamps must contain an explicit timezone")
    return parsed.astimezone(timezone.utc)


def seconds(start: datetime, end: datetime) -> float:
    return round((end - start).total_seconds(), 3)


def analyze(run: dict, jobs: list[dict], observed_at: datetime) -> dict:
    """Explain observed elapsed time without treating skips as passing tests."""
    if run.get("event") != "pull_request":
        raise ValueError("PR-only report received a non-PR workflow")
    created = timestamp(run.get("created_at"))
    if created is None or observed_at < created:
        raise ValueError("missing or future GitHub workflow creation timestamp")
    by_name: dict[str, dict] = {}
    completed: list[dict] = []
    for job in jobs:
        name = job.get("name")
        if not isinstance(name, str) or name in by_name:
            raise ValueError("missing or duplicate GitHub job display name")
        by_name[name] = job
        started, finished = timestamp(job.get("started_at")), timestamp(job.get("completed_at"))
        if job.get("conclusion") != "success" or not started or not finished:
            continue
        if not created <= started <= finished <= observed_at:
            raise ValueError(f"inconsistent server timestamps for {name}")
        if name != "CI":
            completed.append({
                "name": name,
                "job_seconds": seconds(started, finished),
                "event_to_finish_seconds": seconds(created, finished),
                "finished_at": finished,
            })

    completed.sort(key=lambda row: row["finished_at"], reverse=True)
    paths = []
    for label, (runner_name, upstream_names) in E2E_PATHS.items():
        runner = by_name.get(runner_name)
        if not runner or runner.get("conclusion") != "success":
            continue
        runner_started = timestamp(runner.get("started_at"))
        runner_done = timestamp(runner.get("completed_at"))
        upstream = []
        for name in upstream_names:
            candidate = by_name.get(name)
            finished = timestamp(candidate.get("completed_at")) if candidate else None
            if not candidate or candidate.get("conclusion") != "success" or not finished:
                raise ValueError(f"{runner_name} succeeded without successful prerequisite {name}")
            upstream.append((name, finished))
        if not runner_started or not runner_done:
            raise ValueError(f"successful {runner_name} is missing timestamps")
        slowest_dep_name, last_dep_finish = max(upstream, key=lambda item: item[1])
        if runner_started < last_dep_finish:
            raise ValueError(f"{runner_name} started before its source-bound prerequisite")
        paths.append({
            "route": label,
            "last_compiler": slowest_dep_name,
            "compiler_ready_seconds": seconds(created, last_dep_finish),
            "runner_gap_seconds": seconds(last_dep_finish, runner_started),
            "e2e_job_seconds": seconds(runner_started, runner_done),
            "event_to_e2e_finish_seconds": seconds(created, runner_done),
        })
    return {
        "run_id": run.get("id"),
        "head_sha": run.get("head_sha"),
        "observed_event_to_current_step_seconds": seconds(created, observed_at),
        "finishers": completed[:6],
        "measured_paths": paths,
        "status": "PRE_GATE_COMPLETION_OBSERVATION_ONLY",
        "event_to_green_180_confirmed": False,
    }


def to_summary(report: dict) -> str:
    rows = [
        "### #94: exact PR event-to-gate critical-path observation",
        "",
        f"Source: PR workflow `{report['run_id']}`, GitHub SHA `{report['head_sha']}`.",
        f"Elapsed at diagnostic step: **{report['observed_event_to_current_step_seconds']:.1f}s**. "
        "The final gate end is measured independently by GitHub Actions; "
        "this observation alone is **not** an accepted ≤180s green result.",
        "",
        "| Real E2E path | Last required compiler | Ready since event | Runner gap | Real E2E job | End since event |",
        "| --- | --- | ---: | ---: | ---: | ---: |",
    ]
    for path in report["measured_paths"]:
        rows.append(
            f"| {path['route']} | `{path['last_compiler']}` | "
            f"{path['compiler_ready_seconds']:.1f}s | {path['runner_gap_seconds']:.1f}s | "
            f"{path['e2e_job_seconds']:.1f}s | {path['event_to_e2e_finish_seconds']:.1f}s |"
        )
    rows += ["", "Last successfully finished mandatory jobs (excluding this gate):", ""]
    for job in report["finishers"]:
        rows.append(
            f"- `{job['name']}`: {job['job_seconds']:.1f}s runner; "
            f"{job['event_to_finish_seconds']:.1f}s after event"
        )
    return "\n".join(rows) + "\n"


def fetch(path: str, token: str, repository: str) -> dict:
    url = f"https://api.github.com/repos/{repository}/{path}"
    request = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "alegria-ci-critical-path",
    })
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)


def main() -> int:
    token = os.environ.get("GITHUB_TOKEN", "")
    repository = os.environ.get("GITHUB_REPOSITORY", "")
    run_id = os.environ.get("CI_RUN_ID", "")
    source_sha = os.environ.get("CI_SOURCE_SHA", "")
    if not (token and repository and run_id.isdecimal() and len(source_sha) == 40
            and all(ch in "0123456789abcdef" for ch in source_sha)):
        print("CI_CRITICAL_PATH=UNAVAILABLE missing GitHub run credentials or identity")
        return 0
    try:
        run = fetch(f"actions/runs/{run_id}", token, repository)
        jobs = []
        for page in range(1, 6):
            response = fetch(f"actions/runs/{run_id}/jobs?per_page=100&page={page}", token, repository)
            batch = response["jobs"]
            jobs.extend(batch)
            if len(batch) < 100:
                break
        else:
            raise ValueError("workflow has over 500 jobs; incomplete evidence")
        if run.get("id") != int(run_id) or run.get("head_sha") != source_sha:
            raise ValueError("GitHub workflow run does not attest this exact PR source SHA")
        report = analyze(run, jobs, datetime.now(timezone.utc))
        print("CI_CRITICAL_PATH=" + json.dumps({
            k: v for k, v in report.items() if k != "finishers"
        }, sort_keys=True))
        summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary_path:
            with Path(summary_path).open("a", encoding="utf-8") as stream:
                stream.write(to_summary(report))
    except (OSError, ValueError, KeyError, urllib.error.URLError) as error:
        # The actual selected-job acceptance gate immediately above is still
        # authoritative. A transient GitHub REST outage cannot turn the same
        # verified source into an artificial PR failure or a false SLA PASS.
        print(f"CI_CRITICAL_PATH=UNAVAILABLE {type(error).__name__}: {error}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
