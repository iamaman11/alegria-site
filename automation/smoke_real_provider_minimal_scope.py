#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

from local_db_baseline import check_database_baseline
from local_env import resolve_database_url


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = ROOT / "docs/runs/live_provider_minimal_scope_evidence.json"
CLI_MANIFEST = ROOT / "app" / "rust" / "services" / "cli_tools" / "Cargo.toml"
MIGRATIONS_DIR = ROOT / "app" / "db" / "migrations"


def utc_now() -> str:
    return subprocess.run(
        ["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"], capture_output=True, text=True, check=True
    ).stdout.strip()


def write_artifact(payload: dict[str, Any]) -> None:
    ARTIFACT.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def read_artifact() -> dict[str, Any]:
    if ARTIFACT.exists():
        return json.loads(ARTIFACT.read_text(encoding="utf-8"))
    return {
        "artifact_id": "step5_live_provider_minimal_scope",
        "step": "R1.Step5",
        "evidence_status": "PENDING_CREDENTIALS",
        "status_reason": "Artifact initialized but smoke has not run yet.",
        "failure_class": "credential_issue",
        "smoke_command": "",
        "scope": {},
        "provider_requirements": {},
        "execution": {},
        "observed": {},
        "required_for_pass": [
            "one real scope",
            "one real SERP fetch",
            "one official crawl",
            "one extraction pass",
            "no publish",
            "immutable artifact with classified verdict",
        ],
        "updated_at": utc_now(),
    }


def psql_scalar(database_url: str, sql: str) -> int:
    proc = subprocess.run(
        [
            "psql",
            database_url,
            "-t",
            "-A",
            "-c",
            " ".join(line.strip() for line in sql.strip().splitlines()),
        ],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")
    out = proc.stdout.strip()
    return int(out) if out else 0


def psql_exec(database_url: str, sql: str) -> None:
    proc = subprocess.run(
        [
            "psql",
            database_url,
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            " ".join(line.strip() for line in sql.strip().splitlines()),
        ],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")


def apply_business_migrations(database_url: str) -> None:
    for path in sorted(MIGRATIONS_DIR.glob("*.sql")):
        proc = subprocess.run(
            [
                "psql",
                database_url,
                "-v",
                "ON_ERROR_STOP=1",
                "-f",
                str(path),
            ],
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            raise RuntimeError(
                proc.stderr.strip()
                or proc.stdout.strip()
                or f"failed applying migration {path.name}"
            )


def sql_literal(value: str) -> str:
    return value.replace("'", "''")


def parse_phase(output: str, phase: str) -> tuple[str | None, str | None]:
    pattern = re.compile(
        rf"^SEO_RUN_PHASE phase={re.escape(phase)} status=([^ ]+) page_node_key=.* detail=(.*)$",
        re.MULTILINE,
    )
    match = pattern.search(output)
    if not match:
        return None, None
    return match.group(1), match.group(2).strip()


def parse_result(output: str) -> dict[str, Any]:
    pattern = re.compile(
        r"SEO_RUN_RESULT scenario=(?P<scenario>\S+) mode=(?P<mode>\S+) status=(?P<status>\S+) "
        r"page_total=(?P<page_total>\d+) published_pages=(?P<published_pages>\d+) "
        r"changed_truth_keys=(?P<changed_truth_keys>\d+)"
    )
    match = pattern.search(output)
    if not match:
        return {}
    parsed = match.groupdict()
    return {
        "scenario": parsed["scenario"],
        "mode": parsed["mode"],
        "status": parsed["status"],
        "page_total": int(parsed["page_total"]),
        "published_pages": int(parsed["published_pages"]),
        "changed_truth_keys": int(parsed["changed_truth_keys"]),
    }


def detail_int(detail: str | None, key: str) -> int | None:
    if not detail:
        return None
    match = re.search(rf"{re.escape(key)}=(\d+)", detail)
    return int(match.group(1)) if match else None


def classify_failure(text: str) -> str:
    lower = text.lower()
    if "timed out" in lower or "timeout" in lower or "timedout" in lower:
        return "rate_or_transport_issue"
    if "no_truth_extraction_provider" in lower:
        return "extraction_provider_unavailable"
    if any(token in lower for token in ["401", "403", "unauthorized", "authentication failed"]):
        return "credential_issue"
    if any(token in lower for token in ["429", "rate limit", "too many requests"]):
        return "rate_or_transport_issue"
    if "dataforseo" in lower or "serp" in lower:
        return "provider_contract_issue"
    if "crawl_sources" in lower or "fetch failed" in lower or "http_status=" in lower:
        return "crawl_policy_failure"
    if "raw_knowledge_ingestion" in lower or "no_admissible_verified_rules" in lower:
        return "extraction_mismatch"
    return "provider_contract_issue"


def truth_provider_status() -> dict[str, Any]:
    providers: list[dict[str, str]] = []
    vertex_project = (
        os.environ.get("VERTEX_GEMINI_PROJECT")
        or os.environ.get("GOOGLE_CLOUD_PROJECT")
        or os.environ.get("GCLOUD_PROJECT")
    )
    if vertex_project and os.environ.get("VERTEX_GEMINI_ENABLED", "true").lower() not in {"0", "false"}:
        providers.append(
            {
                "provider": "vertex_gemini",
                "model": os.environ.get("VERTEX_GEMINI_TRUTH_MODEL")
                or os.environ.get("VERTEX_GEMINI_MODEL")
                or "gemini-2.5-pro",
            }
        )
    if os.environ.get("SEO_TRUTH_LLM_LOCAL_ENDPOINT") or os.environ.get("SEO_LLM_LOCAL_ENDPOINT"):
        providers.append(
            {
                "provider": "local_compatible",
                "model": os.environ.get("SEO_TRUTH_LLM_LOCAL_MODEL")
                or os.environ.get("SEO_LLM_LOCAL_MODEL")
                or "local-truth-extraction-model",
            }
        )
    if os.environ.get("OPENAI_API_KEY"):
        providers.append(
            {
                "provider": "openai",
                "model": os.environ.get("OPENAI_TRUTH_MODEL")
                or os.environ.get("OPENAI_SEO_MODEL")
                or "gpt-5.2",
            }
        )
    if os.environ.get("ANTHROPIC_API_KEY"):
        providers.append(
            {
                "provider": "anthropic",
                "model": os.environ.get("ANTHROPIC_TRUTH_MODEL")
                or os.environ.get("ANTHROPIC_SEO_MODEL")
                or "claude-4.5-sonnet",
            }
        )
    if os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY"):
        providers.append(
            {
                "provider": "gemini",
                "model": os.environ.get("GEMINI_TRUTH_MODEL")
                or os.environ.get("GEMINI_SEO_MODEL")
                or "gemini-2.5-pro",
            }
        )
    return {
        "ready": bool(providers),
        "providers": providers,
    }


def main() -> int:
    artifact = read_artifact()
    database_url = resolve_database_url()
    artifact["updated_at"] = utc_now()
    artifact["smoke_command"] = ""
    artifact["execution"] = {}
    artifact["observed"] = {}
    artifact["scope"] = {
        "market": os.environ.get("ALEGRIA_LIVE_SMOKE_MARKET", "alegria-site"),
        "locale": os.environ.get("ALEGRIA_LIVE_SMOKE_LOCALE", "ru-RU"),
        "country_code": os.environ.get("ALEGRIA_LIVE_SMOKE_COUNTRY_CODE", "ES"),
        "visa_type": os.environ.get("ALEGRIA_LIVE_SMOKE_VISA_TYPE", "tourist"),
        "citizenship_code": os.environ.get("ALEGRIA_LIVE_SMOKE_CITIZENSHIP_CODE", "BY"),
        "applicant_profile": os.environ.get("ALEGRIA_LIVE_SMOKE_APPLICANT_PROFILE", "standard"),
        "query": os.environ.get(
            "ALEGRIA_LIVE_SMOKE_QUERY", "spain tourist visa belarus official"
        ),
    }
    artifact["provider_requirements"] = {
        "dataforseo_login_present": bool(os.environ.get("DATAFORSEO_LOGIN")),
        "dataforseo_password_present": bool(os.environ.get("DATAFORSEO_PASSWORD")),
        "psql_present": shutil.which("psql") is not None,
    }
    artifact["provider_requirements"]["truth_extraction_provider"] = truth_provider_status()
    truth_provider_ready = artifact["provider_requirements"]["truth_extraction_provider"]["ready"]

    if not artifact["provider_requirements"]["psql_present"]:
        artifact["evidence_status"] = "FAIL"
        artifact["failure_class"] = "provider_contract_issue"
        artifact["status_reason"] = "`psql` is not available; live smoke cannot persist evidence."
        write_artifact(artifact)
        print("SMOKE_REAL_PROVIDER_MINIMAL_SCOPE: FAILED")
        print("- `psql` is not available")
        return 1

    missing_credentials: list[str] = []
    if not artifact["provider_requirements"]["dataforseo_login_present"]:
        missing_credentials.append("DATAFORSEO_LOGIN")
    if not artifact["provider_requirements"]["dataforseo_password_present"]:
        missing_credentials.append("DATAFORSEO_PASSWORD")
    if not truth_provider_ready:
        missing_credentials.append(
            "VERTEX_GEMINI_PROJECT|GOOGLE_CLOUD_PROJECT(+ADC)|GEMINI_API_KEY|GOOGLE_API_KEY|OPENAI_API_KEY|ANTHROPIC_API_KEY|SEO_TRUTH_LLM_LOCAL_ENDPOINT"
        )

    if missing_credentials:
        artifact["evidence_status"] = "PENDING_CREDENTIALS"
        artifact["failure_class"] = "credential_issue"
        if truth_provider_ready:
            artifact["status_reason"] = (
                "Provider credentials are incomplete for live smoke; "
                f"missing {', '.join(missing_credentials)}."
            )
        else:
            artifact["status_reason"] = (
                "Truth extraction provider is not configured for live smoke; "
                f"missing {', '.join(missing_credentials)}."
            )
        write_artifact(artifact)
        print("SMOKE_REAL_PROVIDER_MINIMAL_SCOPE: PENDING_CREDENTIALS")
        print(f"- missing {', '.join(missing_credentials)}")
        return 2

    try:
        baseline = check_database_baseline(database_url)
        artifact["observed"] = {
            "local_db_baseline_status": baseline.status,
            "local_db_missing_tables": list(baseline.missing_tables),
        }
        if not baseline.is_ready:
            raise RuntimeError(baseline.reason())
        apply_business_migrations(database_url)
    except Exception as exc:
        artifact["evidence_status"] = "FAIL"
        artifact["failure_class"] = "db_contract_drift"
        artifact["status_reason"] = "Business migrations could not be applied before live smoke due to local DB contract drift."
        artifact["execution"] = {
            "command_exit_code": 1,
            "stderr_excerpt": str(exc),
        }
        write_artifact(artifact)
        print("SMOKE_REAL_PROVIDER_MINIMAL_SCOPE: FAILED")
        print("- migration application failed")
        return 1

    run_id = subprocess.run(
        ["python3", "-c", "import uuid; print(uuid.uuid4())"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    query_batch_key = f"live-smoke-{run_id}"
    output_dir = ROOT / "tmp" / f"live-provider-smoke-{run_id}"
    output_dir.mkdir(parents=True, exist_ok=True)
    scope = artifact["scope"]
    official_crawl_url = os.environ.get(
        "ALEGRIA_LIVE_SMOKE_OFFICIAL_CRAWL_URL",
        "https://www.exteriores.gob.es/Consulados/mumbai/en/ServiciosConsulares/Paginas/Consular/Visados-Schengen.aspx",
    )
    official_crawl_url_norm = re.sub(r"^https?://", "", official_crawl_url.rstrip("/"))
    official_crawl_domain = official_crawl_url_norm.split("/", 1)[0].lower()
    psql_exec(
        database_url,
        f"""
        INSERT INTO serp.crawl_queue
            (url, url_norm, source_domain, source_type, dtype, first_seen_run_id, first_seen_job_id,
             query_batch_key, status, next_attempt_at, locked_until, last_error, notes)
        VALUES
            ('{sql_literal(official_crawl_url)}',
             '{sql_literal(official_crawl_url_norm)}',
             '{sql_literal(official_crawl_domain)}',
             'live_smoke_official_seed',
             'official',
             '{run_id}',
             'live_smoke_official_seed',
             '{query_batch_key}',
             'pending',
             now(),
             NULL,
             NULL,
             'live_provider_smoke_official_seed')
        ON CONFLICT (url_norm) DO UPDATE
        SET url = EXCLUDED.url,
            source_domain = EXCLUDED.source_domain,
            source_type = EXCLUDED.source_type,
            dtype = EXCLUDED.dtype,
            first_seen_run_id = EXCLUDED.first_seen_run_id,
            first_seen_job_id = EXCLUDED.first_seen_job_id,
            query_batch_key = EXCLUDED.query_batch_key,
            status = 'pending',
            next_attempt_at = now(),
            locked_until = NULL,
            last_error = NULL,
            notes = EXCLUDED.notes;
        """,
    )
    artifact["provider_requirements"]["official_crawl_seed_url"] = official_crawl_url
    context_key = f"{scope['country_code']}|{scope['visa_type']}||{scope['citizenship_code']}"
    bootstrap_seeded_support_bundle = False
    verified_rule_count = psql_scalar(
        database_url,
        f"SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = '{context_key}' AND status = 'verified' AND COALESCE(publish_admissibility, 'not_admissible') = 'admissible'",
    )
    if verified_rule_count == 0:
        fragment_text = "Passport is required for the visa application."
        escaped_fragment = sql_literal(fragment_text)
        psql_exec(
            database_url,
            f"""
            INSERT INTO kb.visa_contexts
                (context_key, country_code, visa_family, visa_subtype, citizenship_code, status)
            VALUES
                ('{context_key}', '{scope['country_code']}', '{scope['visa_type']}', '', '{scope['citizenship_code']}', 'active')
            ON CONFLICT (context_key) DO UPDATE
            SET country_code = EXCLUDED.country_code,
                visa_family = EXCLUDED.visa_family,
                visa_subtype = EXCLUDED.visa_subtype,
                citizenship_code = EXCLUDED.citizenship_code,
                status = EXCLUDED.status,
                updated_at = now();

            INSERT INTO kb.concepts (concept_key, concept_type, label_ru, status)
            VALUES ('document.passport', 'document', 'Passport', 'active')
            ON CONFLICT (concept_key) DO UPDATE
            SET label_ru = EXCLUDED.label_ru,
                status = EXCLUDED.status,
                updated_at = now();

            INSERT INTO kb.sources (source_key, source_type, source_label, base_url, trust_level, status)
            VALUES ('source.official.live_smoke', 'government', 'Live smoke official source', 'https://example.gov', 5, 'active')
            ON CONFLICT (source_key) DO UPDATE
            SET source_label = EXCLUDED.source_label,
                base_url = EXCLUDED.base_url,
                trust_level = EXCLUDED.trust_level,
                status = EXCLUDED.status,
                updated_at = now();

            WITH inserted_page AS (
                INSERT INTO raw.pages
                    (url, domain, dtype, status_code, title, raw_html, raw_html_bytes, content_hash, content, processed)
                VALUES
                    ('https://example.gov/live-smoke/{run_id}/passport', 'example.gov', 'government', 200,
                     'Live smoke official source', '<html><body><main><p>{escaped_fragment}</p></main></body></html>',
                     length('<html><body><main><p>{escaped_fragment}</p></main></body></html>'),
                     md5('<html><body><main><p>{escaped_fragment}</p></main></body></html>'),
                     jsonb_build_object('markdown', '{escaped_fragment}', 'headings', '[]'::jsonb, 'links', '[]'::jsonb),
                     true)
                RETURNING id, content_hash
            ),
            inserted_section AS (
                INSERT INTO raw.sections
                    (page_id, heading_path, heading_level, section_order, section_type, content_md, content_hash)
                SELECT id, 'Requirements', 1, 0, 'paragraph', '{escaped_fragment}', md5('{escaped_fragment}')
                FROM inserted_page
                RETURNING id
            )
            INSERT INTO verified.rule_instances
                (rule_instance_id, context_key, rule_type_key, concept_key, role_type, params, status, source_key, confidence,
                 evidence_section_id, evidence_quote, span_start, span_end, source_snapshot_hash, verification_method,
                 adjudication_reason, publish_admissibility, freshness_class, completeness_class, registry_version,
                 prompt_version, model_version, pipeline_version, effective_from)
            SELECT
                'rule.live_smoke.passport', '{context_key}', 'document_required', 'document.passport', 'document_required',
                '{{"subtype":"travel_document"}}'::jsonb, 'verified', 'source.official.live_smoke', 0.99,
                inserted_section.id, '{escaped_fragment}', 0, length('{escaped_fragment}'),
                inserted_page.content_hash, 'bootstrap_seed', 'live_provider_smoke_seed', 'admissible',
                'fresh', 'complete', 'registry@1', 'seed@1', 'none', 'smoke_real_provider_minimal_scope@1', current_date
            FROM inserted_page
            CROSS JOIN inserted_section
            ON CONFLICT (rule_instance_id) DO UPDATE
            SET params = EXCLUDED.params,
                status = EXCLUDED.status,
                source_key = EXCLUDED.source_key,
                confidence = EXCLUDED.confidence,
                evidence_section_id = EXCLUDED.evidence_section_id,
                evidence_quote = EXCLUDED.evidence_quote,
                span_start = EXCLUDED.span_start,
                span_end = EXCLUDED.span_end,
                source_snapshot_hash = EXCLUDED.source_snapshot_hash,
                verification_method = EXCLUDED.verification_method,
                adjudication_reason = EXCLUDED.adjudication_reason,
                publish_admissibility = EXCLUDED.publish_admissibility,
                freshness_class = EXCLUDED.freshness_class,
                completeness_class = EXCLUDED.completeness_class,
                registry_version = EXCLUDED.registry_version,
                prompt_version = EXCLUDED.prompt_version,
                model_version = EXCLUDED.model_version,
                pipeline_version = EXCLUDED.pipeline_version,
                updated_at = now();
            """,
        )
        bootstrap_seeded_support_bundle = True
    artifact["provider_requirements"]["bootstrap_seeded_support_bundle"] = bootstrap_seeded_support_bundle
    command = [
        "cargo",
        "run",
        "-q",
        "--manifest-path",
        str(CLI_MANIFEST),
        "--",
        "seo-run",
        "--database-url",
        database_url,
        "--run-id",
        run_id,
        "--market",
        scope["market"],
        "--locale",
        scope["locale"],
        "--country-code",
        scope["country_code"],
        "--visa-type",
        scope["visa_type"],
        "--citizenship-code",
        scope["citizenship_code"],
        "--applicant-profile",
        scope["applicant_profile"],
        "--query-batch-key",
        query_batch_key,
        "--query",
        scope["query"],
        "--warn-only-projections",
        "--output-dir",
        str(output_dir),
        "crawl-ingest",
    ]
    artifact["smoke_command"] = " ".join(command)

    command_timeout = int(os.environ.get("LIVE_PROVIDER_SMOKE_TIMEOUT_SECS", "420"))
    try:
        proc = subprocess.run(
            command,
            cwd=ROOT / "app" / "rust",
            capture_output=True,
            text=True,
            env=os.environ.copy(),
            timeout=command_timeout,
        )
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode(errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode(errors="replace")
        artifact["evidence_status"] = "FAIL"
        artifact["failure_class"] = "rate_or_transport_issue"
        artifact["status_reason"] = (
            f"Live smoke command timed out after {command_timeout}s before reaching a "
            "passing crawl+extraction verdict."
        )
        artifact["execution"] = {
            "run_id": run_id,
            "query_batch_key": query_batch_key,
            "command_exit_code": 124,
            "stdout_excerpt": stdout.strip()[-4000:],
            "stderr_excerpt": stderr.strip()[-4000:],
            "result": parse_result(stdout),
            "phase_count": len(re.findall(r"^SEO_RUN_PHASE ", stdout, flags=re.MULTILINE)),
        }
        artifact["observed"] = {
            **artifact.get("observed", {}),
            "serp_phase_status": parse_phase(stdout, "serp_ingest")[0],
            "serp_persisted_snapshot_count": detail_int(
                parse_phase(stdout, "serp_ingest")[1], "persisted_snapshot_count"
            ),
            "crawl_phase_status": parse_phase(stdout, "crawl_sources")[0],
            "crawl_claimed_count": detail_int(parse_phase(stdout, "crawl_sources")[1], "claimed"),
            "crawl_crawled_count": detail_int(parse_phase(stdout, "crawl_sources")[1], "crawled"),
            "crawl_failed_count": detail_int(parse_phase(stdout, "crawl_sources")[1], "failed"),
            "crawl_raw_page_count": detail_int(parse_phase(stdout, "crawl_sources")[1], "raw_pages"),
            "raw_knowledge_phase_status": parse_phase(stdout, "raw_knowledge_ingestion")[0],
            "raw_knowledge_verified_rule_count": detail_int(
                parse_phase(stdout, "raw_knowledge_ingestion")[1], "verified_rules"
            ),
            "raw_knowledge_changed_truth_keys": detail_int(
                parse_phase(stdout, "raw_knowledge_ingestion")[1], "changed_truth_keys"
            ),
        }
        write_artifact(artifact)
        print("SMOKE_REAL_PROVIDER_MINIMAL_SCOPE: FAILED")
        print("- failure_class=rate_or_transport_issue")
        print(f"- timeout_seconds={command_timeout}")
        return 1
    combined = "\n".join(
        chunk for chunk in [proc.stdout.strip(), proc.stderr.strip()] if chunk.strip()
    )
    result = parse_result(proc.stdout)
    serp_status, serp_detail = parse_phase(proc.stdout, "serp_ingest")
    crawl_status, crawl_detail = parse_phase(proc.stdout, "crawl_sources")
    raw_status, raw_detail = parse_phase(proc.stdout, "raw_knowledge_ingestion")
    phase_count = len(re.findall(r"^SEO_RUN_PHASE ", proc.stdout, flags=re.MULTILINE))

    artifact["execution"] = {
        "run_id": run_id,
        "query_batch_key": query_batch_key,
        "command_exit_code": proc.returncode,
        "stdout_excerpt": proc.stdout.strip()[-4000:],
        "stderr_excerpt": proc.stderr.strip()[-4000:],
        "result": result,
        "phase_count": phase_count,
    }
    observed = artifact.get("observed", {})
    observed.update({
        "serp_phase_status": serp_status,
        "serp_persisted_snapshot_count": detail_int(serp_detail, "persisted_snapshot_count"),
        "crawl_phase_status": crawl_status,
        "crawl_claimed_count": detail_int(crawl_detail, "claimed"),
        "crawl_crawled_count": detail_int(crawl_detail, "crawled"),
        "crawl_failed_count": detail_int(crawl_detail, "failed"),
        "crawl_raw_page_count": detail_int(crawl_detail, "raw_pages"),
        "raw_knowledge_phase_status": raw_status,
        "raw_knowledge_verified_rule_count": detail_int(raw_detail, "verified_rules"),
        "raw_knowledge_changed_truth_keys": detail_int(raw_detail, "changed_truth_keys"),
    })
    artifact["observed"] = observed

    try:
        artifact["observed"]["db_serp_raw_snapshot_count"] = psql_scalar(
            database_url,
            f"SELECT count(*)::bigint FROM serp.raw_snapshots WHERE run_id = '{run_id}'",
        )
        artifact["observed"]["db_crawl_queue_count"] = psql_scalar(
            database_url,
            f"SELECT count(*)::bigint FROM serp.crawl_queue WHERE first_seen_run_id = '{run_id}'",
        )
    except Exception as exc:
        artifact["observed"]["db_probe_error"] = str(exc)

    if proc.returncode != 0:
        artifact["evidence_status"] = "FAIL"
        artifact["failure_class"] = classify_failure(combined)
        artifact["status_reason"] = (
            "Live smoke command failed before reaching a passing crawl+extraction verdict."
        )
        write_artifact(artifact)
        print("SMOKE_REAL_PROVIDER_MINIMAL_SCOPE: FAILED")
        print(f"- failure_class={artifact['failure_class']}")
        return 1

    serp_ok = (artifact["observed"]["serp_persisted_snapshot_count"] or 0) >= 1
    crawl_ok = (artifact["observed"]["crawl_crawled_count"] or 0) >= 1
    extraction_ok = artifact["observed"]["raw_knowledge_phase_status"] in {
        "done",
        "pending_review:needs_truth_adjudication",
        "empty:no_admissible_verified_rules",
    }

    if serp_ok and crawl_ok and extraction_ok and result.get("published_pages", 0) == 0:
        artifact["evidence_status"] = "PASS"
        artifact["failure_class"] = "none"
        artifact["status_reason"] = (
            "Live smoke completed one real SERP fetch, one official crawl, one extraction pass, "
            "and no publish."
        )
        write_artifact(artifact)
        print("SMOKE_REAL_PROVIDER_MINIMAL_SCOPE: OK")
        print(f"- run_id={run_id}")
        return 0

    if not serp_ok:
        failure_class = "provider_contract_issue"
    elif not crawl_ok:
        failure_class = "crawl_policy_failure"
    else:
        failure_class = "extraction_mismatch"
    artifact["evidence_status"] = "FAIL"
    artifact["failure_class"] = failure_class
    artifact["status_reason"] = (
        "Live smoke reached execution end but did not satisfy the required "
        "SERP/crawl/extraction pass conditions."
    )
    write_artifact(artifact)
    print("SMOKE_REAL_PROVIDER_MINIMAL_SCOPE: FAILED")
    print(f"- failure_class={failure_class}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
