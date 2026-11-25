#!/usr/bin/env python3
"""No-network regression tests for honest production topology acceptance."""

import unittest

from check_production_boundary_v5 import check, service_sections


EXPERIMENTS = """services:
  temporal-server:
    networks:
      - default
      - temporal_pure_control
  temporal-orchestrator-v3:
    profiles: ["v3-evaluation"]
    environment:
      TEMPORAL_URL: http://temporal-server:7233
    depends_on:
      temporal-server:
        condition: service_healthy
    networks:
      - temporal_pure_control
  temporal-effect-dag-v4:
    profiles: ["v4-effect-evaluation"]
    environment:
      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v4
    depends_on:
      temporal-server:
        condition: service_healthy
    networks:
      - temporal_pure_control
  temporal-effect-dag-v5:
    profiles: ["v5-effect-evaluation"]
    environment:
      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v5
    depends_on:
      temporal-server:
        condition: service_healthy
    networks:
      - temporal_pure_control
  temporal-pg-effects-v3:
    profiles: ["v3-evaluation"]
    environment:
      PG_EFFECT_WORKER_DATABASE_URL_FILE: /run/secrets/pg_effect_worker_dsn
"""
NETWORKS = """networks:
  temporal_pure_control:
    internal: true
"""


def fixture(extra: str = "") -> str:
    return EXPERIMENTS + extra + NETWORKS


class ProductionBoundaryTests(unittest.TestCase):
    def test_experimental_profile_does_not_count_as_production_owner(self) -> None:
        report = check(fixture())
        self.assertEqual(report["experimental_boundary"], "PASS")
        self.assertEqual(report["production_static_boundary"], "NO_GO")
        self.assertFalse(report["production_release_authorized"])

    def test_default_legacy_worker_with_pg_graph_vector_tokens_is_no_go(self) -> None:
        compose = fixture("""  temporal-worker:
    environment:
      DATABASE_URL: postgres://postgres:password@postgres/db
      NEO4J_URI: neo4j:7687
      QDRANT_URL: http://qdrant:6334
      OPENAI_API_KEY: secret
""")
        report = check(compose)
        self.assertEqual(report["experimental_boundary"], "PASS")
        self.assertEqual(report["production_static_boundary"], "NO_GO")
        self.assertTrue(any("temporal-worker" in problem for problem in report["issues"]))

    def test_default_separated_parent_and_pg_owner_pass_static_only(self) -> None:
        compose = fixture("""  temporal-orchestrator-production:
    environment:
      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v4
      TEMPORAL_URL: http://temporal-server:7233
    depends_on:
      temporal-server:
        condition: service_healthy
    networks:
      - temporal_pure_control
  temporal-pg-effects-production:
    environment:
      PG_EFFECT_WORKER_DATABASE_URL_FILE: /run/secrets/pg_effect_worker_dsn
""")
        report = check(compose)
        self.assertEqual(report["production_static_boundary"], "PASS")
        self.assertFalse(report["production_release_authorized"])
        self.assertNotIn("temporal_pure_control", service_sections(compose))

    def test_default_separated_parent_cannot_inherit_provider_secret(self) -> None:
        compose = fixture("""  temporal-orchestrator-production:
    environment:
      TEMPORAL_ORCHESTRATOR_TASK_QUEUE: alegria-effect-dag-v4
      CLOUDFLARE_API_TOKEN: token
    depends_on:
      temporal-server:
        condition: service_healthy
    networks:
      - temporal_pure_control
  temporal-pg-effects-production:
    environment:
      PG_EFFECT_WORKER_DATABASE_URL_FILE: /run/secrets/pg_effect_worker_dsn
""")
        self.assertEqual(check(compose)["production_static_boundary"], "NO_GO")

    def test_experimental_dsn_or_profile_drift_fails_instead_of_masking(self) -> None:
        drifted = fixture().replace(
            "      PG_EFFECT_WORKER_DATABASE_URL_FILE: /run/secrets/pg_effect_worker_dsn",
            "      DATABASE_URL: postgres://postgres:password@postgres/db",
        )
        self.assertEqual(check(drifted)["experimental_boundary"], "FAIL")
        drifted = fixture().replace('    profiles: ["v4-effect-evaluation"]\n', "", 1)
        self.assertEqual(check(drifted)["experimental_boundary"], "FAIL")

    def test_pure_parent_network_or_dependency_escape_is_rejected(self) -> None:
        wrong_net = fixture().replace(
            "      - temporal_pure_control\n  temporal-pg-effects-v3:",
            "      - default\n  temporal-pg-effects-v3:",
        )
        self.assertEqual(check(wrong_net)["experimental_boundary"], "FAIL")
        wrong_dep = fixture().replace(
            "      temporal-server:\n        condition: service_healthy\n"
            "    networks:\n      - temporal_pure_control\n  temporal-pg-effects-v3:",
            "      postgres:\n        condition: service_healthy\n"
            "    networks:\n      - temporal_pure_control\n  temporal-pg-effects-v3:",
        )
        self.assertEqual(check(wrong_dep)["experimental_boundary"], "FAIL")

    def test_provider_must_not_join_pure_network(self) -> None:
        unsafe = fixture("""  qdrant:
    networks:
      - default
      - temporal_pure_control
""")
        report = check(unsafe)
        self.assertEqual(report["experimental_boundary"], "FAIL")
        self.assertTrue(any("qdrant" in problem for problem in report["issues"]))


if __name__ == "__main__":
    unittest.main()
