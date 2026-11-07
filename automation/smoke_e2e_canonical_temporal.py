#!/usr/bin/env python3
"""Offline proof that R5 drives the one canonical durable Temporal workflow."""
from __future__ import annotations

import os
from unittest import mock
import subprocess
import inspect
import e2e_supersite_certification as cert
from uuid import UUID

from e2e_supersite_certification import canonical_temporal_command, parse_result


def main() -> int:
    run_id = "9c337079-ae0b-4cb4-b052-bb3a0ca623c1"
    with mock.patch.dict(os.environ, {"ALEGRIA_CERT_CITIZENSHIP_CODE": "BY"}):
        command = canonical_temporal_command(
            queries=["spain tourist visa belarus official"],
            run_id=run_id,
            query_batch_key="test-batch",
            publish=True,
        )
    assert str(UUID(run_id)) == run_id
    assert command[0:6] == [
        "docker", "compose", "exec", "-T", "temporal-worker", "temporal_starter"
    ], command
    assert command[command.index("--temporal-url") + 1] == "http://temporal-server:7233"
    assert "cargo" not in command
    assert command[command.index("--workflow") + 1] == "seo-site-build-canonical-cutover"
    assert command[command.index("--run-mode") + 1] == "publish_with_hitl"
    assert command[command.index("--citizenship-code") + 1] == "BY"
    assert command[-3:-1] != ["--publish", "full"]
    assert "--wait" in command
    assert "cli_tools" not in command
    result_text = (
        "WORKFLOW_RESULT workflow_type=SeoSiteBuildCanonicalCutoverWorkflow "
        f"workflow_id={run_id} "
        f"result=seo_site_build_canonical_cutover_ok run_id={run_id} "
        "ia_pages=3 published_pages=3 truth_verified=4 rebuild_verdict=not_run\n"
    )
    result = parse_result(result_text)
    assert result["mode"] == "temporal_durable"
    assert result["status"] == "published"
    assert result["page_total"] == 3
    assert result["truth_verified"] == 4
    assert parse_result(result_text.replace("published_pages=3", "published_pages=0"))[
        "status"
    ].startswith("blocked:")
    assert not parse_result(result_text.replace("truth_verified=4", "truth_verified=0"))[
        "status"
    ] == "published"
    assert parse_result(
        result_text.replace("seo_site_build_canonical_cutover_ok", "blocked:no_truth")
    ) == {}
    assert parse_result(
        "SEO_RUN_RESULT scenario=site_build_full mode=semi_auto_operator "
        "status=done page_total=3 published_pages=3 changed_truth_keys=3"
    ) == {}
    # Read-only readiness must never call the synthetic producer or destructive
    # restore drill; check the actual commands, not a documentary assertion.
    called = []
    def record(command, *, env=None):
        called.append(command)
        return subprocess.CompletedProcess(command, 0, stdout="ok", stderr="")
    with mock.patch.dict(os.environ, {
        "ALEGRIA_CERT_BASE_URL": "https://seo-staging.alegria.by",
    }), mock.patch.object(cert, "run_command", side_effect=record):
        readiness = cert.clean_runtime_preflight()
    assert len(readiness) == 3 and all(
        check["status"] == "PASS" for check in readiness.values()
    ), readiness
    assert len(called) == 3
    assert all(c[:3] == ["docker", "compose", "exec"] for c in called)
    forbidden = ("temporal_production_gate.sh", "restore_drill.sh",
                 "cms-approve-publish", "DROP SCHEMA", "seed_admissible")
    assert not any(token in repr(called) for token in forbidden), called
    main_source = inspect.getsource(cert.main)
    assert "temporal_production_gate.sh" not in main_source
    assert "restore_drill.sh" not in main_source
    with mock.patch.dict(os.environ, {
        "ALEGRIA_CERT_BASE_URL": "https://seo-staging.alegria.by",
    }), mock.patch.object(cert, "run_command", return_value=subprocess.CompletedProcess(
        ["docker", "compose", "exec"], 1, stdout="", stderr="error"
    )):
        readiness = cert.clean_runtime_preflight()
    assert len(readiness) == 1 and readiness["postgres_readiness"]["status"] == "FAIL"
    old = {
        "rule-a": {"params": {"amount": 35}, "source_snapshot_hash": "hash-1",
                   "source_key": "official"},
    }
    new = {
        "rule-a": {"params": {"amount": 40}, "source_snapshot_hash": "hash-2",
                   "source_key": "official"},
    }
    assert cert.factual_change_proof(
        old, new, {"visa/index.html": "old"}, {"visa/index.html": "new"}
    )["status"] == "PASS"
    assert cert.factual_change_proof(
        old, old, {"visa/index.html": "old"}, {"visa/index.html": "new"}
    )["status"] == "FAIL"
    assert cert.factual_change_proof(
        old, new, {"visa/index.html": "same"}, {"visa/index.html": "same"}
    )["status"] == "FAIL"
    assert cert.factual_change_proof(
        {}, new, {"visa/index.html": "old"}, {"visa/index.html": "new"}
    )["status"] == "FAIL"
    print("E2E_CANONICAL_TEMPORAL_SMOKE: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
