#!/usr/bin/env python3
"""First-site milestone is separate from the three-scenario R5 certificate."""
from __future__ import annotations

from io import StringIO
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

import e2e_supersite_certification as cert


TEST_ENV = {
    "DATAFORSEO_LOGIN": "fixture",
    "DATAFORSEO_PASSWORD": "fixture",
    "SEO_LLM_PROVIDER": "openai",
    "OPENAI_API_KEY": "fixture",
    "ALEGRIA_CERT_PUBLISH": "1",
    "CLOUDFLARE_ACCOUNT_ID": "78e5fcb3762c1473113d51838556da74",
    "ALEGRIA_CERT_BASE_URL": "https://alegria-seo-staging.alegria-site.workers.dev",
    "SEO_STATIC_BASE_URL": "https://alegria-seo-staging.alegria-site.workers.dev",
    "ALEGRIA_CERT_PUBLIC_OUTPUT_DIR": "/tmp/fixture-approved-release",
}


def fake_scenario(**kwargs: object) -> dict:
    scenario = str(kwargs["scenario_id"])
    return {
        "artifact_id": "r5_" + scenario,
        "title": str(kwargs["title"]),
        "scenario_id": scenario,
        "evidence_status": "PASS",
        "failure_class": "none",
        "run_id": "fixture-run",
        "workflow_id": "fixture-run",
        "report_path": "docs/runs/fixture.json",
    }


class FirstSiteAcceptanceTests(unittest.TestCase):
    def run_case(
        self, *, first_site: bool, codes: tuple[int, ...] = (0, 0, 0),
        scenario_failure: bool = False,
        env_overrides: dict[str, str] | None = None,
    ) -> tuple[int, dict, object, object]:
        reports: list[tuple[Path, dict]] = []

        def write_json(path: Path, value: dict) -> None:
            reports.append((path, value.copy()))

        def scenario(**kwargs: object) -> dict:
            result = fake_scenario(**kwargs)
            if scenario_failure:
                result["evidence_status"] = "FAIL"
                result["failure_class"] = "real_source_blocked"
            return result

        argv = ["certification", "--first-site"] if first_site else ["certification"]
        with (
            mock.patch.dict("os.environ", {**TEST_ENV, **(env_overrides or {})}, clear=True),
            mock.patch.object(sys, "argv", argv),
            mock.patch.object(cert, "classify_provider", return_value=("openai", "openai")),
            mock.patch.object(cert, "clean_runtime_preflight", return_value={
                "worker": {"status": "PASS"},
            }),
            mock.patch.object(cert, "scenario_payload", side_effect=scenario) as runner,
            mock.patch.object(cert, "run_command", side_effect=[
                subprocess.CompletedProcess(["check"], code) for code in codes
            ]) as command,
            mock.patch.object(cert, "write_json", side_effect=write_json),
            mock.patch("builtins.print"),
        ):
            status = cert.main()
        self.assertEqual(len(reports), 1)
        report_path, manifest = reports[0]
        if first_site:
            self.assertTrue(report_path.name.startswith("first_site_certification_"))
            self.assertNotEqual(report_path, cert.MANIFEST)
        else:
            self.assertEqual(report_path, cert.MANIFEST)
        return status, manifest, runner, command

    def test_first_site_runs_only_canonical_first_scenario_and_https_check(self) -> None:
        code, manifest, runner, command = self.run_case(first_site=True)
        self.assertEqual(code, 0)
        self.assertEqual(runner.call_count, 1)
        self.assertEqual(runner.call_args.kwargs["scenario_id"], "fresh_scope")
        self.assertEqual(manifest["certification_scope"], "first_site_only")
        self.assertEqual(manifest["evidence_status"], "PASS")
        self.assertEqual(manifest["public_acceptance"], {
            "run_release_provenance": "PASS", "static_output": "PASS",
            "independent_https_readback": "PASS",
        })
        self.assertEqual(command.call_count, 3)
        self.assertIn("check_public_http_readback.py", " ".join(command.call_args.args[0]))
        self.assertIn(
            "check_first_site_release.py", " ".join(command.call_args_list[0].args[0])
        )
        self.assertIn("fixture-run", command.call_args_list[0].args[0])

    def test_first_site_blocks_failed_remote_bytes(self) -> None:
        code, manifest, runner, command = self.run_case(first_site=True, codes=(0, 0, 1))
        self.assertEqual(code, 1)
        self.assertEqual(runner.call_count, 1)
        self.assertEqual(command.call_count, 3)
        self.assertEqual(manifest["evidence_status"], "FAIL")
        self.assertEqual(manifest["failure_class"], "public_acceptance_failure")

    def test_failed_real_sources_never_attempt_https_pass(self) -> None:
        code, manifest, runner, command = self.run_case(
            first_site=True, scenario_failure=True
        )
        self.assertEqual(code, 1)
        self.assertEqual(runner.call_count, 1)
        command.assert_not_called()
        self.assertEqual(manifest["failure_class"], "real_source_blocked")

    def test_full_r5_still_requires_all_three_scenarios(self) -> None:
        code, manifest, runner, command = self.run_case(first_site=False)
        self.assertEqual(code, 0)
        self.assertEqual(manifest["certification_scope"], "full_r5")
        self.assertEqual(runner.call_count, 3)
        self.assertEqual(
            [call.kwargs["scenario_id"] for call in runner.call_args_list],
            ["fresh_scope", "scope_expansion", "factual_change_rebuild"],
        )
        command.assert_not_called()

    def test_wrong_cloudflare_account_blocks_before_launch(self) -> None:
        code, manifest, runner, command = self.run_case(
            first_site=True, env_overrides={"CLOUDFLARE_ACCOUNT_ID": "foreign-account"}
        )
        self.assertEqual(code, 2)
        self.assertIn("dedicated_cloudflare_account", manifest["missing"])
        runner.assert_not_called()
        command.assert_not_called()

    def test_previous_technical_origin_blocks_before_launch(self) -> None:
        previous_origin = "https://seo-staging.alegria.by"
        code, manifest, runner, command = self.run_case(
            first_site=True,
            env_overrides={
                "ALEGRIA_CERT_BASE_URL": previous_origin,
                "SEO_STATIC_BASE_URL": previous_origin,
            },
        )
        self.assertEqual(code, 2)
        self.assertIn("dedicated_workers_dev_origin", manifest["missing"])
        runner.assert_not_called()
        command.assert_not_called()

    def test_dry_run_does_not_print_database_credentials(self) -> None:
        env = dict(TEST_ENV, DATABASE_URL="postgres://user:topsecret@host/db")
        output = StringIO()
        with (
            mock.patch.dict("os.environ", env, clear=True),
            mock.patch.object(sys, "argv", ["certification", "--first-site", "--dry-run"]),
            mock.patch("sys.stdout", output),
            mock.patch.object(cert, "classify_provider", return_value=("openai", "openai")),
        ):
            self.assertEqual(cert.main(), 0)
        self.assertNotIn("topsecret", output.getvalue())
        self.assertIn("fresh_scope", output.getvalue())
        self.assertIn("- configuration=READY", output.getvalue())
        self.assertIn("- missing=none", output.getvalue())

    def test_dry_run_reports_missing_configuration_without_actions_or_secrets(self) -> None:
        env = dict(TEST_ENV)
        env.pop("DATAFORSEO_PASSWORD")
        env["DATABASE_URL"] = "postgres://user:hidden-password@host/db"
        output = StringIO()
        with (
            mock.patch.dict("os.environ", env, clear=True),
            mock.patch.object(sys, "argv", ["certification", "--first-site", "--dry-run"]),
            mock.patch("sys.stdout", output),
            mock.patch.object(cert, "run_command") as run_command,
            mock.patch.object(cert, "write_json") as write_json,
        ):
            self.assertEqual(cert.main(), 2)
        self.assertIn("- configuration=BLOCKED", output.getvalue())
        self.assertIn("DATAFORSEO_PASSWORD", output.getvalue())
        self.assertNotIn("hidden-password", output.getvalue())
        self.assertNotIn("fixture", output.getvalue())
        run_command.assert_not_called()
        write_json.assert_not_called()

    def test_chat_manual_with_stray_api_key_cannot_pass_unattended_gate(self) -> None:
        env = dict(TEST_ENV, SEO_LLM_PROVIDER="chat_manual")
        output = StringIO()
        with (
            mock.patch.dict("os.environ", env, clear=True),
            mock.patch.object(sys, "argv", ["certification", "--first-site", "--dry-run"]),
            mock.patch("sys.stdout", output),
        ):
            self.assertEqual(cert.main(), 2)
        self.assertIn("- provider_kind=chat_manual", output.getvalue())
        self.assertIn("production_llm_provider", output.getvalue())
        self.assertIn("requires llm-next/llm-submit", output.getvalue())

    def test_preview_requirements_equal_real_launch_requirements(self) -> None:
        env = dict(TEST_ENV)
        env.pop("DATAFORSEO_LOGIN")
        with mock.patch.dict("os.environ", env, clear=True):
            live, kind = cert.classify_provider()
            self.assertEqual(kind, "openai")
            missing = [
                key for key, present in cert.certification_requirements(live).items()
                if not present
            ]
        self.assertEqual(missing, ["DATAFORSEO_LOGIN"])


if __name__ == "__main__":
    unittest.main()
