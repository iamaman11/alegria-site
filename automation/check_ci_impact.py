_svc": set(),
            "temporal_worker": set(),
            "outbox_worker": set(),
        }

    def cargo_metadata(self) -> dict:
        # Cargo metadata's target kinds, not a second manually maintained
        # Cargo dependency graph, decide what can produce reusable rlibs.
        binary_only = {"cli_tools", "gsc_sync", "temporal_worker", "analytics_svc"}
        packages = [
            {
                "id": f"workspace#{name}",
                "name": name,
                "targets": [{"kind": ["bin" if name in binary_only else "lib"]}],
            }
            for name in self.owners.values()
        ]
        return {
            "workspace_members": [pkg["id"] for pkg in packages],
            "packages": packages,
        }

    def impact(self, *paths: str, lock_snapshots: tuple[str, str] | None = None) -> dict:
        return analyze(
            list(paths), self.owners, self.reverse, head="HEAD", base="BASE",
            lock_snapshots=lock_snapshots,
        )

    def test_ci_only_main_merge_is_full_pr_but_not_a_new_worker_image(self) -> None:
        paths = [".github/workflows/ci.yml", "automation/ci_impact.py", "automation/check_ci_impact.py"]
        report = self.impact(*paths)
        self.assertEqual(report["scope"], "full")
        self.assertEqual(set(report["selected_ci_jobs"]), set(CI_PR_OWNERS))
        self.assertIn("temporal-worker", report["selected_images"])
        decision = worker_release_decision(report, paths, manual=False)
        self.assertFalse(decision["worker_required"])
        self.assertEqual(decision["reason"], "ci_operator_maintenance_only")

    def test_compiler_cache_and_feedback_tools_do_not_ship_into_worker(self) -> None:
        # A change to compiler orchestration still receives the full PR gate:
        # the release graph must not confuse it with a delivered Worker input.
        for path in (
            "automation/run_main_cache_prime.py",
            "automation/run_scoped_cargo_feedback.py",
            "automation/cargo_feedback.py",
            "automation/dev_rust_feedback.py",
            "automation/check_ci_impact.py",
        ):
            with self.subTest(path=path):
                report = self.impact(path)
                decision = worker_release_decision(report, [path], manual=False)
                self.assertFalse(decision["worker_required"])


if __name__ == "__main__":
    unittest.main()