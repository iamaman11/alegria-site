#!/usr/bin/env python3
"""Read-only first-site diagnostic. Runs on native Windows Python, no WSL.

Report configured presence separately from live connectivity and public proof.
Never emits environment values or provider response bodies.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request


SERVICES = (
    "alegria_postgres", "alegria_postgres_temporal", "alegria_pgbouncer",
    "alegria_temporal", "alegria_neo4j", "alegria_qdrant",
    "alegria_outbox_worker", "alegria_temporal_worker",
)
REQUIRED = ("DATAFORSEO_LOGIN", "DATAFORSEO_PASSWORD", "VOYAGE_API_KEY",
            "CLOUDFLARE_API_TOKEN")
ACCOUNT_ID = "78e5fcb3762c1473113d51838556da74"
EXPECTED_ORIGIN = "https://alegria-seo-staging.alegria-site.workers.dev"


def command(args: list[str], *, timeout: int = 10) -> tuple[bool, str]:
    try:
        result = subprocess.run(
            args, capture_output=True, text=True, timeout=timeout,
            check=False, encoding="utf-8", errors="replace",
        )
    except (OSError, subprocess.TimeoutExpired):
        return False, ""
    return result.returncode == 0, result.stdout[:200_000] if result.returncode == 0 else ""


def docker(args: list[str], *, timeout: int = 10) -> tuple[bool, str]:
    return command(["docker", *args], timeout=timeout)


def worker_env() -> tuple[dict[str, str], str]:
    ok, output = docker([
        "inspect", "alegria_temporal_worker", "--format", "{{json .Config.Env}}",
    ])
    if not ok:
        return {}, "UNAVAILABLE"
    try:
        values = json.loads(output)
        if not isinstance(values, list):
            return {}, "INVALID"
        parsed = dict(entry.split("=", 1) for entry in values if isinstance(entry, str) and "=" in entry)
        return parsed, "READ"
    except (TypeError, ValueError):
        return {}, "INVALID"


def cloudflare_http() -> str:
    # The Bearer value stays entirely inside the existing worker. Output is
    # an HTTP status *only*, never the token or the response body.
    probe = (
        "import os,urllib.request,urllib.error;"
        "t=os.environ.get('CLOUDFLARE_API_TOKEN','');"
        "u='https://api.cloudflare.com/client/v4/accounts/"
        + ACCOUNT_ID +
        "/workers/scripts';"
        "r=urllib.request.Request(u,headers={'Authorization':'Bearer '+t});"
        "\ntry:\n"
        " with urllib.request.urlopen(r,timeout=8) as response: print('HTTP_'+str(response.status))\n"
        "except urllib.error.HTTPError as exc: print('HTTP_'+str(exc.code))\n"
        "except Exception: print('UNAVAILABLE')\n"
    )
    ok, stdout = docker(["exec", "alegria_temporal_worker", "python3", "-c", probe], timeout=13)
    value = stdout.strip()
    return value if ok and (value == "UNAVAILABLE" or (
        value.startswith("HTTP_") and value[5:].isdigit()
    )) else "UNAVAILABLE"


def http_code(url: str) -> str:
    try:
        request = urllib.request.Request(
            url, method="HEAD", headers={"User-Agent": "Alegria-Operator-Doctor/1"}
        )
        with urllib.request.urlopen(request, timeout=7) as response:
            return str(response.status)
    except urllib.error.HTTPError as error:
        return str(error.code)
    except (OSError, ValueError):
        return "UNAVAILABLE"


def run() -> dict:
    services: dict[str, dict[str, str]] = {}
    for name in SERVICES:
        ok, body = docker(["inspect", name, "--format", "{{json .State}}"])
        try:
            state = json.loads(body) if ok else {}
        except (ValueError, TypeError):
            state = {}
        health = state.get("Health") if isinstance(state, dict) else None
        services[name] = {
            "state": str(state.get("Status", "UNKNOWN")) if isinstance(state, dict) else "UNKNOWN",
            "health": str(health.get("Status", "UNKNOWN")) if isinstance(health, dict) else "NOT_CONFIGURED",
        }

    env, env_read = worker_env()
    providers = {
        name: {
            "operator_configured": bool(os.environ.get(name)),
            "worker_configured": bool(env.get(name)),
            "capability": "NOT_TESTED",
        } for name in REQUIRED
    }
    providers["CLOUDFLARE_API_TOKEN"]["capability"] = (
        cloudflare_http() if env.get("CLOUDFLARE_API_TOKEN") else "NOT_CONFIGURED"
    )

    db_ok, db_ping = docker([
        "exec", "alegria_postgres", "pg_isready", "-U", "postgres", "-d", "alegria",
    ])
    db_ready = db_ok and "accepting connections" in db_ping
    query = (
        "SELECT (SELECT count(*) FROM pipeline.execution_runs)::text || ',' || "
        "(SELECT count(*) FROM pipeline.manual_llm_jobs WHERE job_status='pending')::text || ',' || "
        "(SELECT count(*) FROM site.seo_hitl_tasks WHERE queue_state IN "
        "('open','in_review','reopened'))::text"
    )
    counts_ok, counts_raw = docker([
        "exec", "alegria_postgres", "psql", "-X", "-U", "postgres", "-d", "alegria",
        "-A", "-t", "-c", query,
    ]) if db_ready else (False, "")
    counts = None
    if counts_ok:
        values = counts_raw.strip().split(",")
        if len(values) == 3 and all(value.isdigit() for value in values):
            counts = dict(zip(("run_count", "pending_manual_llm", "pending_cms_hitl"),
                              map(int, values)))

    temporal_ok, temporal_output = docker([
        "exec", "alegria_temporal_worker", "temporal_starter", "--temporal-url",
        "http://temporal-server:7233", "ping",
    ])
    temporal_ready = temporal_ok and temporal_output.startswith("ok temporal_url=")

    gds_ok, gds_output = docker([
        "exec", "alegria_neo4j", "sh", "-ec",
        'cypher-shell -u neo4j -p "${NEO4J_AUTH#*/}" "RETURN gds.version() AS version"',
    ])
    gds_ready = gds_ok and "version" in gds_output and "Error" not in gds_output

    try:
        with urllib.request.urlopen("http://127.0.0.1:6333/collections", timeout=5) as response:
            qdrant_result = json.load(response)
        collections = qdrant_result.get("result", {}).get("collections", [])
        collection_count = len(collections) if isinstance(collections, list) else None
    except (OSError, ValueError, TypeError, KeyError):
        collection_count = None

    metrics_status = http_code("http://127.0.0.1:9464/metrics")
    origin = env.get("SEO_STATIC_BASE_URL") or EXPECTED_ORIGIN
    public_http = http_code(origin) if origin == EXPECTED_ORIGIN else "NOT_CHECKED"
    expected_sha = os.environ.get("ALEGRIA_DOCTOR_EXPECTED_SHA", "")
    actual_sha = env.get("WORKER_BUILD_ID", "")
    actual_sha_valid = len(actual_sha) == 40 and all(c in "0123456789abcdef" for c in actual_sha.lower())
    source_matches = not expected_sha or actual_sha == expected_sha

    infra_ok = (
        all(item["state"] == "running" and item["health"] != "unhealthy"
            for item in services.values())
        and db_ready and counts is not None and temporal_ready and gds_ready
        and collection_count is not None and metrics_status == "200"
    )
    configured = all(bool(env.get(key)) and bool(os.environ.get(key)) for key in REQUIRED) and (
        env.get("SEO_LLM_PROVIDER") == os.environ.get("SEO_LLM_PROVIDER") == "chat_manual"
        and env.get("SEO_TRUTH_LLM_PROVIDER") == os.environ.get("SEO_TRUTH_LLM_PROVIDER") == "chat_manual"
        and env.get("CLOUDFLARE_ACCOUNT_ID") == ACCOUNT_ID
        and origin == EXPECTED_ORIGIN
        and actual_sha_valid and source_matches
    )
    cf_auth = providers["CLOUDFLARE_API_TOKEN"]["capability"] == "HTTP_200"
    status = (
        "RUNTIME_BLOCKED" if not infra_ok else
        "PROVIDER_CONFIG_BLOCKED" if not configured or not cf_auth else
        "CONFIG_PRESENT_UNVERIFIED"
    )
    return {
        "doctor": "alegria_first_site_read_only_v1",
        "status": status,
        "source": {
            "worker_build_id": actual_sha if actual_sha_valid else "UNKNOWN",
            "expected_sha_match": source_matches if expected_sha else "NOT_REQUESTED",
        },
        "services": services,
        "database": {"reachable": db_ready, "counts": counts},
        "temporal": {"ping": "PASS" if temporal_ready else "FAIL"},
        "graph": {"gds": "PASS" if gds_ready else "FAIL"},
        "retrieval": {"qdrant_collections": collection_count},
        "worker": {
            "env_read": env_read, "llm_provider": env.get("SEO_LLM_PROVIDER", "UNKNOWN"),
            "truth_provider": env.get("SEO_TRUTH_LLM_PROVIDER", "UNKNOWN"),
        },
        "providers": providers,
        "public": {
            "expected_origin": EXPECTED_ORIGIN,
            "http_status": public_http,
            "independent_release_hash": "NOT_TESTED",
            "publication_proven": False,
        },
        "metrics_http": metrics_status,
        "next_step": (
            "RESTORE_RUNTIME" if not infra_ok else
            "CONFIGURE_AND_VERIFY_PROVIDERS" if not configured or not cf_auth else
            "START_REAL_CANONICAL_FIRST_SITE_AND_VERIFY_PUBLIC_BYTES"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", action="store_true", help="machine-readable safe diagnostics")
    parser.add_argument("--expected-sha", default="", help="optional installed build SHA")
    args = parser.parse_args()
    if args.expected_sha:
        os.environ["ALEGRIA_DOCTOR_EXPECTED_SHA"] = args.expected_sha
    report = run()
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if report["status"] == "CONFIG_PRESENT_UNVERIFIED" else 2


if __name__ == "__main__":
    sys.exit(main())
