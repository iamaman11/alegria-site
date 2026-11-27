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
import re
import urllib.error
import urllib.request


# Every row is an existing source-attested runner and its actual predecessors.
# No invented build/test durations and no change to the workflow's `needs`.
NATIVE_QDRANT_COMPILERS = (
    "architecture-and-rust",
    "rust-e2e-qdrant-temporal-harness",
    "rust-e2e-qdrant-effects-binaries",
    "rust-e2e-v3-binaries",
    "rust-e2e-legacy-binaries",
)
E2E_PATHS = {
    "manual-temporal": (
        "rust-temporal-legacy-e2e (temporal_manual)",
        ("rust-e2e-temporal-harness", "rust-e2e-legacy-binaries"),
    ),
    "native-qdrant": (
        "rust-qdrant-temporal-e2e (temporal_qdrant)",
        NATIVE_QDRANT_COMPILERS,
    ),
    "native-mixed-v6": (
        "rust-qdrant-temporal-e2e (temporal_mixed_v6)", NATIVE_QDRANT_COMPILERS,
    ),
    "native-mixed-v7": (
        "rust-qdrant-temporal-e2e (temporal_mixed_v7)", NATIVE_QDRANT_COMPILERS,
    ),
    "native-mixed-v2": (
        "rust-qdrant-temporal-e2e (temporal_mixed_v2)", NATIVE_QDRANT_COMPILERS,
    ),
    "native-dataforseo": (
        "rust-qdrant-temporal-e2e (temporal_dataforseo)", NATIVE_QDRANT_COMPILERS,
    ),
    "outbox": (
        "rust-outbox-e2e (outbox)",
        ("rust-e2e-outbox-harness", "rust-e2e-legacy-binaries"),
    ),
}

# Actual YAML step labels, deliberately not regex-based guesses about whether
# Cargo ran. These are observable GitHub step durations, NOT per-rustc CPU time.
COMPILER_STEPS = {
    "architecture-and-rust": "Compile independent DataForSEO, pure Acquisition and source issuer binaries",
    "rust-e2e-temporal-harness": "Compile real Temporal/full-infra owner E2E test executable",
    "rust-e2e-legacy-binaries": "Compile canonical Worker, Outbox and historical Temporal in one Cargo pass",
    "rust-e2e-qdrant-temporal-harness": "Compile distinct live Temporal/PG/Qdrant E2E test executable",
    "rust-e2e-qdrant-effects-binaries": "Compile real Qdrant/PG Activity worker without contaminating pure parent",
    "rust-e2e-v3-binaries": "Compile default provider-free Temporal DAG and independent PostgreSQL Worker",
    "rust-e2e-outbox-harness": "Compile independent exact-source A14 Outbox E2E test executable",
}
# No additional Cargo interval: the canonical Worker, Outbox and historical
# Temporal binary now share one Cargo feature resolution in the existing job.
COMPILER_FINAL_STEPS = {}
LIVE_E2E_STEPS = {
    "manual-temporal": "Execute exact selected temporal legacy real E2E shard",
    "native-qdrant": "Prove live Temporal history and durable PostgreSQL/Qdrant receipt",
    "native-mixed-v6": "Prove live Temporal history and durable PostgreSQL/Qdrant receipt",
    "native-mixed-v7": "Prove live Temporal history and durable PostgreSQL/Qdrant receipt",
    "native-mixed-v2": "Prove live Temporal history and durable PostgreSQL/Qdrant receipt",
    "native-dataforseo": "Prove live Temporal history and durable PostgreSQL/Qdrant receipt",
    "outbox": "Execute nine exact A14 PostgreSQL/Worker E2E checks",
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


def observed_step(job: dict, step_name: str) -> tuple[datetime, datetime]:
    """No missing/failed/duplicate Cargo or live E2E step can masquerade as timing."""
    matches = [step for step in job.get("steps") or [] if step.get("name") == step_name]
    if len(matches) != 1 or matches[0].get("conclusion") != "success":
        raise ValueError(f"missing successful source-verified step {step_name} in {job['name']}")
    step = matches[0]
    started = timestamp(step.get("started_at"))
    finished = timestamp(step.get("completed_at"))
    job_start = timestamp(job.get("started_at"))
    job_end = timestamp(job.get("completed_at"))
    if not (started and finished and job_start and job_end
            and job_start <= started <= finished <= job_end):
        raise ValueError(f"out-of-job timestamp for {step_name} in {job['name']}")
    return started, finished


def observed_compiler_span(job: dict, owner: str) -> tuple[datetime, datetime]:
    first, finished = observed_step(job, COMPILER_STEPS[owner])
    additional = COMPILER_FINAL_STEPS.get(owner)
    if additional is not None:
        later_started, later_finished = observed_step(job, additional)
        if later_started < finished:
            raise ValueError(f"historical Temporal compile started before canonical Worker compile completed in {owner}")
        finished = later_finished
    return first, finished


def analyze(
    run: dict, jobs: list[dict], observed_at: datetime,
    *, checkout_sha: str | None = None, expected_attempt: int | None = None,
) -> dict:
    """Explain observed elapsed time without treating skips as passing tests."""
    if run.get("event") != "pull_request":
        raise ValueError("PR-only report received a non-PR workflow")
    if not re.fullmatch(r"[0-9a-f]{40}", run.get("head_sha") or ""):
        raise ValueError("PR workflow run is missing its exact canonical source SHA")
    if checkout_sha is not None and not re.fullmatch(r"[0-9a-f]{40}", checkout_sha):
        raise ValueError("CI checkout requires an exact synthetic-merge source SHA")
    if expected_attempt is not None and run.get("run_attempt") != expected_attempt:
        raise ValueError("GitHub run attempt differs from the reporter invocation")
    created = timestamp(run.get("created_at"))
    if created is None or observed_at < created:
        raise ValueError("missing or future GitHub workflow creation timestamp")
    by_name: dict[str, dict] = {}
    completed: list[dict] = []
    for job in jobs:
        name = job.get("name")
        if not isinstance(name, str) or name in by_name:
            raise ValueError("missing or duplicate GitHub job display name")
        if ("run_id" in job or (checkout_sha is not None and job.get("conclusion") == "success")) \
                and job.get("run_id") != run.get("id"):
            raise ValueError(f"job {name} belongs to a different workflow run")
        if expected_attempt is not None and "run_attempt" in job and job["run_attempt"] != expected_attempt:
            raise ValueError(f"job {name} belongs to a different workflow run attempt")
        if ("head_sha" in job or (checkout_sha is not None and job.get("conclusion") == "success")) \
                and job.get("head_sha") not in {run["head_sha"], checkout_sha}:
            raise ValueError(f"job {name} is from an unrelated source checkout SHA")
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
        compile_steps = []
        for upstream_name, finished in upstream:
            compile_started, compile_finished = observed_compiler_span(
                by_name[upstream_name], upstream_name,
            )
            compile_steps.append({
                "owner": upstream_name,
                "setup_before_compile_seconds": seconds(
                    timestamp(by_name[upstream_name]["started_at"]), compile_started,
                ),
                "cargo_step_seconds": seconds(compile_started, compile_finished),
                "compile_to_artifact_job_finish_seconds": seconds(compile_finished, finished),
                "event_to_compile_finish_seconds": seconds(created, compile_finished),
            })
        live_started, live_finished = observed_step(runner, LIVE_E2E_STEPS[label])
        if live_started < last_dep_finish:
            raise ValueError(f"real {label} E2E step executed before its required compiler completed")
        paths.append({
            "route": label,
            "last_compiler": slowest_dep_name,
            "compiler_ready_seconds": seconds(created, last_dep_finish),
            "runner_gap_seconds": seconds(last_dep_finish, runner_started),
            "e2e_job_seconds": seconds(runner_started, runner_done),
            "e2e_runner_setup_and_artifact_seconds": seconds(runner_started, live_started),
            "real_e2e_step_seconds": seconds(live_started, live_finished),
            "e2e_job_epilogue_seconds": seconds(live_finished, runner_done),
            "event_to_e2e_finish_seconds": seconds(created, runner_done),
            "compiler_steps": compile_steps,
        })
    return {
        "run_id": run.get("id"),
        "head_sha": run.get("head_sha"),
        "checkout_sha": checkout_sha,
        "run_attempt": run.get("run_attempt"),
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
        f"Source: PR workflow `{report['run_id']}` attempt `{report['run_attempt']}`, "
        f"canonical PR head `{report['head_sha']}`, "
        f"actual checkout `{report['checkout_sha'] or 'unavailable'}`. "
        "These commit IDs may differ; successful required jobs bind their artifacts to the checkout SHA.",
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
    rows += ["", "Precise runner stages from GitHub's successful steps (seconds):", "",
             "| Route | Cargo owner (actual step) | Setup | Cargo | Pack/upload | Download/setup | Live E2E |",
             "| --- | --- | ---: | ---: | ---: | ---: | ---: |"]
    for path in report["measured_paths"]:
        for compiler in path["compiler_steps"]:
            rows.append(
                f"| {path['route']} | `{compiler['owner']}` | "
                f"{compiler['setup_before_compile_seconds']:.1f} | "
                f"{compiler['cargo_step_seconds']:.1f} | "
                f"{compiler['compile_to_artifact_job_finish_seconds']:.1f} | "
                f"{path['e2e_runner_setup_and_artifact_seconds']:.1f} | "
                f"{path['real_e2e_step_seconds']:.1f} |"
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
    checkout_sha = os.environ.get("CI_CHECKOUT_SHA", "")
    run_attempt = os.environ.get("CI_RUN_ATTEMPT", "")
    if not (token and repository and run_id.isdecimal() and len(source_sha) == 40
            and all(ch in "0123456789abcdef" for ch in source_sha)
            and re.fullmatch(r"[0-9a-f]{40}", checkout_sha)
            and run_attempt.isdecimal() and int(run_attempt) > 0):
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
        report = analyze(run, jobs, datetime.now(timezone.utc),
                         checkout_sha=checkout_sha, expected_attempt=int(run_attempt))
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
