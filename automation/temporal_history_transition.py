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
import hashlib
import json
import os
from pathlib import Path
import re
import stat
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
    "AlegriaQdrantProjectionDagV1Workflow",
    "QdrantKeywordProjectionV1Workflow",
)
# Historical name retained for compatibility with the local safety tests.
LEGACY_TYPES = WORKFLOW_TYPES
WORKFLOW_QUERY_RUNNING = 'ExecutionStatus = "Running"'
SHA_RE = re.compile(r"[a-f0-9]{40}\Z")
ROLE_SQL = """
WITH quarantine AS MATERIALIZED (
    SELECT * FROM pipeline.tenant_historical_quarantine_v1()
)
SELECT json_build_object(
 'global_visibility', (SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = current_user),
 'historical_quarantine_unattributed', (SELECT coalesce(sum(unattributed), 0) FROM quarantine),
 'historical_quarantine_inconsistent', (SELECT coalesce(sum(inconsistent), 0) FROM quarantine),
 'quarantine_by_relation', (
   SELECT json_object_agg(relation_name, json_build_object(
       'unattributed', unattributed, 'inconsistent', inconsistent)) FROM quarantine
 ),
 'execution_runs_unowned', (SELECT count(*) FROM pipeline.execution_runs WHERE tenant_id IS NULL),
 'raw_pages_unowned', (SELECT count(*) FROM raw.pages WHERE tenant_id IS NULL),
 'raw_sections_unowned', (SELECT count(*) FROM raw.sections WHERE tenant_id IS NULL),
 'candidates_unowned', (SELECT count(*) FROM extracted.rule_candidates WHERE tenant_id IS NULL),
 'hitl_unowned', (SELECT count(*) FROM site.seo_hitl_tasks WHERE tenant_id IS NULL),
 'admissions_unowned', (SELECT count(*) FROM pipeline.pg_effect_v3_admissions WHERE tenant_id IS NULL),
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
   LEFT JOIN pipeline.execution_runs r ON r.run_id = a.run_id
   LEFT JOIN pipeline.tenant_context_ownership o ON o.context_key = a.context_key
   WHERE r.run_id IS NULL OR o.context_key IS NULL
      OR a.tenant_id IS DISTINCT FROM o.tenant_id
      OR a.tenant_id IS DISTINCT FROM r.tenant_id
      OR a.context_key IS DISTINCT FROM r.context_key
 ),
 'receipt_owner_mismatches', (
   SELECT count(*) FROM pipeline.pg_effect_v3_receipts receipt
   LEFT JOIN pipeline.pg_effect_v3_admissions a ON a.admission_id = receipt.admission_id
   LEFT JOIN pipeline.execution_runs r ON r.run_id = receipt.run_id
   LEFT JOIN pipeline.tenant_context_ownership o ON o.context_key = receipt.context_key
   WHERE a.admission_id IS NULL OR r.run_id IS NULL OR o.context_key IS NULL
      OR receipt.tenant_id IS DISTINCT FROM o.tenant_id
      OR receipt.tenant_id IS DISTINCT FROM a.tenant_id
      OR receipt.tenant_id IS DISTINCT FROM r.tenant_id
      OR receipt.run_id IS DISTINCT FROM a.run_id
      OR receipt.context_key IS DISTINCT FROM a.context_key
      OR receipt.context_key IS DISTINCT FROM r.context_key
 ),
 'section_page_mismatches', (
   SELECT count(*) FROM raw.sections s LEFT JOIN raw.pages p ON p.id = s.page_id
   WHERE p.id IS NULL OR s.tenant_id IS DISTINCT FROM p.tenant_id
 ),
 'candidate_source_mismatches', (
   SELECT count(*) FROM extracted.rule_candidates c
   LEFT JOIN raw.sections rs ON rs.id = c.raw_section_id
   LEFT JOIN raw.pages rp ON rp.id = rs.page_id
   LEFT JOIN raw.sections es ON es.id = c.evidence_section_id
   LEFT JOIN raw.pages ep ON ep.id = es.page_id
   WHERE rs.id IS NULL OR rp.id IS NULL OR es.id IS NULL OR ep.id IS NULL
      OR c.tenant_id IS DISTINCT FROM rs.tenant_id
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
    "historical_quarantine_unattributed", "historical_quarantine_inconsistent",
    "execution_runs_unowned", "raw_pages_unowned", "raw_sections_unowned",
    "candidates_unowned", "hitl_unowned", "admissions_unowned", "receipts_unowned", "unfinished_runs",
    "unregistered_run_contexts", "unregistered_candidate_contexts",
    "admission_owner_mismatches", "receipt_owner_mismatches",
    "section_page_mismatches", "candidate_source_mismatches",
    "hitl_candidate_mismatches",
)
QUARANTINE_RELATIONS = (
    "raw.pages", "raw.sections", "extracted.rule_candidates",
    "pipeline.execution_runs", "site.seo_hitl_tasks",
    "pipeline.pg_effect_v3_admissions", "pipeline.pg_effect_v3_receipts",
    "pipeline.qdrant_projection_v1_admissions",
    "pipeline.qdrant_projection_v1_attempts",
    "pipeline.qdrant_projection_v1_receipts",
)

# A route exists only if this exact native binary registers the Workflow in
# its offline Temporal SDK replayer. The other historical types remain a hard
# NO-GO until an owner supplies its real replay implementation. A Workflow
# Type alone is not a queue identity: pure V1 exists in two Worker generations.
REPLAY_ROUTES: dict[tuple[str, str], tuple[str, str, str]] = {
    ("SeoSiteBuildCanonicalCutoverWorkflow", "alegria-pipeline"):
        ("legacy", "--replay-history", "CI_TEMPORAL_HISTORY_REPLAY=PASS"),
    ("SeoSiteBuildComposableV2Workflow", "alegria-pipeline"):
        ("legacy", "--replay-history", "CI_TEMPORAL_HISTORY_REPLAY=PASS"),
    ("AlegriaPureOperationDagV1Workflow", "alegria-pipeline"):
        ("legacy", "--replay-history", "CI_TEMPORAL_HISTORY_REPLAY=PASS"),
    ("AlegriaPureOperationDagV1Workflow", "alegria-pure-dag-v3"):
        ("orchestrator", "--replay-history", "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
    ("AlegriaCheckedEffectDagV1Workflow", "alegria-pure-dag-v3"):
        ("orchestrator", "--replay-history", "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
    ("AlegriaAdmittedEffectDagV2Workflow", "alegria-effect-dag-v4"):
        ("orchestrator", "--replay-effect-history", "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
    ("AlegriaAdmittedEffectDagV3Workflow", "alegria-effect-dag-v5"):
        ("orchestrator", "--replay-v5-effect-history", "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
    ("PgTruthReviewReconcileV3Workflow", "alegria-pg-command-v3"):
        ("pg", "--replay-history", "CI_TEMPORAL_PG_EFFECT_REPLAY=PASS"),
    ("PgTruthReviewReconcileV4Workflow", "alegria-pg-command-v3"):
        ("pg", "--replay-history", "CI_TEMPORAL_PG_EFFECT_REPLAY=PASS"),
    ("AlegriaQdrantProjectionDagV1Workflow", "alegria-qdrant-dag-v1"):
        ("orchestrator", "--replay-qdrant-effect-history", "CI_TEMPORAL_ORCHESTRATOR_REPLAY=PASS"),
    ("QdrantKeywordProjectionV1Workflow", "alegria-qdrant-command-v1"):
        ("qdrant", "--replay-history", "CI_TEMPORAL_QDRANT_EFFECT_REPLAY=PASS"),
}
MAX_HISTORY_BYTES = 16 * 1024 * 1024
MAX_REPLAYER_BYTES = 512 * 1024 * 1024
HISTORY_SHA_RE = re.compile(r"[a-f0-9]{64}\Z")
ROUTING_IDENTIFIER_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z")
# Native replay must not inherit live PG, Temporal, provider, signing, or
# deployment credentials from the operator's census environment.
REPLAY_ENV_ALLOW = ("PATH", "LD_LIBRARY_PATH", "LANG", "LC_ALL", "TZ", "RUST_BACKTRACE", "TMPDIR")


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
    if not isinstance(document, dict) or set(document) != {
        *COUNT_FIELDS, "global_visibility", "quarantine_by_relation",
    }:
        raise EvidenceError("PostgreSQL inventory has missing or unexpected fields")
    if document["global_visibility"] is not True:
        raise EvidenceError("audit PostgreSQL role cannot prove global visibility under RLS")
    evidence = {"source": "live_postgresql_all_tenant_read_only", "global_visibility": True, **{
        key: nonnegative(document[key], key) for key in COUNT_FIELDS
    }, "quarantine_by_relation": document["quarantine_by_relation"]}
    validate_quarantine(evidence)
    return evidence


def validate_quarantine(postgres: dict[str, Any]) -> None:
    by_relation = postgres.get("quarantine_by_relation")
    if not isinstance(by_relation, dict) or set(by_relation) != set(QUARANTINE_RELATIONS):
        raise EvidenceError("historical tenant quarantine lacks all ten authoritative relations")
    for label in ("unattributed", "inconsistent"):
        summed = 0
        for relation, counts in by_relation.items():
            if not isinstance(counts, dict) or set(counts) != {"unattributed", "inconsistent"}:
                raise EvidenceError(f"malformed historical tenant quarantine for {relation}")
            summed += nonnegative(counts[label], f"{relation}.{label}")
        if summed != nonnegative(postgres[f"historical_quarantine_{label}"], label):
            raise EvidenceError("historical quarantine sum differs from per-relation census")


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
    validate_quarantine(postgres)
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


def history_type_and_queue(raw: bytes) -> tuple[str, str]:
    """Read the server-exported WorkflowExecutionStarted metadata, not labels.

    This validates the outer Temporal JSON envelope before passing the exact
    bytes to the SDK; a plausible manifest alone is never enough to choose a
    replay binary. Export variants with unknown shapes fail closed.
    """
    try:
        document = json.loads(raw)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise EvidenceError("Temporal History export is not JSON") from error
    if not isinstance(document, dict):
        raise EvidenceError("Temporal History must be a JSON object")
    if "history" in document and "events" in document:
        raise EvidenceError("ambiguous Temporal History root and envelope")
    envelope = document.get("history", document)
    if not isinstance(envelope, dict) or not isinstance(envelope.get("events"), list):
        raise EvidenceError("Temporal History has no complete events array")
    events = envelope["events"]
    if not events:
        raise EvidenceError("Temporal History is empty")
    first = events[0]
    if not isinstance(first, dict):
        raise EvidenceError("Temporal History first event is invalid")
    if "eventType" in first and "event_type" in first:
        raise EvidenceError("ambiguous Temporal History event Type aliases")
    event_type = first.get("eventType", first.get("event_type"))
    if type(event_type) is int and event_type == 1:
        pass  # SDK serde protobuf's EventType::WorkflowExecutionStarted value.
    elif event_type not in (
        "WorkflowExecutionStarted", "EVENT_TYPE_WORKFLOW_EXECUTION_STARTED",
    ):
        raise EvidenceError("Temporal History first event is not WorkflowExecutionStarted")
    # Native Rust WorkflowHistory::to_json() serializes HistoryEvent.attributes
    # as a nested protobuf oneof. Temporal CLI/proto JSON can place the typed
    # attributes on the event. Reject conflicting dual layouts.
    nested = first.get("attributes")
    flat = first.get("workflowExecutionStartedEventAttributes")
    if nested is not None and flat is not None:
        raise EvidenceError("ambiguous Temporal History started-event attributes")
    if nested is not None:
        if not isinstance(nested, dict):
            raise EvidenceError("Temporal History attributes oneof is invalid")
        oneof = [key for key in (
            "WorkflowExecutionStartedEventAttributes",
            "workflowExecutionStartedEventAttributes",
            "workflow_execution_started_event_attributes",
        ) if key in nested]
        if len(oneof) != 1 or len(nested) != 1:
            raise EvidenceError("Temporal History started-event oneof is ambiguous")
        attributes = nested[oneof[0]]
    else:
        attributes = flat
    if not isinstance(attributes, dict):
        raise EvidenceError("Temporal History has no started-event attributes")
    if (("workflowType" in attributes and "workflow_type" in attributes)
            or ("taskQueue" in attributes and "task_queue" in attributes)):
        raise EvidenceError("ambiguous Temporal History Type/Queue field aliases")
    workflow = attributes.get("workflowType", attributes.get("workflow_type"))
    queue = attributes.get("taskQueue", attributes.get("task_queue"))
    if not isinstance(workflow, dict) or not isinstance(queue, dict):
        raise EvidenceError("Temporal History lacks Workflow Type or Task Queue")
    workflow_type, task_queue = workflow.get("name"), queue.get("name")
    if not isinstance(workflow_type, str) or not workflow_type or not isinstance(task_queue, str) or not task_queue:
        raise EvidenceError("Temporal History Workflow Type or Task Queue is invalid")
    return workflow_type, task_queue


def protected_replayer(binary: Path, owner: str, protected_root: Path) -> tuple[Path, dict[str, Any]]:
    """Snapshot the exact executable bytes; execute only the immutable copy.

    Merely checking the path and then hashing it permits a build artifact to
    be replaced before the SDK subprocess starts. This copy is diagnostic
    provenance, not a signature or proof that the binary came from CI.
    """
    if (not isinstance(binary, Path) or binary.is_symlink() or not binary.is_file()
            or not os.access(binary, os.X_OK)):
        raise EvidenceError("required executable SDK replay binary is absent")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(binary, flags)
    except OSError as error:
        raise EvidenceError("SDK replay binary cannot be opened without following symlinks") from error
    destination = protected_root / f"replayer-{owner}"
    digest = hashlib.sha256()
    size = 0
    try:
        with os.fdopen(fd, "rb") as source:
            metadata = os.fstat(source.fileno())
            if not stat.S_ISREG(metadata.st_mode) or not (metadata.st_mode & 0o111):
                raise EvidenceError("SDK replay binary is not a regular executable file")
            if not 0 < metadata.st_size <= MAX_REPLAYER_BYTES:
                raise EvidenceError("SDK replay binary size exceeds the attestation budget")
            with destination.open("xb") as target:
                os.fchmod(target.fileno(), 0o600)
                while chunk := source.read(1024 * 1024):
                    size += len(chunk)
                    if size > MAX_REPLAYER_BYTES:
                        raise EvidenceError("SDK replay binary changed beyond the size budget")
                    digest.update(chunk)
                    target.write(chunk)
                if size != metadata.st_size:
                    raise EvidenceError("SDK replay binary changed during protected copy")
                target.flush()
                os.fchmod(target.fileno(), 0o500)
    except OSError as error:
        raise EvidenceError("SDK replay binary cannot be safely snapshotted") from error
    return destination, {"sha256": digest.hexdigest(), "size_bytes": size}


def _replay_audit_verified(snapshot: dict[str, Any], manifest: dict[str, Any], *,
                           history_root: Path, binaries: dict[str, Path],
                           protected_copy_root: Path,
                           clock: datetime | None = None) -> dict[str, Any]:
    """Replay only verified protected copies with the registered native SDK.

    Returns only candidate evidence; user-supplied exports cannot prove their
    own live server provenance, signature, or completeness. Production remains
    NO-GO and only an independent server-side attestation may close that gate.
    """
    clock = clock or now_utc()
    if (not isinstance(snapshot, dict) or type(snapshot.get("schema_version")) is not int
            or snapshot["schema_version"] != SCHEMA_VERSION):
        raise EvidenceError("unsupported historical capture schema")
    source = snapshot.get("source")
    if (not isinstance(source, dict) or set(source) != {"sha", "tree"}
            or any(not isinstance(value, str) or not SHA_RE.fullmatch(value)
                   for value in source.values())):
        raise EvidenceError("replay snapshot has no exact source identity")
    if source != exact_source(source["sha"]):
        raise EvidenceError("replay snapshot source differs from clean checkout")
    if snapshot.get("environment") not in ("staging", "production"):
        raise EvidenceError("replay snapshot lacks explicit environment")
    captured = timestamp(snapshot.get("captured_at"))
    if clock - captured > timedelta(minutes=10) or captured > clock + timedelta(seconds=15):
        raise EvidenceError("replay snapshot is stale or in the future")
    temporal, postgres = snapshot.get("temporal"), snapshot.get("postgres")
    if not isinstance(temporal, dict) or not isinstance(postgres, dict):
        raise EvidenceError("replay snapshot has no live owner inventories")
    reasons = blockers(temporal, postgres)
    non_replay = [reason for reason in reasons if not reason.startswith("historical SDK replay remains unproved:")]
    if non_replay:
        raise EvidenceError("replay cannot bypass live Temporal drain or PG tenant blockers")
    if (not isinstance(manifest, dict) or type(manifest.get("schema_version")) is not int
            or manifest["schema_version"] != SCHEMA_VERSION
            or manifest.get("source") != source
            or manifest.get("environment") != snapshot["environment"]
            or manifest.get("namespace") != temporal["namespace"]
            or set(manifest) != {"schema_version", "source", "environment", "namespace", "histories"}
            or not isinstance(manifest["histories"], list)):
        raise EvidenceError("replay manifest is not bound to captured source and namespace")
    expected = {name: counts["total"] for name, counts in temporal["legacy_workflow_types"].items()}
    observed = {name: 0 for name in WORKFLOW_TYPES}
    seen_runs: set[tuple[str, str]] = set()
    seen_bytes: set[str] = set()
    prepared: list[tuple[str, str, str, Path, dict[str, str]]] = []
    try:
        root = history_root.resolve(strict=True)
    except OSError as error:
        raise EvidenceError("history export directory is unavailable") from error
    if not root.is_dir():
        raise EvidenceError("history root is not a directory")
    for entry in manifest["histories"]:
        if not isinstance(entry, dict) or set(entry) != {
            "workflow_type", "workflow_id", "run_id", "task_queue", "file", "sha256",
        }:
            raise EvidenceError("invalid history manifest entry")
        workflow_type, task_queue = entry["workflow_type"], entry["task_queue"]
        if workflow_type not in observed or (workflow_type, task_queue) not in REPLAY_ROUTES:
            raise EvidenceError("historical Workflow Type/Task Queue lacks an SDK replay route")
        for name in ("workflow_id", "run_id"):
            if not isinstance(entry[name], str) or not entry[name].strip() or len(entry[name]) > 256:
                raise EvidenceError("historical Workflow ID or Run ID is invalid")
        identity = (entry["workflow_id"], entry["run_id"])
        if identity in seen_runs:
            raise EvidenceError("duplicate historical Workflow ID/Run ID")
        seen_runs.add(identity)
        if not isinstance(entry["sha256"], str) or not HISTORY_SHA_RE.fullmatch(entry["sha256"]):
            raise EvidenceError("invalid SHA-256 of exported History")
        if entry["sha256"] in seen_bytes:
            raise EvidenceError("same exported History reused for different Workflow runs")
        seen_bytes.add(entry["sha256"])
        relative = entry["file"]
        if not isinstance(relative, str) or not relative or Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise EvidenceError("History path is absolute or escapes export directory")
        path = root / relative
        if path.is_symlink():
            raise EvidenceError("History symlink is not trusted")
        try:
            path = path.resolve(strict=True)
            if not path.is_relative_to(root) or not path.is_file() or path.stat().st_size > MAX_HISTORY_BYTES:
                raise EvidenceError("History file is missing, oversized or outside export root")
            raw = path.read_bytes()
        except OSError as error:
            raise EvidenceError("History file cannot be read safely") from error
        # Reject a file that grew between stat and read. The SDK must never
        # reopen the mutable export path after SHA/type checks (TOCTOU).
        if len(raw) > MAX_HISTORY_BYTES:
            raise EvidenceError("History grew beyond offline replay size limit")
        if hashlib.sha256(raw).hexdigest() != entry["sha256"]:
            raise EvidenceError("exported History SHA-256 differs from manifest")
        if history_type_and_queue(raw) != (workflow_type, task_queue):
            raise EvidenceError("exported History conflicts with claimed Type/Queue")
        replay_copy = protected_copy_root / f"{entry['sha256']}.json"
        # TemporaryDirectory is 0700. Create a fresh read-only 0400 file,
        # never a symlink, and use only this exact verified copy for replay.
        with replay_copy.open("xb") as destination:
            os.fchmod(destination.fileno(), 0o600)
            destination.write(raw)
        replay_copy.chmod(0o400)
        owner, command, marker = REPLAY_ROUTES[(workflow_type, task_queue)]
        observed[workflow_type] += 1
        prepared.append((owner, command, marker, replay_copy, {
            "workflow_type": workflow_type, "task_queue": task_queue,
            "workflow_id": entry["workflow_id"], "run_id": entry["run_id"],
            "sha256": entry["sha256"],
        }))
    if observed != expected:
        raise EvidenceError("exported History set does not cover every counted Workflow Type")
    # First check the complete manifest, THEN run any offline replay. An
    # incomplete or unsupported history must never yield partial green output.
    verified_binaries: dict[str, Path] = {}
    replayer_digests: dict[str, dict[str, Any]] = {}
    for owner in {item[0] for item in prepared}:
        verified_binaries[owner], replayer_digests[owner] = protected_replayer(
            binaries.get(owner), owner, protected_copy_root,
        )
    replay_env = {key: os.environ[key] for key in REPLAY_ENV_ALLOW if key in os.environ}
    for owner, command, marker, path, _ in prepared:
        binary = verified_binaries[owner]
        stdout = run_command([str(binary), command, str(path)], env=replay_env)
        if marker not in stdout.splitlines():
            raise EvidenceError("native Temporal SDK replay did not attest PASS")
    return {
        "schema_version": SCHEMA_VERSION, "status": "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY",
        "environment": snapshot["environment"], "source": source,
        "temporal_namespace": temporal["namespace"],
        "replayed_by_type": observed, "replayed_sha256": sorted(seen_bytes),
        "native_replayers": replayer_digests,
        "replayed_runs": sorted(
            (item[4] for item in prepared),
            key=lambda run: (run["workflow_type"], run["workflow_id"], run["run_id"]),
        ),
        "replay_count": len(prepared), "cutover_authorized": False,
        "unverified_external_gates": [
            "independent_server_history_export_identity_and_completeness",
            "authoritative_legacy_starter_fence_and_worker_routing",
            "operator_signed_replay_binary_provenance_and_rollback_drill",
            "production_mTLS_and_explicit_cutover_authorization",
        ],
    }


def replay_audit(snapshot: dict[str, Any], manifest: dict[str, Any], *,
                 history_root: Path, binaries: dict[str, Path],
                 clock: datetime | None = None) -> dict[str, Any]:
    """Offline SDK replay; secure copies auto-delete on success *or* failure."""
    with tempfile.TemporaryDirectory(prefix="alegria-temporal-replay-") as folder:
        return _replay_audit_verified(
            snapshot, manifest, history_root=history_root, binaries=binaries,
            protected_copy_root=Path(folder), clock=clock,
        )


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
               replay_evidence: dict[str, Any] | None = None,
               clock: datetime | None = None) -> dict[str, Any]:
    """Compare two independently captured live snapshots after a drain window.

    This is a *diagnostic precondition*, never a switch/route command.
    Files may be untrusted; no boolean in them grants rollout authority.
    """
    clock = clock or now_utc()
    errors = []
    for name, document in (("earlier", earlier), ("later", later)):
        if (not isinstance(document, dict)
                or type(document.get("schema_version")) is not int
                or document["schema_version"] != SCHEMA_VERSION):
            errors.append(f"{name}: unsupported evidence schema")
            continue
        if document.get("cutover_authorized") is not False:
            errors.append(f"{name}: forbidden cutover_authorized marker")
        temporal_record = document.get("temporal")
        historical = temporal_record.get("legacy_workflow_types") if isinstance(temporal_record, dict) else None
        historical = historical if isinstance(historical, dict) else {}
        expected_status = "BLOCKED" if replay_evidence is not None and any(
            counts.get("total", 0) for counts in historical.values() if isinstance(counts, dict)
        ) else "DRAIN_CANDIDATE_ONLY"
        if document.get("status") != expected_status:
            errors.append(f"{name}: drain inventory status conflicts with historical counts")
        if replay_evidence is None and document.get("blockers") != []:
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
            if replay_evidence is not None:
                if record.get("blockers") != reasons:
                    errors.append(f"{name}: capture blocker list differs from live inventory")
                reasons = [reason for reason in reasons if not reason.startswith(
                    "historical SDK replay remains unproved:")]
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
    if errors:
        return {"status": "BLOCKED", "errors": errors, "cutover_authorized": False}
    if replay_evidence is not None:
        if not isinstance(replay_evidence, dict):
            errors.append("replay evidence is not an object")
        else:
            if (type(replay_evidence.get("schema_version")) is not int
                    or replay_evidence["schema_version"] != SCHEMA_VERSION
                    or replay_evidence.get("status") != "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY"
                    or replay_evidence.get("cutover_authorized") is not False
                    or replay_evidence.get("source") != later["source"]
                    or replay_evidence.get("environment") != later["environment"]
                    or replay_evidence.get("temporal_namespace") != later["temporal"].get("namespace")):
                errors.append("replay evidence source/environment/namespace/status mismatch")
            expected = {
                name: counts["total"] for name, counts in
                later["temporal"]["legacy_workflow_types"].items()
            }
            counts = replay_evidence.get("replayed_by_type")
            if (not isinstance(counts, dict) or counts != expected
                    or any(type(value) is not int or value < 0 for value in counts.values())):
                errors.append("SDK replay coverage differs from historical census")
            replay_hashes = replay_evidence.get("replayed_sha256")
            replay_count = replay_evidence.get("replay_count")
            if (not isinstance(replay_hashes, list)
                    or not all(isinstance(value, str) and HISTORY_SHA_RE.fullmatch(value)
                               for value in replay_hashes)
                    or len(set(replay_hashes)) != len(replay_hashes)
                    or type(replay_count) is not int or replay_count != sum(expected.values())
                    or len(replay_hashes) != replay_count):
                errors.append("SDK replay run identities or hashes are incomplete")
            replayed_runs = replay_evidence.get("replayed_runs")
            reported_by_type = {name: 0 for name in expected}
            reported_identity: set[tuple[str, str]] = set()
            reported_hashes: set[str] = set()
            runs_valid = isinstance(replayed_runs, list) and len(replayed_runs) == replay_count
            if runs_valid:
                for run in replayed_runs:
                    if (not isinstance(run, dict) or set(run) != {
                        "workflow_type", "task_queue", "workflow_id", "run_id", "sha256",
                    } or not all(isinstance(value, str) and value for value in run.values())
                            or run["workflow_type"] not in expected
                            or (run["workflow_type"], run["task_queue"]) not in REPLAY_ROUTES
                            or not HISTORY_SHA_RE.fullmatch(run["sha256"])):
                        runs_valid = False
                        break
                    identity = (run["workflow_id"], run["run_id"])
                    if identity in reported_identity or run["sha256"] in reported_hashes:
                        runs_valid = False
                        break
                    reported_identity.add(identity)
                    reported_hashes.add(run["sha256"])
                    reported_by_type[run["workflow_type"]] += 1
            if (not runs_valid or reported_by_type != expected
                    or not isinstance(replay_hashes, list)
                    or reported_hashes != set(replay_hashes)):
                errors.append("SDK replay per-run Type/Queue/Workflow ID/Run ID/SHA mapping is inconsistent")
            expected_owners = {
                REPLAY_ROUTES[(run["workflow_type"], run["task_queue"])][0]
                for run in replayed_runs if runs_valid
            } if isinstance(replayed_runs, list) else set()
            replayers = replay_evidence.get("native_replayers")
            if (not isinstance(replayers, dict) or set(replayers) != expected_owners
                    or any(not isinstance(meta, dict) or set(meta) != {"sha256", "size_bytes"}
                           or not isinstance(meta["sha256"], str)
                           or not HISTORY_SHA_RE.fullmatch(meta["sha256"])
                           or type(meta["size_bytes"]) is not int
                           or not 0 < meta["size_bytes"] <= MAX_REPLAYER_BYTES
                           for meta in replayers.values())):
                errors.append("native replay executable hashes or owner mapping are missing")
    return {
        "status": "BLOCKED" if errors else (
            "HISTORY_REPLAY_PRECONDITIONS_CAPTURED" if replay_evidence is not None
            else "DRAIN_PRECONDITIONS_CAPTURED"
        ),
        "errors": errors, "cutover_authorized": False,
        "still_required": [
            "server_verified_legacy_starter_freeze_and_no_duplicate_pollers",
            "official_sdk_replay_of_all_historical_workflow_types",
            "verified_rollback_fleet_and_routing_reversal",
            "production_operator_authorization_and_mTLS",
            "independent_server_attestation_of_exported_history_and_run_identity",
        ],
    }


def rehearse_cutover_plan(earlier: dict[str, Any], later: dict[str, Any],
                          plan: dict[str, Any], *,
                          replay_evidence: dict[str, Any] | None = None,
                          clock: datetime | None = None) -> dict[str, Any]:
    """Check a versioned routing/rollback plan, without any routing side effect.

    The plan is *operator-proposed*, not operator-authorized or server-attested.
    Output is diagnostic evidence and can never unlock a production cutover.
    """
    comparison = check_pair(earlier, later, replay_evidence=replay_evidence, clock=clock)
    errors = list(comparison["errors"])
    if comparison["status"] not in {
        "DRAIN_PRECONDITIONS_CAPTURED", "HISTORY_REPLAY_PRECONDITIONS_CAPTURED",
    }:
        errors.append("read-only two-snapshot/replay prerequisites have not passed")
        return {"status": "BLOCKED", "errors": errors, "cutover_authorized": False}
    if (not isinstance(plan, dict) or set(plan) != {
        "schema_version", "source", "environment", "namespace", "routes",
    } or type(plan["schema_version"]) is not int
            or plan["schema_version"] != SCHEMA_VERSION
            or not isinstance(plan["routes"], list) or not plan["routes"]):
        errors.append("routing rehearsal plan has an unsupported or empty schema")
        return {"status": "BLOCKED", "errors": errors, "cutover_authorized": False}
    if (plan["source"] != later.get("source")
            or plan["environment"] != later.get("environment")
            or plan["namespace"] != later.get("temporal", {}).get("namespace")):
        errors.append("routing rehearsal plan differs from audited namespace/source")
    # A historical Type can have multiple generations of Task Queue. A plan
    # must not retire *any* plausible older routing path while retained
    # histories exist. Server-verified actual build IDs remain an external gate.
    retained_types = {
        name for name, counts in later.get("temporal", {}).get(
            "legacy_workflow_types", {}).items()
        if isinstance(counts, dict) and counts.get("total", 0) > 0
    }
    protected_queues = {
        queue for workflow_type, queue in REPLAY_ROUTES if workflow_type in retained_types
    }
    route_owners: dict[str, str] = {}
    route_summaries: list[dict[str, str]] = []
    for route in plan["routes"]:
        if not isinstance(route, dict) or set(route) != {
            "task_queue", "owner", "from_build_id", "to_build_id", "rollback_build_id",
        } or any(not isinstance(value, str) or not ROUTING_IDENTIFIER_RE.fullmatch(value)
                 for value in route.values()):
            errors.append("routing rehearsal contains an invalid route/Build ID")
            continue
        queue = route["task_queue"]
        if queue in route_owners:
            errors.append("routing rehearsal attempts duplicate Task Queue ownership")
        route_owners[queue] = route["owner"]
        if route["owner"] not in {"legacy", "orchestrator", "pg", "qdrant"}:
            errors.append("routing rehearsal proposes an unknown Worker owner")
        known_owners = {
            REPLAY_ROUTES[(workflow_type, routed_queue)][0]
            for workflow_type, routed_queue in REPLAY_ROUTES if routed_queue == queue
        }
        if not known_owners:
            errors.append("routing rehearsal references an unregistered native Task Queue")
        if known_owners and route["owner"] not in known_owners:
            errors.append("routing rehearsal reassigns a registered Queue to the wrong owner")
        if queue in protected_queues:
            errors.append("routing rehearsal would change a Queue with retained histories")
        if (route["from_build_id"] == route["to_build_id"]
                or route["rollback_build_id"] != route["from_build_id"]):
            errors.append("routing rehearsal lacks an exact reversible Build ID transition")
        route_summaries.append(dict(route))
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "BLOCKED" if errors else "ROUTING_REHEARSAL_CANDIDATE_ONLY",
        "errors": errors, "cutover_authorized": False,
        "environment": plan["environment"], "namespace": plan["namespace"],
        "source": plan["source"],
        "routing_changes_reviewed": sorted(route_summaries, key=lambda item: item["task_queue"]),
        "old_queues_with_retained_history": sorted(protected_queues),
        "unverified_external_gates": [
            "server_attested_run_bound_history_and_tenant_backfill",
            "independently_fenced_legacy_starters_and_task_queue_pollers",
            "actual_build_id_routing_and_signed_exact_source_binary_provenance",
            "operator_approved_live_rollback_and_recovery_drill",
            "production_mTLS_and_change_control_authorization",
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
    compare.add_argument("--replay-evidence", type=Path,
                         help="optional OFFLINE_SDK_REPLAY_CANDIDATE_ONLY report")
    compare.add_argument("--output", type=Path, required=True)
    replay = sub.add_parser(
        "replay-audit",
        help="offline SDK replay of exact exported histories; never authorizes cutover",
    )
    replay.add_argument("--capture", type=Path, required=True)
    replay.add_argument("--manifest", type=Path, required=True)
    replay.add_argument("--history-root", type=Path, required=True)
    replay.add_argument("--legacy-bin", type=Path, required=True)
    replay.add_argument("--orchestrator-bin", type=Path, required=True)
    replay.add_argument("--pg-bin", type=Path, required=True)
    replay.add_argument("--qdrant-bin", type=Path,
                        help="required only when a Qdrant child history is present")
    replay.add_argument("--output", type=Path, required=True)
    rehearsal = sub.add_parser("rehearse", help="validate hypothetical Worker routing; never changes it")
    rehearsal.add_argument("--earlier", type=Path, required=True)
    rehearsal.add_argument("--later", type=Path, required=True)
    rehearsal.add_argument("--replay-evidence", type=Path)
    rehearsal.add_argument("--routing-plan", type=Path, required=True)
    rehearsal.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "capture":
            outcome = capture(
                environment=args.environment, address=args.temporal_address,
                namespace=args.temporal_namespace, dsn_env=args.dsn_env,
                source_sha=args.source_sha,
            )
        elif args.command == "compare":
            outcome = check_pair(
                json.loads(args.earlier.read_text(encoding="utf-8")),
                json.loads(args.later.read_text(encoding="utf-8")),
                replay_evidence=(
                    json.loads(args.replay_evidence.read_text(encoding="utf-8"))
                    if args.replay_evidence is not None else None
                ),
            )
        elif args.command == "replay-audit":
            outcome = replay_audit(
                json.loads(args.capture.read_text(encoding="utf-8")),
                json.loads(args.manifest.read_text(encoding="utf-8")),
                history_root=args.history_root,
                binaries={
                    "legacy": args.legacy_bin,
                    "orchestrator": args.orchestrator_bin,
                    "pg": args.pg_bin,
                    "qdrant": args.qdrant_bin,
                },
            )
        else:
            outcome = rehearse_cutover_plan(
                json.loads(args.earlier.read_text(encoding="utf-8")),
                json.loads(args.later.read_text(encoding="utf-8")),
                json.loads(args.routing_plan.read_text(encoding="utf-8")),
                replay_evidence=(
                    json.loads(args.replay_evidence.read_text(encoding="utf-8"))
                    if args.replay_evidence is not None else None
                ),
            )
        atomic_json(args.output, outcome)
        print(f"TEMPORAL_HISTORY_TRANSITION={outcome['status']} cutover_authorized=false")
        return 0 if outcome["status"] in {
            "DRAIN_CANDIDATE_ONLY", "DRAIN_PRECONDITIONS_CAPTURED",
            "OFFLINE_SDK_REPLAY_CANDIDATE_ONLY", "HISTORY_REPLAY_PRECONDITIONS_CAPTURED",
            "ROUTING_REHEARSAL_CANDIDATE_ONLY",
        } else 1
    except (EvidenceError, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        # The error reason must never include a DSN, URI query, token or stderr.
        print(f"TEMPORAL_HISTORY_TRANSITION=BLOCKED reason={type(error).__name__}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
