#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import hashlib
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from uuid import uuid4

from deploy_seo_static_cloudflare import EXPECTED_ACCOUNT, EXPECTED_ORIGIN


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
    if provider in {"local_compatible", "chat_manual"}:
        # Manual ChatGPT tasks can complete the canonical Temporal workflow,
        # but they are not an unattended API-provider credential.
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


def certification_requirements(
    provider_live: str | None, *, supervised_manual: bool = False,
) -> dict[str, bool]:
    """One non-mutating policy shared by preview and launch.

    Only configuration presence is checked here. Actual provider capability,
    evidence, approval and public bytes are certified by the live gates.
    """
    from check_public_static_output import validate_origin

    return {
        "DATAFORSEO_LOGIN": bool(os.environ.get("DATAFORSEO_LOGIN")),
        "DATAFORSEO_PASSWORD": bool(os.environ.get("DATAFORSEO_PASSWORD")),
        "VOYAGE_API_KEY": bool(os.environ.get("VOYAGE_API_KEY")),
        "production_llm_provider": provider_live is not None or (
            supervised_manual
            and os.environ.get("SEO_LLM_PROVIDER") == "chat_manual"
            and os.environ.get("SEO_TRUTH_LLM_PROVIDER") == "chat_manual"
        ),
        "dedicated_cloudflare_account": os.environ.get("CLOUDFLARE_ACCOUNT_ID") == EXPECTED_ACCOUNT,
        "dedicated_workers_dev_origin": os.environ.get("ALEGRIA_CERT_BASE_URL") == EXPECTED_ORIGIN,
        "publication_explicitly_requested": os.environ.get("ALEGRIA_CERT_PUBLISH") == "1",
        "real_https_origin": not validate_origin(
            os.environ.get("ALEGRIA_CERT_BASE_URL", "https://example.com")
        ),
        "public_output_dir_configured": bool(os.environ.get("ALEGRIA_CERT_PUBLIC_OUTPUT_DIR")),
        "worker_origin_matches_certification": (
            os.environ.get("SEO_STATIC_BASE_URL") ==
            os.environ.get("ALEGRIA_CERT_BASE_URL")
            and bool(os.environ.get("SEO_STATIC_BASE_URL"))
        ),
    }


def canonical_temporal_command(
    *,
    queries: list[str],
    run_id: str,
    query_batch_key: str,
    publish: bool,
) -> list[str]:
    if not publish:
        raise ValueError("canonical launch certification requires real HITL publication")
    cmd = [
        # Execute the starter already bundled in the canonical Temporal worker
        # container. No host Rust toolchain, extra service or secondary engine.
        "docker", "compose", "exec", "-T", "temporal-worker",
        "temporal_starter",
        "--temporal-url", "http://temporal-server:7233",
        "start",
        "--workflow", "seo-site-build-canonical-cutover",
        "--workflow-id", run_id,
        # Registration reuses DATABASE_URL from the worker's Compose environment.
        "--market", os.environ.get("ALEGRIA_CERT_MARKET", "alegria-site"),
        "--locale", os.environ.get("ALEGRIA_CERT_LOCALE", "ru-RU"),
        "--country-code", os.environ.get("ALEGRIA_CERT_COUNTRY_CODE", "ES"),
        "--visa-type", os.environ.get("ALEGRIA_CERT_VISA_TYPE", "tourist"),
        "--citizenship-code", os.environ.get("ALEGRIA_CERT_CITIZENSHIP_CODE", "BY"),
        "--applicant-profile",
        os.environ.get("ALEGRIA_CERT_APPLICANT_PROFILE", "standard"),
        "--query-batch-key", query_batch_key,
        "--run-mode", "publish_with_hitl",
        "--wait",
    ]
    for query in queries:
        cmd.extend(["--query", query])
    return cmd


def existing_canonical_temporal_command(run_id: str) -> list[str]:
    """Reattach only; never re-register a context or start a new Temporal run."""
    return [
        "docker", "compose", "exec", "-T", "temporal-worker",
        "temporal_starter", "--temporal-url", "http://temporal-server:7233",
        "workflow-wait", "--workflow-id", run_id,
    ]


def first_site_session(run_id: str) -> Path:
    return RUNS_DIR / f"first_site_session_{run_id}.json"


def record_first_site_session(run_id: str, *, resume: bool) -> None:
    """Durable operator receipt before subprocess.run(... --wait) may be killed."""
    path = first_site_session(run_id)
    if not resume:
        for previous in RUNS_DIR.glob("first_site_session_*.json"):
            try:
                payload = json.loads(previous.read_text(encoding="utf-8"))
            except (OSError, ValueError) as exc:
                raise RuntimeError(
                    f"unreadable first-site session {previous.name}: inspect it before starting"
                ) from exc
            if payload.get("status") == "WAITING_FOR_RESULT":
                raise RuntimeError(
                    f"existing first-site run {payload.get('run_id')} may still be active; "
                    "reattach with --resume-run-id instead of starting another workflow"
                )
    write_json(path, {
        "kind": "first_site_canonical_temporal_session",
        "run_id": run_id,
        "workflow_id": run_id,
        "status": "WAITING_FOR_RESULT",
        "action": "REATTACH" if resume else "START_REQUESTED",
        "updated_at": utc_now(),
    })
    print(
        f"FIRST_SITE_RUN_ID={run_id} action={'REATTACH' if resume else 'START_REQUESTED'}",
        flush=True,
    )


def parse_result(output: str) -> dict[str, Any]:
    # This is the canonical Temporal completion report, never semi-auto SEO_RUN_RESULT.
    match = re.search(
        r"^WORKFLOW_RESULT workflow_type=SeoSiteBuildCanonicalCutoverWorkflow "
        r"workflow_id=(?P<workflow_id>\S+) result=(?P<body>.+)$",
        output,
        flags=re.MULTILINE,
    )
    if match is None:
        return {}
    body = match.group("body")
    if not body.startswith("seo_site_build_canonical_cutover_ok "):
        return {}
    fields = dict(re.findall(r"([a-z][a-z_]+)=([^\s]+)", body))
    required = ("ia_pages", "published_pages", "truth_verified", "run_id")
    if any(name not in fields for name in required):
        return {}
    try:
        page_total = int(fields["ia_pages"])
        published_pages = int(fields["published_pages"])
        truth_verified = int(fields["truth_verified"])
    except ValueError:
        return {}
    workflow_id = match.group("workflow_id")
    if fields["run_id"] != workflow_id:
        return {}
    return {
        "workflow_type": "SeoSiteBuildCanonicalCutoverWorkflow",
        "workflow_id": workflow_id,
        "scenario": "site_build_full",
        "mode": "temporal_durable",
        "status": (
            "published"
            if page_total > 0 and published_pages > 0 and truth_verified > 0
            else "blocked:incomplete_canonical_result"
        ),
        "page_total": page_total,
        "published_pages": published_pages,
        "truth_verified": truth_verified,
    }


def snapshot_verified_facts() -> dict[str, dict[str, Any]]:
    """Read-only authoritative truth snapshot through the existing shell/psql boundary."""
    result = run_command(["bash", "automation/read_verified_facts_for_certification.sh"])
    if result.returncode != 0:
        raise RuntimeError("read-only verified fact snapshot failed")
    rows = json.loads(result.stdout.strip())
    if not isinstance(rows, dict):
        raise ValueError("verified fact snapshot must be a keyed JSON object")
    return rows


def snapshot_published_html() -> dict[str, str]:
    output = Path(os.environ["ALEGRIA_CERT_PUBLIC_OUTPUT_DIR"])
    if not output.is_dir():
        raise ValueError("certified public output directory is missing")
    return {
        str(path.relative_to(output)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in output.rglob("*.html")
        if path.is_file()
    }


def factual_change_proof(
    before_facts: dict[str, dict[str, Any]],
    after_facts: dict[str, dict[str, Any]],
    before_html: dict[str, str],
    after_html: dict[str, str],
) -> dict[str, Any]:
    """An added rule or changed query alone cannot prove factual correction."""
    changed_rules = sorted(
        key for key in before_facts.keys() & after_facts.keys()
        if before_facts[key].get("params") != after_facts[key].get("params")
        and before_facts[key].get("source_snapshot_hash")
            != after_facts[key].get("source_snapshot_hash")
        and after_facts[key].get("source_key")
    )
    changed_pages = sorted(
        key for key in before_html.keys() & after_html.keys()
        if before_html[key] != after_html[key]
    )
    return {
        "verified_existing_rule_changes": len(changed_rules),
        "modified_existing_html_pages": len(changed_pages),
        "modified_page_paths": changed_pages,
        "status": "PASS" if changed_rules and changed_pages else "FAIL",
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
    resume_run_id: str | None = None,
) -> dict[str, Any]:
    run_id = resume_run_id or str(uuid4())
    query_batch_key = f"r5-{scenario_id}-{run_id}"
    command = (
        existing_canonical_temporal_command(run_id)
        if resume_run_id
        else canonical_temporal_command(
            queries=queries,
            run_id=run_id,
            query_batch_key=query_batch_key,
            publish=publish,
        )
    )
    if scenario_id == "fresh_scope" and publish:
        record_first_site_session(run_id, resume=resume_run_id is not None)
    before_facts = before_html = None
    if scenario_id == "factual_change_rebuild":
        # Read both sides of the real change from the same running staging
        # database and the actual published output. Never inject a synthetic
        # changed rule into a product acceptance run.
        before_facts = snapshot_verified_facts()
        before_html = snapshot_published_html()
    proc = run_command(command)
    if scenario_id == "fresh_scope" and publish:
        receipt = first_site_session(run_id)
        write_json(receipt, {
            "kind": "first_site_canonical_temporal_session",
            "run_id": run_id,
            "workflow_id": run_id,
            "status": "RESULT_RECEIVED" if proc.returncode == 0 else "WAITING_FOR_RESULT",
            "action": "REATTACH" if resume_run_id else "START_REQUESTED",
            "updated_at": utc_now(),
        })
    factual_proof = None
    if scenario_id == "factual_change_rebuild" and proc.returncode == 0:
        factual_proof = factual_change_proof(
            before_facts,
            snapshot_verified_facts(),
            before_html,
            snapshot_published_html(),
        )
    combined = "\n".join(chunk for chunk in [proc.stdout.strip(), proc.stderr.strip()] if chunk.strip())
    result = parse_result(proc.stdout)
    phase_lines = [
        line
        for line in proc.stdout.splitlines()
        if line.startswith("SEO_RUN_PHASE ")
    ]
    valid_canonical_result = (
        bool(result)
        and result.get("status") == "published"
        and result.get("workflow_id") == run_id
        and (
            scenario_id != "factual_change_rebuild"
            or (factual_proof is not None and factual_proof["status"] == "PASS")
        )
    )
    evidence_status = "PASS" if proc.returncode == 0 and valid_canonical_result else "FAIL"
    failure_class = "none" if evidence_status == "PASS" else "certification_run_failed"
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
        "status_reason": "Canonical real-source scenario certified." if evidence_status == "PASS" else "Canonical execution or independently observed factual-change proof failed.",
        "provider_kind": provider_kind,
        "publish_requested": publish,
        "run_id": run_id,
        "workflow_id": run_id,
        "query_batch_key": query_batch_key,
        "queries": queries,
        "command": " ".join(command),
        "exit_code": proc.returncode,
        "result": result,
        "factual_change_evidence": factual_proof,
        "phase_lines": phase_lines,
        "stdout_excerpt": proc.stdout.strip()[-4000:],
        "stderr_excerpt": proc.stderr.strip()[-4000:],
        "updated_at": utc_now(),
        "gate_results": gate_results,
    }

    # Never overwrite immutable per-run evidence when scenarios rerun the same day.
    md_name = f"supersite_e2e_{scenario_id}_{run_id}.md"
    payload["report_path"] = f"docs/runs/{md_name}"
    write_markdown(RUNS_DIR / md_name, title, payload)
    return payload


def clean_runtime_preflight(*, supervised_manual: bool = False) -> dict[str, Any]:
    """Read-only launch readiness; never run fixture seeding or database restore.

    The canonical workflow itself enforces live provider, facts, graph and
    retrieval gates. This preliminary check only verifies that the already
    running services are reachable and the worker's publish policy is strict.
    """
    checks = [
        (
            "postgres_readiness",
            ["docker", "compose", "exec", "-T", "postgres",
             "pg_isready", "-U", "postgres", "-d", "alegria"],
        ),
        (
            "canonical_temporal_connectivity",
            ["docker", "compose", "exec", "-T", "temporal-worker",
             "temporal_starter", "--temporal-url", "http://temporal-server:7233",
             "ping"],
        ),
        (
            "canonical_worker_configuration",
            ["docker", "compose", "exec", "-T", "-e",
             f"ALEGRIA_SUPERVISED_MANUAL={'1' if supervised_manual else '0'}",
             "-e", f"ALEGRIA_EXPECTED_ORIGIN={os.environ['ALEGRIA_CERT_BASE_URL']}",
             "-e", f"ALEGRIA_EXPECTED_OUTPUT={os.environ['ALEGRIA_CERT_PUBLIC_OUTPUT_DIR']}",
             "-e", f"ALEGRIA_EXPECTED_ACCOUNT={EXPECTED_ACCOUNT}",
             "temporal-worker", "sh", "-ec", """
                set -eu
                test "${SEO_STATIC_BASE_URL:-}" = "$ALEGRIA_EXPECTED_ORIGIN"
                test "${SEO_STATIC_OUTPUT_DIR:-}" = "$ALEGRIA_EXPECTED_OUTPUT"
                test "${SEO_STATIC_PUBLISH_BACKEND:-}" = cloudflare_workers_static_assets
                test -n "${CLOUDFLARE_API_TOKEN:-}"
                test "${CLOUDFLARE_ACCOUNT_ID:-}" = "$ALEGRIA_EXPECTED_ACCOUNT"
                test -n "${DATAFORSEO_LOGIN:-}"
                test -n "${DATAFORSEO_PASSWORD:-}"
                test -n "${VOYAGE_API_KEY:-}"
                for flag in RETRIEVAL_CAPABILITY_REQUIRED CANONICAL_VECTOR_RETRIEVAL_REQUIRED \
                    CONTEXTUAL_RAW_CHUNK_RETRIEVAL_REQUIRED VOYAGE_RERANK_REQUIRED \
                    GRAPH_CAPABILITY_REQUIRED NEO4J_SYNC_REQUIRED GRAPH_QUERY_REQUIRED \
                    GRAPH_GDS_REQUIRED; do
                    value="$(printenv "$flag" || true)"
                    test "$value" = true
                done
                case "${SEO_LLM_PROVIDER:-}" in
                    openai) test -n "${OPENAI_API_KEY:-}" ;;
                    anthropic) test -n "${ANTHROPIC_API_KEY:-}" ;;
                    gemini) test -n "${GEMINI_API_KEY:-}${GOOGLE_API_KEY:-}" ;;
                    chat_manual)
                        test "${ALEGRIA_SUPERVISED_MANUAL:-0}" = 1
                        test "${SEO_TRUTH_LLM_PROVIDER:-}" = chat_manual ;;
                    *) exit 1 ;;
                esac
             """],
        ),
    ]
    evidence: dict[str, Any] = {}
    for name, command in checks:
        result = run_command(command)
        evidence[name] = {
            "exit_code": result.returncode,
            # Intentionally do not persist command/env/stdout/stderr: the
            # worker configuration contains account and credential context.
            "status": "PASS" if result.returncode == 0 else "FAIL",
        }
        if result.returncode != 0:
            break
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--first-site", action="store_true",
        help="certify only the first real published site; never claim full R5",
    )
    parser.add_argument(
        "--manual", action="store_true",
        help="supervised chat_manual first-site run; still requires real HITL and HTTPS",
    )
    parser.add_argument(
        "--resume-run-id", type=str,
        help="reattach to this existing canonical first-site UUID; never starts a workflow",
    )
    args = parser.parse_args()
    if args.resume_run_id:
        try:
            from uuid import UUID
            if str(UUID(args.resume_run_id)) != args.resume_run_id:
                raise ValueError("noncanonical UUID")
        except ValueError:
            parser.error("--resume-run-id requires a canonical UUID")
        if not args.first_site or not args.manual or args.dry_run:
            parser.error("--resume-run-id requires --first-site --manual, without --dry-run")
    if args.manual and not args.first_site:
        parser.error("--manual is permitted only with --first-site")
    if args.manual and (
        os.environ.get("SEO_LLM_PROVIDER") != "chat_manual"
        or os.environ.get("SEO_TRUTH_LLM_PROVIDER") != "chat_manual"
    ):
        parser.error("--manual requires both editorial and truth providers to be chat_manual")
    report_path = (
        RUNS_DIR / f"first_site_certification_{uuid4()}.json"
        if args.first_site else MANIFEST
    )

    provider_live, provider_kind = classify_provider()

    # A single configuration preflight is used for preview AND execution:
    # dry-run cannot report "ready" under weaker requirements.
    required = certification_requirements(provider_live, supervised_manual=args.manual)
    missing = [key for key, present in required.items() if not present]
    if args.dry_run:
        print("First-site configuration preview" if args.first_site else "R5 configuration preview")
        print("- database_connectivity=NOT_TESTED (checked in running PostgreSQL at launch)")
        print(f"- provider_kind={provider_kind}")
        print(f"- configuration={'BLOCKED' if missing else 'READY'}")
        print(f"- missing={','.join(missing) if missing else 'none'}")
        if provider_kind == "chat_manual":
            print("- manual_mode=requires llm-next/llm-submit and separate CMS approval; "
                  + ("supervised first-site certification" if args.manual else
                     "not eligible for unattended live certification"))
        print("- gate: real canonical Temporal workflow, no synthetic seeding or restore")
        print("- scenarios: fresh_scope" if args.first_site else
              "- scenarios: fresh_scope, scope_expansion, factual_change_rebuild")
        return 2 if missing else 0

    # The same readiness gate blocks live provider calls before any mutations.
    if missing:
        credentials_missing = any(
            key in {"DATAFORSEO_LOGIN", "DATAFORSEO_PASSWORD", "VOYAGE_API_KEY", "production_llm_provider"}
            for key in missing
        )
        status = "PENDING_CREDENTIALS" if credentials_missing else "PENDING_CONFIGURATION"
        manifest = {
            "artifact_id": "r5_supersite_certification_manifest",
            "step": "R5.Step4",
            "evidence_status": status,
            "failure_class": "credential_issue" if credentials_missing else "configuration_issue",
            "status_reason": "Live certification requires credentials, explicit publication, a real HTTPS origin, and public output configuration.",
            "provider_kind": provider_kind,
            "required": required,
            "missing": missing,
            "updated_at": utc_now(),
        }
        write_json(report_path, manifest)
        print(f"E2E_SUPERSITE_CERTIFICATION: {status}")
        return 2

    # Previous mutating smoke gates seeded synthetic facts, auto-approved
    # pages and restored PostgreSQL schemas. Product certification MUST use
    # only read-only readiness checks and genuine external editorial approval.
    gate_results = clean_runtime_preflight(supervised_manual=args.manual)
    if not gate_results or any(
        check["status"] != "PASS" for check in gate_results.values()
    ):
        manifest = {
            "artifact_id": "r5_supersite_certification_manifest",
            "step": "R5.Step4",
            "evidence_status": "FAIL",
            "failure_class": "runtime_preflight_failure",
            "status_reason": (
                "Read-only service/configuration preflight failed; no workflow was started."
            ),
            "provider_kind": provider_kind,
            "gate_results": gate_results,
            "updated_at": utc_now(),
        }
        write_json(report_path, manifest)
        print("E2E_SUPERSITE_CERTIFICATION: FAILED")
        return 1

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

    if args.first_site:
        scenarios = scenarios[:1]

    manifest = {
        "artifact_id": (
            "first_site_certification_manifest"
            if args.first_site else "r5_supersite_certification_manifest"
        ),
        "step": "FIRST_SITE" if args.first_site else "R5.Step4",
        "certification_scope": "first_site_only" if args.first_site else "full_r5",
        "supervised_manual": args.manual,
        "evidence_status": "PASS",
        "failure_class": "none",
        "status_reason": (
            "First canonical real-source site published and independently HTTP verified; "
            "does not certify full R5."
            if args.first_site else "Three live certification scenarios completed."
        ),
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
            resume_run_id=args.resume_run_id,
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
                "report_path": payload["report_path"],
            }
        )
        if payload["evidence_status"] != "PASS":
            manifest["evidence_status"] = "FAIL"
            manifest["failure_class"] = payload["failure_class"]
            manifest["status_reason"] = f"Certification scenario `{scenario_id}` failed."
            break

    if args.first_site and manifest["evidence_status"] == "PASS":
        # This is a separate product milestone, NOT a shortcut around the
        # three-scenario R5 checker. Verify actual deployable HTML and
        # bytes independently served by the configured HTTPS origin.
        output_dir = os.environ["ALEGRIA_CERT_PUBLIC_OUTPUT_DIR"]
        # Release symlinks use container-absolute paths. Running validators
        # on the host could inspect a different directory or a broken link.
        worker_prefix = ["docker", "compose", "exec", "-T", "temporal-worker", "python3"]
        current_run = manifest["scenarios"][0]["run_id"]
        public_checks = (
            ("run_release_provenance", worker_prefix + [
                "/app/automation/check_first_site_release.py",
                "--run-id", current_run,
            ]),
            ("static_output", worker_prefix + [
                "/app/automation/check_public_static_output.py", output_dir,
            ]),
            ("independent_https_readback", worker_prefix + [
                "/app/automation/check_public_http_readback.py", output_dir,
            ]),
        )
        manifest["public_acceptance"] = {}
        for name, command in public_checks:
            checked = run_command(command)
            manifest["public_acceptance"][name] = (
                "PASS" if checked.returncode == 0 else "FAIL"
            )
            if checked.returncode != 0:
                manifest["evidence_status"] = "FAIL"
                manifest["failure_class"] = "public_acceptance_failure"
                manifest["status_reason"] = (
                    f"First site {name} failed; no independent public proof."
                )
                break

    write_json(report_path, manifest)
    print(f"E2E_SUPERSITE_CERTIFICATION: {manifest['evidence_status']}")
    print(f"E2E_SUPERSITE_REPORT: {report_path.relative_to(ROOT)}")
    return 0 if manifest["evidence_status"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
