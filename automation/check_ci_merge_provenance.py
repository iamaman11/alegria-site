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

from ci_impact import CI_PR_OWNERS, E2E_ARCH_SHARDS, E2E_VERIFY_SHARDS


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/ci.yml"
MERGE = "m" * 40
SOURCE = "s" * 40
BASE = "b" * 40
JOBS = (
    "impact",
    "architecture-and-rust", "rust-e2e-harness", "rust-e2e-pg-harness", "rust-e2e-legacy-binaries",
    "rust-e2e-v3-binaries",
    *(f"rust-architecture-e2e ({name})" for name in E2E_ARCH_SHARDS),
    *(f"rust-verify-e2e ({name})" for name in E2E_VERIFY_SHARDS),
    "product-truth", "rust-security",
    "cloudflare-static-build", "stack-temporal-postgres18",
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


def pr_gate_source() -> str:
    """Use the actual PR aggregation script, not a hand-written approximation."""
    lines = WORKFLOW.read_text(encoding="utf-8").splitlines()
    marker = "      - name: Require every mandatory job to succeed"
    if lines.count(marker) != 1:
        raise ValueError("exact-source PR gate missing or duplicated")
    begin = lines.index("          python3 - <<'PY'", lines.index(marker)) + 1
    end = lines.index("          PY", begin)
    source = textwrap.dedent("\n".join(lines[begin:end]))
    compile(source, str(WORKFLOW), "exec")
    return source


def execute_pr_gate(
    selected: list[str], *, omitted: str | None = None,
    wrong_status: str | None = None, wrong_sha: bool = False,
) -> tuple[bool, str]:
    results = {
        name: {"result": "success" if name in selected else "skipped"}
        for name in CI_PR_OWNERS if name != omitted
    }
    results["impact"] = {"result": "success"}
    if wrong_status:
        results[wrong_status]["result"] = "skipped" if wrong_status in selected else "success"
    output = io.StringIO()
    with contextlib.redirect_stdout(output), patch.dict(os.environ, {
        "NEEDS_JSON": json.dumps(results),
        "SELECTED_CI_JOBS": json.dumps(selected),
        "CI_PLAN_SHA": "wrong" if wrong_sha else SOURCE,
        "CI_HEAD_SHA": SOURCE,
    }):
        try:
            exec(pr_gate_source(), {"__name__": "__main__"})
        except SystemExit as error:
            return False, str(error) + output.getvalue()
    return True, output.getvalue()


def execute_gate(*, equal_tree: bool = True, omit_job: str | None = None,
                 skipped_job: str | None = None, delayed_pr_indexes: int = 0,
                 missing_pr: bool = False,
                 accepted_run: bool = True,
                 scoped_matrix: bool = False,
                 scoped_matrix_name: str | None = None,
                 omit_matrix_owner: str | None = None) -> tuple[bool, str, set[str]]:
    calls: set[str] = set()
    jobs = [{"name": name, "conclusion": "skipped" if name == skipped_job else "success"}
            for name in JOBS if name != omit_job
            and not (omit_matrix_owner and name.startswith(omit_matrix_owner + " ("))
            and not (scoped_matrix and (name.startswith("rust-architecture-e2e (")
                                        or name.startswith("rust-verify-e2e (")))]
    if scoped_matrix:
        jobs += [{"name": (name if scoped_matrix_name is None
                          else f"{name} ({scoped_matrix_name})"), "conclusion": "skipped"}
                 for name in ("rust-architecture-e2e", "rust-verify-e2e")]
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
    pr_queries = 0

    def urlopen(request: urllib.request.Request, *, timeout: int) -> io.BytesIO:
        nonlocal pr_queries
        if timeout != 20 or not request.full_url.startswith(base_url):
            raise AssertionError("unexpected GitHub request")
        path = request.full_url[len(base_url):]
        calls.add(path)
        if path not in responses:
            raise AssertionError(f"unknown GitHub endpoint: {path}")
        if path == f"commits/{MERGE}/pulls":
            pr_queries += 1
            if missing_pr or pr_queries <= delayed_pr_indexes:
                return io.BytesIO(b"[]")
        return io.BytesIO(json.dumps(responses[path]).encode("utf-8"))

    output = io.StringIO()
    with contextlib.redirect_stdout(output), patch.dict(os.environ, {
        "REPO_NAME": "iamaman11/alegria-site", "MERGE_SHA": MERGE,
        "GITHUB_TOKEN": "offline-test-token",
    }), patch("urllib.request.urlopen", side_effect=urlopen), patch("time.sleep") as sleep:
        try:
            exec(main_gate_source(), {"__name__": "__main__"})
        except SystemExit as failure:
            if delayed_pr_indexes and not missing_pr:
                assert sleep.call_count == min(delayed_pr_indexes, 10)
            return False, str(failure), calls
    if delayed_pr_indexes:
        assert sleep.call_count == min(delayed_pr_indexes, 10)
    return True, output.getvalue(), calls


class MainMergeProvenanceTests(unittest.TestCase):
    def test_protected_main_gate_imports_its_exact_checked_out_source_registry(self) -> None:
        # This gate runs on PR *and* accepted-main push. Its matrix owner
        # import must never depend on the PR-only checkout that existed
        # before sharding; that would block the signed-artifact release.
        source = WORKFLOW.read_text(encoding="utf-8")
        checkout = "      - name: Checkout exact CI plan owner registry\n"
        marker = "      - name: Require every mandatory job to succeed\n"
        self.assertEqual(source.count(checkout), 1)
        _, after = source.split(checkout, 1)
        checkout_step = after.split(marker, 1)[0]
        self.assertIn("uses: actions/checkout@v7", checkout_step)
        self.assertIn("ref: ${{ github.sha }}", checkout_step)
        self.assertNotIn("if: github.event_name == 'pull_request'", checkout_step)
        self.assertIn("from ci_impact import E2E_ARCH_SHARDS", main_gate_source())

    def test_identical_git_trees_and_all_required_green_jobs_accept_without_build(self) -> None:
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

    def test_missing_verify_owner_rejects_previous_pr_proof(self) -> None:
        ok, message, _ = execute_gate(omit_job="verify")
        self.assertFalse(ok)
        self.assertIn("did not pass every mandatory owner", message)

    def test_missing_precompiled_e2e_or_verified_binary_owner_blocks_merge(self) -> None:
        for omitted in (
            "rust-e2e-harness", "rust-e2e-pg-harness", "rust-e2e-legacy-binaries", "rust-e2e-v3-binaries",
        ):
            with self.subTest(omitted=omitted):
                ok, message, _ = execute_gate(omit_job=omitted)
                self.assertFalse(ok)
                self.assertIn("did not pass every mandatory owner", message)

    def test_matrix_real_e2e_shards_are_all_green_and_scoped_skip_is_explicit(self) -> None:
        ok, message, _ = execute_gate()
        self.assertTrue(ok, message)
        for name in (None, "", "not_applicable"):
            with self.subTest(scoped_display=name):
                ok, message, _ = execute_gate(scoped_matrix=True, scoped_matrix_name=name)
                self.assertTrue(ok, message)
        ok, message, _ = execute_gate(scoped_matrix=True, scoped_matrix_name="unknown")
        self.assertFalse(ok)
        self.assertIn("unrecognized E2E shard", message)
        for omitted in ("rust-architecture-e2e", "rust-verify-e2e"):
            with self.subTest(omitted=omitted):
                ok, message, _ = execute_gate(omit_matrix_owner=omitted)
                self.assertFalse(ok)
                self.assertIn("neither selected shards", message)
        for skipped in ("rust-architecture-e2e (graph)", "rust-verify-e2e (verify_d)"):
            with self.subTest(skipped=skipped):
                ok, message, _ = execute_gate(skipped_job=skipped)
                self.assertFalse(ok)
                self.assertIn("did not pass all selected E2E shards", message)

    def test_fresh_merge_waits_for_github_pr_indexing_then_accepts(self) -> None:
        ok, message, _ = execute_gate(delayed_pr_indexes=2)
        self.assertTrue(ok, message)
        self.assertIn("CI_GATE=PASS", message)

    def test_permanently_missing_pr_fails_closed_after_bounded_poll(self) -> None:
        ok, message, _ = execute_gate(missing_pr=True)
        self.assertFalse(ok)
        self.assertIn("merge commit does not match one accepted PR head", message)

    def test_no_accepted_exact_head_run_rejects_reuse(self) -> None:
        ok, message, _ = execute_gate(accepted_run=False)
        self.assertFalse(ok)
        self.assertIn("no successful full CI", message)

    def test_optional_skipped_job_requires_previous_green_pr_aggregate(self) -> None:
        ok, message, _ = execute_gate(skipped_job="stack-storage-graph")
        self.assertTrue(ok, message)
        ok, message, _ = execute_gate(skipped_job="impact")
        self.assertFalse(ok)
        self.assertIn("impact selector or aggregate gate", message)


class PrSelectedOwnersGateTests(unittest.TestCase):
    def test_docs_only_skips_every_unneeded_owner_without_docker(self) -> None:
        ok, message = execute_pr_gate([])
        self.assertTrue(ok, message)
        self.assertIn("CI_GATE=PASS", message)

    def test_full_unknown_impact_requires_all_real_owners(self) -> None:
        ok, message = execute_pr_gate(list(CI_PR_OWNERS))
        self.assertTrue(ok, message)

    def test_missing_owner_or_wrong_sha_fails_closed(self) -> None:
        for params in ({"omitted": "stack-storage-graph"}, {"wrong_sha": True}):
            with self.subTest(params=params):
                ok, message = execute_pr_gate([], **params)
                self.assertFalse(ok)
                self.assertIn("CI_GATE=FAIL", message)

    def test_selected_skipped_or_unselected_executed_fails_closed(self) -> None:
        for selected, wrong in [(["architecture-and-rust"], "architecture-and-rust"),
                                ([], "cloudflare-static-build")]:
            with self.subTest(wrong=wrong):
                ok, message = execute_pr_gate(selected, wrong_status=wrong)
                self.assertFalse(ok)
                self.assertIn("CI_GATE=FAIL", message)


if __name__ == "__main__":
    unittest.main()
