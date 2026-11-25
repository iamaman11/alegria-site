#!/usr/bin/env python3
"""Read-only, fail-closed V1/V2 Temporal history transition evidence.

This tool intentionally has no mutating Temporal, PostgreSQL, deployment, or
GitHub command. A successful drain snapshot is *not* permission to cut over:
starter quiescence, server-side replay, worker routing, rollback availability,
and deployment authorization require separate live operator evidence.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_VERSION = 1
# Deliberately explicit full inventory, including retired Workflow Types that
# can no longer be discovered from today's register_workflow() declarations.
# The source-conformance test detects additions to every registered legacy,
# PG and provider-free Worker. Unknown *closed* types also block cutover via
# count(all) != sum(count(each known type)).
WORKFLOW_TYPES = (
    "SeoSiteBuildWorkflow",
    "ContentGenerationWorkflow",
    "SeoSiteBuildCanonicalCutoverWorkflow",
    "SeoSiteBuildComposableV2Workflow",
    "ExpertExtractionWorkflow",
    "ExpertProjectionWorkflow",
    "ExpertSemanticSliceWorkflow",
    "ExpertDecomposedExtractionWorkflow",
    "ProjectionReconcileWorkflow",
    "FreshnessCheckWorkflow",
    "TestHitlWorkflow",
    "AlegriaPureOperationDagV1Workflow",
    "AlegriaCheckedEffectDagV1Workflow",
    "AlegriaAdmittedEffectDagV2Workflow",
    "AlegriaAdmittedEffectDagV3Workflow",
    "PgTruthReviewReconcileV3Workflow",
    "PgTruthReviewReconcileV4Workflow",
)
# Historical name retained for compatibility with the local safety tests.
LEGACY_TYPES = WORKFLOW_TYPES
WORKFLOW_QUERY_RUNNING = 'ExecutionStatus = "Running"'
SHA_RE = re.compile(r"[a-f0-9]{40}\Z")
ROLE_SQL = """
SELECT json_build_object(
 'global_visibility', (SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = current_user),
 'execution_runs_unowned', (SELECT count(*) FROM pipeline.execution_runs WHERE tenant_id IS NULL),
 'raw_pages_unowned', (SELECT count(*) FROM raw.pages WHERE tenant_id IS NULL),
 'raw_sections_unowned', (SELECT count(*) FROM raw.sections WHERE tenant_id IS NULL),
 'candidates_unowned', (SELECT count(*) FROM extracted.rule_candidates WHERE tenant_id IS NULL),
 'hitl_unowned', (SELECT count(*) FROM site.seo_hitl_tasks WHERE tenant_id IS NULL),
 'receipts_unowned', (SELECT count(*) FROM pipeline.pg_effect_v3_receipts WHERE tenant_id IS NULL),
 'unfinished_runs', (SELECT count(*) FROM pipeline.execution_runs WHERE status NOT IN ('done','failed')),
 'unregistered_run_contexts', (
   SELECT count(*) FROM pipeline.execution_runs r
   LEFT JOIN pipeline.tenant_context_ownership o ON o.context_key = r.context_key
   WHERE o.context_key IS NULL OR r.tenant_id IS DISTINCT FROM o.tenant_id
 ),
 'unregistered_candidate_contexts', (
   SELECT count(*) FROM extracted.rule_candidates c
   LEFT JOIN pipeline.tenant_context_ownership o ON o.context_key = c.context_key
   WHERE o.context_key IS NULL OR c.tenant_id IS DISTINCT FROM o.tenant_id
 ),
 'admission_owner_mismatches', (
   SELECT count(*) FROM pipeline.pg_effect_v3_admissions a
   JOIN pipeline.execution_runs r ON r.run_id = a.run_id
   LEFT JOIN pipeline.tenant_context_ownership o ON o.context_key = a.context_key
   WHERE o.context_key IS NULL
      OR a.tenant_id IS DISTINCT FROM o.tenant_id
      OR a.tenant_id IS DISTINCT FROM r.tenant_id
      OR a.context_key IS DISTINCT FROM r.context_key
 ),
 'receipt_owner_mismatches', (
   SELECT count(*) FROM pipeline.pg_effect_v3_receipts receipt
   JOIN pipeline.pg_effect_v3_admissions a ON a.admission_id = receipt.admission_id
   JOIN pipeline.execution_runs r ON r.run_id = receipt.run_id
   LEFT JOIN pipeline.tenant_context_ownership o ON o.context_key = receipt.context_key
   WHERE o.context_key IS NULL
      OR receipt.tenant_id IS DISTINCT FROM o.tenant_id
      OR receipt.tenant_id IS DISTINCT FROM a.tenant_id
      OR receipt.tenant_id IS DISTINCT FROM r.tenant_id
      OR receipt.context_key IS DISTINCT FROM r.context_key
 ),
 'section_page_mismatches', (
   SELECT count(*) FROM raw.sections s JOIN raw.pages p ON p.id = s.page_id
   WHERE s.tenant_id IS DISTINCT FROM p.tenant_id
 ),
 'candidate_source_mismatches', (
   SELECT count(*) FROM extracted.rule_candidates c
   JOIN raw.sections rs ON rs.id = c.raw_section_id
   JOIN raw.pages rp ON rp.id = rs.page_id
   JOIN raw.sections es ON es.id = c.evidence_section_id
   JOIN raw.pages ep ON ep.id = es.page_id
   WHERE c.tenant_id IS DISTINCT FROM rs.tenant_id
      OR c.tenant_id IS DISTINCT FROM rp.tenant_id
      OR c.tenant_id IS DISTINCT FROM es.tenant_id
      OR c.tenant_id IS DISTINCT FROM ep.tenant_id
 ),
 'hitl_candidate_mismatches', (
   SELECT count(*) FROM site.seo_hitl_tasks h
   LEFT JOIN extracted.rule_candidates c ON c.rule_candidate_id = h.blocking_execution_key
   WHERE h.task_type = 'truth_candidate_review'
     AND (c.rule_candidate_id IS NULL OR h.tenant_id IS DISTINCT FROM c.tenant_id)
 )
)::text;
"""
COUNT_FIELDS = (
    "execution_runs_unowned", "raw_pages_unowned", "raw_sections_unowned",
    "candidates_unowned", "hitl_unowned", "receipts_unowned", "unfinished_runs",
    "unregistered_run_contexts", "unregistered_candidate_contexts",
    "admission_owner_mismatches", "receipt_owner_mismatches",
    "section_page_mismatches", "candidate_source_mismatches",
    "hitl_candidate_mismatches",
)


class EvidenceError(RuntimeError):
    """A missing, ambiguous or untrusted observation fails closed."""


def now_utc() -> datetime:
    return datetime.now(timezone.utc)


def timestamp(value: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except (ValueError, AttributeError) as error:
        raise EvidenceError("invalid UTC evidence timestamp") from error
    if parsed.tzinfo is None or parsed.utcoffset() != timedelta(0):
        raise EvidenceError("evidence timestamp must specify UTC")
    return parsed


def nonnegative(value: Any, name: str) -> int:
    if type(value) is not int or value < 0:
        raise EvidenceError(f"{name} must be a nonnegative integer")
    return value


def run_command(argv: list[str], *, env: dict[str, str] | None = None,
                allow_empty: bool = False) -> str:
    try:
        process = subprocess.run(
            argv, cwd=ROOT, env=env, text=True, capture_output=True,
            encoding="utf-8", timeout=45, check=False,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired, UnicodeError) as error:
        raise EvidenceError(f"required observation tool unavailable: {argv[0]}") from error
    if process.returncode:
        # stderr may include an authenticated address, service URL or credentials.
        raise EvidenceError(f"{argv[0]} observation failed (exit={process.returncode}); no result trusted")
    if not process.stdout.strip() and not allow_empty:
        raise EvidenceError(f"{argv[0]} returned empty evidence")
    return process.stdout.strip()


def exact_source(expected_sha: str) -> dict[str, str]:
    if not SHA_RE.fullmatch(expected_sha):
        raise EvidenceError("exact source requires a full 40-character lowercase Git SHA")
    actual = run_command(["git", "rev-parse", "HEAD"])
    if actual != expected_sha:
        raise EvidenceError("checked-out source differs from the requested exact SHA")
    if run_command(["git", "status", "--porcelain", "--untracked-files=no"], allow_empty=True):
        raise EvidenceError("tracked worktree is dirty; exact-source evidence unavailable")
    tree = run_command(["git", "rev-parse", "HEAD^{tree}"])
    if not SHA_RE.fullmatch(tree):
        raise EvidenceError("Git tree identity is not a valid SHA")
    return {"sha": actual, "tree": tree}


def temporal_count(raw: str) -> int:
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as error:
        raise EvidenceError("Temporal count did not return JSON") from error
    if (not isinstance(document, dict) or "count" not in document
            or set(document) - {"count", "groups"}
            or document.get("groups", []) not in (None, [])):
        raise EvidenceError("unexpected Temporal count JSON schema")
    count = document["count"]
    if isinstance(count, str) and re.fullmatch(r"0|[1-9][0-9]*", count):
        count = int(count)
    return nonnegative(count, "Temporal count")


def temporal_inventory(address: str, namespace: str) -> dict[str, Any]:
    if not address or "@" in address or "://" in address or not namespace:
        raise EvidenceError("Temporal address must be host:port; namespace must be explicit")

    def count(query: str | None) -> int:
        # Read-only official Temporal CLI count; never list only one page then
        # mistake that page for a complete history inventory.
        argv = [
            "temporal", "workflow", "count", "--address", address,
            "--namespace", namespace, "--output", "json",
        ]
        if query is not None:
            argv.extend(("--query", query))
        raw = run_command(argv)
        return temporal_count(raw)

    total_before = count(None)
    before = count(WORKFLOW_QUERY_RUNNING)
    historical = {}
    for name in LEGACY_TYPES:
        base = f'WorkflowType = "{name}"'
        historical[name] = {
            "total": count(base),
            "open": count(f"{base} AND {WORKFLOW_QUERY_RUNNING}"),
        }
        if historical[name]["open"] > historical[name]["total"]:
            raise EvidenceError("Temporal open exceeds total; inconsistent visibility")
    total_after = count(None)
    after = count(WORKFLOW_QUERY_RUNNING)
    if before != after or total_before != total_after:
        raise EvidenceError("Temporal running or total count changed during inventory")
    recognized = sum(counts["total"] for counts in historical.values())
    if recognized != total_after:
        raise EvidenceError("Temporal namespace contains unrecognized historical Workflow Types")
    return {
        "source": "live_temporal_server_visibility", "address": address,
        "namespace": namespace, "open_all_before": before,
        "open_all_after": after, "total_all_before": total_before,
        "total_all_after": total_after, "legacy_workflow_types": historical,
    }


def pg_inventory(dsn_env: str) -> dict[str, Any]:
    if not re.fullmatch(r"[A-Z][A-Z0-9_]{4,80}", dsn_env):
        raise EvidenceError("database environment-variable name is invalid")
    dsn = os.environ.get(dsn_env)
    if not dsn:
        raise EvidenceError(f"required audit DSN environment variable is missing: {dsn_env}")
    environment = dict(os.environ)
    # No password/URI in argv, error text, artifacts or logs.
    environment["PGDATABASE"] = dsn
    environment["PGCONNECT_TIMEOUT"] = "5"
    raw = run_command([
        "psql", "-X", "--no-psqlrc", "-A", "-t", "-q",
        "-v", "ON_ERROR_STOP=1", "-c", ROLE_SQL,
    ], env=environment)
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as error:
        raise EvidenceError("PostgreSQL owner inventory returned malformed JSON") from error
    if not isinstance(document, dict) or set(document) != {*COUNT_FIELDS, "global_visibility"}:
        raise EvidenceError("PostgreSQL inventory has missing or unexpected fields")
    if document["global_visibility"] is not True:
        raise EvidenceError("audit PostgreSQL role cannot prove global visibility under RLS")
    return {"source": "live_postgresql_all_tenant_read_only", "global_visibility": True, **{
        key: nonnegative(document[key], key) for key in COUNT_FIELDS
    }}


def blockers(temporal: dict[str, Any], postgres: dict[str, Any]) -> list[str]:
    reasons = []
    if temporal["source"] != "live_temporal_server_visibility":
        raise EvidenceError("Temporal count provenance missing")
    if postgres["source"] != "live_postgresql_all_tenant_read_only" or postgres["global_visibility"] is not True:
        raise EvidenceError("PostgreSQL global read-only visibility not proven")
    if temporal["open_all_before"] != temporal["open_all_after"]:
        raise EvidenceError("Temporal running count unstable")
    if temporal["total_all_before"] != temporal["total_all_after"]:
        raise EvidenceError("Temporal all-history count unstable")
    if set(temporal["legacy_workflow_types"]) != set(LEGACY_TYPES):
        raise EvidenceError("incomplete historical Workflow type inventory")
    nonnegative(temporal["open_all_before"], "Temporal open count")
    nonnegative(temporal["total_all_before"], "Temporal total count")
    recognized_total = 0
    for name, counts in temporal["legacy_workflow_types"].items():
        if not isinstance(counts, dict) or set(counts) != {"open", "total"}:
            raise EvidenceError(f"malformed history counts: {name}")
        nonnegative(counts["total"], f"{name}.total")
        nonnegative(counts["open"], f"{name}.open")
        if counts["open"] > counts["total"]:
            raise EvidenceError(f"{name}: open count exceeds total")
        recognized_total += counts["total"]
    if recognized_total != temporal["total_all_after"]:
        raise EvidenceError("unrecognized closed or open Workflow Types remain")
    for name in COUNT_FIELDS:
        nonnegative(postgres[name], name)
    if temporal["open_all_after"]:
        reasons.append("Temporal has open Workflows in selected namespace")
    for name, counts in temporal["legacy_workflow_types"].items():
        if counts["open"]:
            reasons.append(f"legacy Workflow still open: {name}")
        if counts["total"]:
            reasons.append(f"historical SDK replay remains unproved: {name}")
    if postgres["unfinished_runs"]:
        reasons.append("PostgreSQL execution_runs not terminal")
    for name in COUNT_FIELDS:
        if name == "unfinished_runs":
            continue
        if postgres[name]:
            reasons.append(f"unresolved tenant ownership/source relationship: {name}")
    return reasons


def capture(*, environment: str, address: str, namespace: str,
            dsn_env: str, source_sha: str) -> dict[str, Any]:
    if environment not in {"staging", "production"}:
        raise EvidenceError("capture must name staging or production explicitly")
    source = exact_source(source_sha)
    temporal = temporal_inventory(address, namespace)
    postgres = pg_inventory(dsn_env)
    reasons = blockers(temporal, postgres)
    # Even a zero-open count is only an instantaneous read. It cannot prove
    # no starter will create a legacy execution one microsecond later.
    return {
        "schema_version": SCHEMA_VERSION, "captured_at": now_utc().isoformat(),
        "environment": environment, "source": source,
        "temporal": temporal, "postgres": postgres,
        "status": "BLOCKED" if reasons else "DRAIN_CANDIDATE_ONLY",
        "blockers": reasons,
        "cutover_authorized": False,
        "unverified_external_gates": [
            "legacy_starter_admission_fence",
            "official_sdk_history_replay_and_worker_version_routing",
            "independent_Temporal_server_history_attestation",
            "operator_approved_rollback_fleet_and_recovery_drill",
            "signed_exact_SHA_runtime_and_production_mTLS",
        ],
    }


def check_pair(earlier: dict[str, Any], later: dict[str, Any], *,
               clock: datetime | None = None) -> dict[str, Any]:
    """Compare two independently captured live snapshots after a drain window.

    This is a *diagnostic precondition*, never a switch/route command.
    Files may be untrusted; no boolean in them grants rollout authority.
    """
    clock = clock or now_utc()
    errors = []
    for name, document in (("earlier", earlier), ("later", later)):
        if not isinstance(document, dict) or document.get("schema_version") != SCHEMA_VERSION:
            errors.append(f"{name}: unsupported evidence schema")
            continue
        if document.get("cutover_authorized") is not False:
            errors.append(f"{name}: forbidden cutover_authorized marker")
        if document.get("status") != "DRAIN_CANDIDATE_ONLY" or document.get("blockers") != []:
            errors.append(f"{name}: drain inventory did not pass all read-only criteria")
        for top in ("environment", "source", "temporal", "postgres"):
            if top not in document:
                errors.append(f"{name}: missing {top}")
    if errors:
        return {"status": "BLOCKED", "errors": errors, "cutover_authorized": False}
    for field in ("environment", "source"):
        if earlier[field] != later[field]:
            errors.append(f"environment/source changed across drain window: {field}")
    for field in ("namespace", "address"):
        if earlier["temporal"].get(field) != later["temporal"].get(field):
            errors.append(f"Temporal server changed across drain window: {field}")
    if earlier["temporal"].get("legacy_workflow_types") != later["temporal"].get("legacy_workflow_types"):
        errors.append("historical workflow counts drifted across drain window")
    for name, record in (("earlier", earlier), ("later", later)):
        try:
            reasons = blockers(record["temporal"], record["postgres"])
            if reasons:
                errors.extend(f"{name}: {reason}" for reason in reasons)
        except (KeyError, TypeError, ValueError, EvidenceError):
            errors.append(f"{name}: malformed inventory")
    try:
        first = timestamp(earlier["captured_at"])
        second = timestamp(later["captured_at"])
        if not timedelta(seconds=30) <= second - first <= timedelta(minutes=30):
            errors.append("drain snapshots must be 30s..30min apart")
        if clock - second > timedelta(minutes=10) or second > clock + timedelta(seconds=15):
            errors.append("last drain snapshot too old or in the future")
    except (EvidenceError, KeyError, TypeError):
        errors.append("invalid drain snapshot times")
    return {
        "status": "BLOCKED" if errors else "DRAIN_PRECONDITIONS_CAPTURED",
        "errors": errors, "cutover_authorized": False,
        "still_required": [
            "server_verified_legacy_starter_freeze_and_no_duplicate_pollers",
            "official_sdk_replay_of_all_historical_workflow_types",
            "verified_rollback_fleet_and_routing_reversal",
            "production_operator_authorization_and_mTLS",
        ],
    }


def atomic_json(path: Path, evidence: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, prefix=f".{path.name}.",
        suffix=".tmp", delete=False,
    ) as temp:
        json.dump(evidence, temp, indent=2, sort_keys=True)
        temp.write("\n")
        temporary = Path(temp.name)
    temporary.replace(path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    live = sub.add_parser("capture", help="read live Temporal + PG; never mutate either")
    live.add_argument("--environment", required=True, choices=("staging", "production"))
    live.add_argument("--temporal-address", required=True)
    live.add_argument("--temporal-namespace", required=True)
    live.add_argument("--dsn-env", default="ALEGRIA_HISTORY_AUDIT_DSN")
    live.add_argument("--source-sha", required=True)
    live.add_argument("--output", type=Path, required=True)
    compare = sub.add_parser("compare", help="validate two drain snapshots; no cutover action")
    compare.add_argument("--earlier", type=Path, required=True)
    compare.add_argument("--later", type=Path, required=True)
    compare.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "capture":
            outcome = capture(
                environment=args.environment, address=args.temporal_address,
                namespace=args.temporal_namespace, dsn_env=args.dsn_env,
                source_sha=args.source_sha,
            )
        else:
            outcome = check_pair(
                json.loads(args.earlier.read_text(encoding="utf-8")),
                json.loads(args.later.read_text(encoding="utf-8")),
            )
        atomic_json(args.output, outcome)
        print(f"TEMPORAL_HISTORY_TRANSITION={outcome['status']} cutover_authorized=false")
        return 0 if outcome["status"] in {"DRAIN_CANDIDATE_ONLY", "DRAIN_PRECONDITIONS_CAPTURED"} else 1
    except (EvidenceError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        # The error reason must never include a DSN, URI query, token or stderr.
        print(f"TEMPORAL_HISTORY_TRANSITION=BLOCKED reason={type(error).__name__}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
