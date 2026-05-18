#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from local_env import resolve_database_url


ROOT = Path(__file__).resolve().parents[1]
RUNS_DIR = ROOT / "docs" / "runs"
MANIFEST = RUNS_DIR / "r5_supersite_certification_manifest.json"


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def run_command(args: list[str], *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=ROOT,
        text=True,
        capture_output=True,
        env=env,
        check=False,
    )


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def write_markdown(path: Path, title: str, payload: dict[str, Any]) -> None:
    lines = [
        f"# {title}",
        "",
        f"- status: {payload.get('evidence_status')}",
        f"- scenario: {payload.get('scenario_id')}",
        f"- run_id: {payload.get('run_id', '')}",
        f"- workflow_id: {payload.get('workflow_id', '')}",
        f"- provider: {payload.get('provider_kind', '')}",
        f"- updated_at: {payload.get('updated_at')}",
        "",
        "```json",
        json.dumps(payload, indent=2, ensure_ascii=False),
        "```",
        "",
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")


def classify_provider() -> tuple[str | None, str]:
    provider = os.environ.get("SEO_LLM_PROVIDER", "").strip()
    if provider in {"openai", "anthropic", "gemini"}:
        key_present = {
            "openai": bool(os.environ.get("OPENAI_API_KEY")),
            "anthropic": bool(os.environ.get("ANTHROPIC_API_KEY")),
            "gemini": bool(os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY")),
        }[provider]
        return (provider if key_present else None, provider)
    if provider == "local_compatible":
        return (None, provider)
    if os.environ.get("OPENAI_API_KEY"):
        return "openai", "openai"
    if os.environ.get("ANTHROPIC_API_KEY"):
        return "anthropic", "anthropic"
    if os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY"):
        return "gemini", "gemini"
    if os.environ.get("SEO_LLM_LOCAL_ENDPOINT"):
        return None, "local_compatible"
    return None, "unconfigured"


def seo_run_command(
    *,
    scenario: str,
    queries: list[str],
    run_id: str,
    query_batch_key: str,
    output_dir: Path,
    publish: bool,
) -> list[str]:
    cmd = [
        "cargo",
        "run",
        "-q",
        "-p",
        "cli_tools",
        "--",
        "seo-run",
        "--database-url",
        resolve_database_url(),
        "--run-id",
        run_id,
        "--market",
        os.environ.get("ALEGRIA_CERT_MARKET", "alegria-site"),
        "--locale",
        os.environ.get("ALEGRIA_CERT_LOCALE", "ru-RU"),
        "--country-code",
        os.environ.get("ALEGRIA_CERT_COUNTRY_CODE", "ES"),
        "--visa-type",
        os.environ.get("ALEGRIA_CERT_VISA_TYPE", "tourist"),
        "--citizenship-code",
        os.environ.get("ALEGRIA_CERT_CITIZENSHIP_CODE", "BY"),
        "--applicant-profile",
        os.environ.get("ALEGRIA_CERT_APPLICANT_PROFILE", "standard"),
        "--query-batch-key",
        query_batch_key,
        "--output-dir",
        str(output_dir),
        "--base-url",
        os.environ.get("ALEGRIA_CERT_BASE_URL", "https://example.com"),
    ]
    for query in queries:
        cmd.extend(["--query", query])
    if publish:
        cmd.append("--publish")
    cmd.append(scenario)
    return cmd


def parse_result(output: str) -> dict[str, Any]:
    match = re.search(
        r"SEO_RUN_RESULT scenario=(?P<scenario>\S+) mode=(?P<mode>\S+) status=(?P<status>\S+) "
        r"page_total=(?P<page_total>\d+) published_pages=(?P<published_pages>\d+) "
        r"changed_truth_keys=(?P<changed_truth_keys>\d+)",
        output,
    )
    if not match:
        return {}
    data = match.groupdict()
    return {
        "scenario": data["scenario"],
        "mode": data["mode"],
        "status": data["status"],
        "page_total": int(data["page_total"]),
        "published_pages": int(data["published_pages"]),
        "changed_truth_keys": int(data["changed_truth_keys"]),
    }


def scenario_payload(
    *,
    scenario_id: str,
    title: str,
    seo_scenario: str,
    queries: list[str],
    publish: bool,
    provider_kind: str,
    gate_results: dict[str, Any],
) -> dict[str, Any]:
    run_id = str(datetime.now(timezone.utc).timestamp()).replace(".", "")
    query_batch_key = f"r5-{scenario_id}-{run_id}"
    output_dir = ROOT / "tmp" / f"r5-{scenario_id}-{run_id}"
    output_dir.mkdir(parents=True, exist_ok=True)
    command = seo_run_command(
        scenario=seo_scenario,
        queries=queries,
        run_id=run_id,
        query_batch_key=query_batch_key,
        output_dir=output_dir,
        publish=publish,
    )
    proc = run_command(command)
    combined = "\n".join(chunk for chunk in [proc.stdout.strip(), proc.stderr.strip()] if chunk.strip())
    result = parse_result(proc.stdout)
    phase_lines = [
        line
        for line in proc.stdout.splitlines()
        if line.startswith("SEO_RUN_PHASE ")
    ]
    evidence_status = "PASS" if proc.returncode == 0 else "FAIL"
    failure_class = "none" if proc.returncode == 0 else "certification_run_failed"
    if proc.returncode != 0:
        lower = combined.lower()
        if any(token in lower for token in ["401", "403", "unauthorized", "authentication failed"]):
            failure_class = "credential_issue"
        elif any(token in lower for token in ["429", "rate limit", "too many requests"]):
            failure_class = "rate_or_transport_issue"
        elif "dataforseo" in lower or "serp" in lower:
            failure_class = "provider_contract_issue"
        elif "crawl_sources" in lower or "http_status=" in lower:
            failure_class = "crawl_policy_failure"
        elif "rebuild" in lower:
            failure_class = "rebuild_propagation_failure"

    payload: dict[str, Any] = {
        "artifact_id": f"r5_{scenario_id}",
        "title": title,
        "scenario_id": scenario_id,
        "step": "R5.Step4",
        "evidence_status": evidence_status,
        "failure_class": failure_class,
        "status_reason": "Live certification scenario completed." if proc.returncode == 0 else "Live certification scenario failed.",
        "provider_kind": provider_kind,
        "publish_requested": publish,
        "run_id": run_id,
        "workflow_id": run_id,
        "query_batch_key": query_batch_key,
        "queries": queries,
        "command": " ".join(command),
        "exit_code": proc.returncode,
        "result": result,
        "phase_lines": phase_lines,
        "stdout_excerpt": proc.stdout.strip()[-4000:],
        "stderr_excerpt": proc.stderr.strip()[-4000:],
        "updated_at": utc_now(),
        "gate_results": gate_results,
    }

    md_name = f"supersite_e2e_{scenario_id}_{datetime.now(timezone.utc).strftime('%Y%m%d')}.md"
    write_markdown(RUNS_DIR / md_name, title, payload)
    return payload


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    provider_live, provider_kind = classify_provider()
    database_url = resolve_database_url()

    if args.dry_run:
        print("R5 certification dry-run")
        print(f"- database_url={database_url}")
        print(f"- provider_kind={provider_kind}")
        print("- gate: temporal_production_gate.sh")
        print("- gate: infra/backups/restore_drill.sh")
        print("- scenarios: fresh_scope, scope_expansion, factual_change_rebuild")
        return 0

    required = {
        "DATAFORSEO_LOGIN": bool(os.environ.get("DATAFORSEO_LOGIN")),
        "DATAFORSEO_PASSWORD": bool(os.environ.get("DATAFORSEO_PASSWORD")),
        "production_llm_provider": provider_live is not None,
    }
    if not all(required.values()):
        manifest = {
            "artifact_id": "r5_supersite_certification_manifest",
            "step": "R5.Step4",
            "evidence_status": "PENDING_CREDENTIALS",
            "failure_class": "credential_issue",
            "status_reason": "Live certification requires DataForSEO credentials and one configured production LLM provider.",
            "provider_kind": provider_kind,
            "required": required,
            "updated_at": utc_now(),
        }
        write_json(MANIFEST, manifest)
        print("E2E_SUPERSITE_CERTIFICATION: PENDING_CREDENTIALS")
        return 2

    gate_results: dict[str, Any] = {}

    production_gate = run_command(["bash", "automation/temporal_production_gate.sh"], env=os.environ.copy())
    gate_results["temporal_production_gate"] = {
        "exit_code": production_gate.returncode,
        "stdout_excerpt": production_gate.stdout.strip()[-4000:],
        "stderr_excerpt": production_gate.stderr.strip()[-4000:],
    }
    if production_gate.returncode != 0:
        manifest = {
            "artifact_id": "r5_supersite_certification_manifest",
            "step": "R5.Step4",
            "evidence_status": "FAIL",
            "failure_class": "production_gate_failure",
            "status_reason": "Temporal production gate failed before certification runs.",
            "provider_kind": provider_kind,
            "gate_results": gate_results,
            "updated_at": utc_now(),
        }
        write_json(MANIFEST, manifest)
        print("E2E_SUPERSITE_CERTIFICATION: FAILED")
        return production_gate.returncode

    restore_drill = run_command(["bash", "infra/backups/restore_drill.sh"], env=os.environ.copy())
    gate_results["restore_drill"] = {
        "exit_code": restore_drill.returncode,
        "stdout_excerpt": restore_drill.stdout.strip()[-4000:],
        "stderr_excerpt": restore_drill.stderr.strip()[-4000:],
    }
    if restore_drill.returncode != 0:
        manifest = {
            "artifact_id": "r5_supersite_certification_manifest",
            "step": "R5.Step4",
            "evidence_status": "FAIL",
            "failure_class": "restore_drill_failure",
            "status_reason": "Restore drill failed before certification runs.",
            "provider_kind": provider_kind,
            "gate_results": gate_results,
            "updated_at": utc_now(),
        }
        write_json(MANIFEST, manifest)
        print("E2E_SUPERSITE_CERTIFICATION: FAILED")
        return restore_drill.returncode

    scenarios = [
        (
            "fresh_scope",
            "Fresh Scope",
            "full",
            [os.environ.get("ALEGRIA_CERT_QUERY_1", "spain tourist visa belarus official")],
        ),
        (
            "scope_expansion",
            "Scope Expansion",
            "full",
            [
                os.environ.get("ALEGRIA_CERT_QUERY_1", "spain tourist visa belarus official"),
                os.environ.get("ALEGRIA_CERT_QUERY_2", "spain visa appointment requirements belarus"),
            ],
        ),
        (
            "factual_change_rebuild",
            "Factual Change And Rebuild",
            "rebuild",
            [
                os.environ.get("ALEGRIA_CERT_QUERY_1", "spain tourist visa belarus official"),
                os.environ.get("ALEGRIA_CERT_QUERY_3", "spain tourist visa fee belarus official"),
            ],
        ),
    ]

    manifest = {
        "artifact_id": "r5_supersite_certification_manifest",
        "step": "R5.Step4",
        "evidence_status": "PASS",
        "failure_class": "none",
        "status_reason": "Three live certification scenarios completed.",
        "provider_kind": provider_kind,
        "gate_results": gate_results,
        "scenarios": [],
        "updated_at": utc_now(),
    }

    for scenario_id, title, seo_scenario, queries in scenarios:
        payload = scenario_payload(
            scenario_id=scenario_id,
            title=title,
            seo_scenario=seo_scenario,
            queries=queries,
            publish=os.environ.get("ALEGRIA_CERT_PUBLISH", "0") == "1",
            provider_kind=provider_kind,
            gate_results=gate_results,
        )
        manifest["scenarios"].append(
            {
                "artifact_id": payload["artifact_id"],
                "title": payload["title"],
                "scenario_id": payload["scenario_id"],
                "evidence_status": payload["evidence_status"],
                "failure_class": payload["failure_class"],
                "run_id": payload["run_id"],
                "workflow_id": payload["workflow_id"],
                "report_path": f"docs/runs/supersite_e2e_{scenario_id}_{datetime.now(timezone.utc).strftime('%Y%m%d')}.md",
            }
        )
        if payload["evidence_status"] != "PASS":
            manifest["evidence_status"] = "FAIL"
            manifest["failure_class"] = payload["failure_class"]
            manifest["status_reason"] = f"Certification scenario `{scenario_id}` failed."
            break

    write_json(MANIFEST, manifest)
    print(f"E2E_SUPERSITE_CERTIFICATION: {manifest['evidence_status']}")
    return 0 if manifest["evidence_status"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
