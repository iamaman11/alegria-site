#!/usr/bin/env python3
"""Execute the actual protected-main CI gate against offline GitHub fixtures."""

from __future__ import annotations

import contextlib
import io
import json
import os
import textwrap
import unittest
import urllib.request
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/ci.yml"
MERGE = "m" * 40
SOURCE = "s" * 40
BASE = "b" * 40
JOBS = (
    "architecture-and-rust", "rust-contract-smokes", "product-truth", "rust-security",
    "cloudflare-static-build", "temporal-worker-docker", "stack-temporal-postgres18",
    "stack-storage-graph", "verify", "Native Windows first-site operator regression", "CI",
)


def main_gate_source() -> str:
    """Get the inline Python used by the real main-merge gate, not a duplicate."""
    lines = WORKFLOW.read_text(encoding="utf-8").splitlines()
    marker = "      - name: Verify main merge comes from a fully successful PR CI"
    if lines.count(marker) != 1:
        raise ValueError("protected main gate missing or duplicated")
    start = lines.index(marker)
    try:
        begin = lines.index("          python3 - <<'PY'", start) + 1
        end = lines.index("          PY", begin)
    except ValueError as error:
        raise ValueError("protected main gate source not found") from error
    source = textwrap.dedent("\n".join(lines[begin:end]))
    compile(source, str(WORKFLOW), "exec")
    return source


def execute_gate(*, equal_tree: bool = True, omit_job: str | None = None,
                 accepted_run: bool = True) -> tuple[bool, str, set[str]]:
    calls: set[str] = set()
    jobs = [{"name": name, "conclusion": "success"} for name in JOBS if name != omit_job]
    responses = {
        f"commits/{MERGE}": {"parents": [{"sha": BASE}, {"sha": SOURCE}]},
        f"commits/{MERGE}/pulls": [{"merged_at": "2026-11-20T21:00:00Z",
                                      "merge_commit_sha": MERGE, "head": {"sha": SOURCE},
                                      "base": {"ref": "main"}, "number": 38}],
        f"git/commits/{MERGE}": {"tree": {"sha": "a" * 40}},
        f"git/commits/{SOURCE}": {"tree": {"sha": ("a" if equal_tree else "c") * 40}},
        f"actions/runs?head_sha={SOURCE}&per_page=100": {"workflow_runs": ([{
            "head_sha": SOURCE, "event": "pull_request", "name": "ci",
            "status": "completed", "conclusion": "success", "id": 42,
        }] if accepted_run else [])},
        "actions/runs/42/jobs?per_page=100": {"jobs": jobs},
    }
    base_url = "https://api.github.com/repos/iamaman11/alegria-site/"

    def urlopen(request: urllib.request.Request, *, timeout: int) -> io.BytesIO:
        if timeout != 20 or not request.full_url.startswith(base_url):
            raise AssertionError("unexpected GitHub request")
        path = request.full_url[len(base_url):]
        calls.add(path)
        if path not in responses:
            raise AssertionError(f"unknown GitHub endpoint: {path}")
        return io.BytesIO(json.dumps(responses[path]).encode("utf-8"))

    output = io.StringIO()
    with contextlib.redirect_stdout(output), patch.dict(os.environ, {
        "REPO_NAME": "iamaman11/alegria-site", "MERGE_SHA": MERGE,
        "GITHUB_TOKEN": "offline-test-token",
    }), patch("urllib.request.urlopen", side_effect=urlopen):
        try:
            exec(main_gate_source(), {"__name__": "__main__"})
        except SystemExit as failure:
            return False, str(failure), calls
    return True, output.getvalue(), calls


class MainMergeProvenanceTests(unittest.TestCase):
    def test_identical_git_trees_and_all_eleven_green_jobs_accept_without_build(self) -> None:
        ok, message, calls = execute_gate()
        self.assertTrue(ok, message)
        self.assertIn("CI_GATE=PASS", message)
        self.assertIn(f"git/commits/{MERGE}", calls)
        self.assertIn(f"git/commits/{SOURCE}", calls)

    def test_different_merge_tree_rejects_previous_pr_proof(self) -> None:
        ok, message, _ = execute_gate(equal_tree=False)
        self.assertFalse(ok)
        self.assertIn("merge tree differs", message)

    def test_missing_owner_job_rejects_previous_pr_proof(self) -> None:
        ok, message, _ = execute_gate(omit_job="product-truth")
        self.assertFalse(ok)
        self.assertIn("did not pass every mandatory owner", message)

    def test_missing_parallel_rust_smoke_rejects_previous_pr_proof(self) -> None:
        ok, message, _ = execute_gate(omit_job="rust-contract-smokes")
        self.assertFalse(ok)
        self.assertIn("did not pass every mandatory owner", message)

    def test_no_accepted_exact_head_run_rejects_reuse(self) -> None:
        ok, message, _ = execute_gate(accepted_run=False)
        self.assertFalse(ok)
        self.assertIn("no successful full CI", message)


if __name__ == "__main__":
    unittest.main()
