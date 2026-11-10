#!/usr/bin/env python3
"""Credential-free behavioral tests of the native Windows first-site doctor."""
from __future__ import annotations

import json
import os
import subprocess
import unittest
from unittest import mock

import first_site_doctor as doctor


SHA = "a" * 40
SECRET = "do-not-print-operator-or-worker-secret"
FLAGS = {
    "DATAFORSEO_LOGIN": SECRET + "-login",
    "DATAFORSEO_PASSWORD": SECRET + "-password",
    "VOYAGE_API_KEY": SECRET + "-voyage",
    "CLOUDFLARE_API_TOKEN": SECRET + "-cloudflare",
    "SEO_LLM_PROVIDER": "chat_manual",
    "SEO_TRUTH_LLM_PROVIDER": "chat_manual",
    "ALEGRIA_DOCTOR_EXPECTED_SHA": SHA,
}
WORKER = {
    **FLAGS,
    "WORKER_BUILD_ID": SHA,
    "CLOUDFLARE_ACCOUNT_ID": doctor.ACCOUNT_ID,
    "SEO_STATIC_BASE_URL": doctor.EXPECTED_ORIGIN,
}


class FakeResponse:
    def __init__(self, body: bytes = b'{"result":{"collections":[]}}'):
        self.body = body
        self.status = 200

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self, *_args):
        return self.body


def simulate_docker(
    args: list[str], *, timeout: int = 10, cf_http: int = 200,
    broken_runtime: bool = False, worker: dict[str, str] | None = None,
    stdin: str | None = None,
) -> tuple[bool, str]:
    del timeout, stdin
    worker = WORKER if worker is None else worker
    assert args[0] in {"inspect", "exec"}, "doctor must not mutate Docker"
    if args[0] == "inspect" and args[2] == "--format":
        if ".Config.Env" in args[3]:
            return True, json.dumps([f"{key}={value}" for key, value in worker.items()])
        state = {"Status": "running", "Health": {"Status": "healthy"}}
        if broken_runtime and args[1] == "alegria_postgres":
            state["Status"] = "exited"
        return True, json.dumps(state)
    if "pg_isready" in args:
        return True, "localhost:5432 - accepting connections"
    if "psql" in args:
        return True, "0,0,0"
    if "temporal_starter" in args:
        return True, "ok temporal_url=http://temporal-server:7233 namespace=default"
    if "cypher-shell" in " ".join(args):
        return True, "version\n2026.10.1"
    if "python3" in args:
        return True, f"HTTP_{cf_http}"
    raise AssertionError(f"unexpected read-only probe: {args!r}")


class DoctorTests(unittest.TestCase):
    def perform(
        self, *, cf_http: int = 200, broken_runtime: bool = False,
        operator: dict[str, str] | None = None,
        worker: dict[str, str] | None = None,
    ) -> dict:
        operator = FLAGS if operator is None else operator
        observed: list[list[str]] = []

        def fake(
            args: list[str], *, timeout: int = 10, stdin: str | None = None,
        ) -> tuple[bool, str]:
            observed.append(args)
            return simulate_docker(
                args, timeout=timeout, cf_http=cf_http,
                broken_runtime=broken_runtime, worker=worker, stdin=stdin,
            )

        with (
            mock.patch.dict(os.environ, operator, clear=True),
            mock.patch.object(doctor, "docker", side_effect=fake),
            mock.patch("urllib.request.urlopen", return_value=FakeResponse()),
            mock.patch.object(doctor, "http_code", side_effect=lambda url: (
                "200" if ":9464/" in url else "404"
            )),
        ):
            report = doctor.run()
        self.assertTrue(all(cmd[0] in {"inspect", "exec"} for cmd in observed))
        self.assertNotIn(SECRET, json.dumps(report))
        return report

    def test_configured_is_not_a_false_published_site(self) -> None:
        report = self.perform()
        self.assertEqual(report["status"], "CONFIG_PRESENT_UNVERIFIED")
        self.assertEqual(report["source"]["expected_sha_match"], True)
        self.assertFalse(report["public"]["publication_proven"])
        self.assertEqual(report["public"]["independent_release_hash"], "NOT_TESTED")
        self.assertEqual(report["database"]["counts"]["run_count"], 0)
        self.assertEqual(report["retrieval"]["qdrant_collections"], 0)

    def test_missing_worker_voyage_blocks(self) -> None:
        report = self.perform(worker={**WORKER, "VOYAGE_API_KEY": ""})
        self.assertEqual(report["status"], "PROVIDER_CONFIG_BLOCKED")
        self.assertFalse(report["providers"]["VOYAGE_API_KEY"]["worker_configured"])

    def test_missing_operator_credentials_blocks_even_if_worker_has_them(self) -> None:
        report = self.perform(operator={**FLAGS, "DATAFORSEO_PASSWORD": ""})
        self.assertEqual(report["status"], "PROVIDER_CONFIG_BLOCKED")

    def test_unauthorized_cloudflare_blocks(self) -> None:
        report = self.perform(cf_http=401)
        self.assertEqual(report["status"], "PROVIDER_CONFIG_BLOCKED")
        self.assertEqual(report["providers"]["CLOUDFLARE_API_TOKEN"]["capability"], "HTTP_401")

    def test_stopped_database_blocks(self) -> None:
        report = self.perform(broken_runtime=True)
        self.assertEqual(report["status"], "RUNTIME_BLOCKED")

    def test_subprocess_timeout_fails_closed_without_leaking_stderr(self) -> None:
        with mock.patch("subprocess.run", side_effect=subprocess.TimeoutExpired("docker", 1)):
            ok, stdout = doctor.command(["docker", "inspect", "a"])
        self.assertFalse(ok)
        self.assertEqual(stdout, "")


if __name__ == "__main__":
    unittest.main()
