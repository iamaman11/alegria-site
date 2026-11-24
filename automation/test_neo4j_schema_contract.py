#!/usr/bin/env python3
"""Mutation-test the Neo4j schema fence without pretending to run Bolt E2E.

The existing mandatory graph_query suite executes the real Neo4j container
tests. These fast source mutations only guarantee that disconnecting that
suite or its physical uniqueness preconditions fails CI.
"""

from __future__ import annotations

import unittest

from check_reliability_contracts import (
    CI_IMPACT_PLANNER, CI_WORKFLOW, NEO4J_CONNECTION_OWNER,
    NEO4J_LOCAL_GRAPH_GATE, NEO4J_OPERATOR_DISPATCH, NEO4J_OPERATOR_TYPES,
    NEO4J_REAL_E2E, NEO4J_REAL_HARNESS, NEO4J_SCHEMA_OWNER,
    neo4j_schema_contract_failures,
)


SOURCES = (
    NEO4J_SCHEMA_OWNER, NEO4J_CONNECTION_OWNER, NEO4J_REAL_HARNESS,
    NEO4J_REAL_E2E, CI_IMPACT_PLANNER, CI_WORKFLOW,
    NEO4J_OPERATOR_TYPES, NEO4J_OPERATOR_DISPATCH,
    NEO4J_LOCAL_GRAPH_GATE,
)


class Neo4jSchemaSourceGuardTests(unittest.TestCase):
    def sources(self) -> list[str]:
        return [source.read_text(encoding="utf-8") for source in SOURCES]

    def test_current_repository_has_real_schema_and_e2e_gates(self) -> None:
        self.assertEqual(neo4j_schema_contract_failures(*self.sources()), [])

    def test_removing_any_critical_uniqueness_boundary_fails(self) -> None:
        sources = self.sources()
        mutations = (
            # Typed Neo4j physical identities, including both relationships.
            (0, 'label_or_type: "PageNode"', 'label_or_type: "UnprotectedNode"'),
            (0, 'label_or_type: "RECOMMENDS_LINK_TO"', 'label_or_type: "BROKEN_LINK"'),
            (0, 'label_or_type: "CONFLICTS_WITH"', 'label_or_type: "BROKEN_CONFLICT"'),
            (0, 'REQUIRE r.link_recommendation_key IS UNIQUE', 'REQUIRE r.link_recommendation_key IS NOT NULL'),
            # Verifier must check the *actual* installed metadata before reuse.
            (0, 'found.properties[0] == spec.property', 'found.properties[0] != spec.property'),
            (0, 'SHOW CONSTRAINTS YIELD', 'RETURN 1 AS schema_is_fine'),
            (1, 'schema::verify_projection_schema(&graph).await?', 'Ok(()) /* bypassed */'),
            # Fixture setup is real DDL on a disposable, real Neo4j instance.
            (2, 'graph_neo4j::schema::install_projection_schema(&graph)',
             'graph_neo4j::schema::verify_projection_schema(&graph)'),
            # Concurrent replay must continue to run via the existing graph suite.
            (3, 'tokio::join!(', 'tokio::try_join!('),
            (3, 'assert_eq!(count_links(&graph, &concurrent_link_key).await, 1)',
             'assert_eq!(count_links(&graph, &concurrent_link_key).await, 2)'),
            (4, '"--lib", "graph_capability_", "--locked"',
             '"--lib", "neo4j_probe_only", "--locked"'),
            (5, 'python3 automation/check_reliability_contracts.py',
             'python3 automation/check_unsafe_contracts.py'),
            (6, 'Neo4jSchemaInstall,', 'SkipNeo4jSchemaInstall,'),
            (7, 'neo4rs_adapter::schema::install_projection_schema(&graph).await?',
             'neo4rs_adapter::schema::verify_projection_schema(&graph).await?'),
            (8, 'LOCAL_NEO4J_BOOTSTRAPPED=false', 'LOCAL_NEO4J_BOOTSTRAPPED=true'),
            (8, 'if [[ "$LOCAL_NEO4J_BOOTSTRAPPED" == "true" ]]; then', 'if true; then'),
            (8, 'NEO4J_URI="127.0.0.1:7687"', 'NEO4J_URI="${NEO4J_URI:-127.0.0.1:7687}"'),
            (8, '-- neo4j-schema-install', '-- neo4j-schema-skip'),
        )
        for index, old, new in mutations:
            with self.subTest(source=SOURCES[index].name, removed=old):
                self.assertIn(old, sources[index])
                changed = sources.copy()
                changed[index] = changed[index].replace(old, new, 1)
                self.assertTrue(
                    neo4j_schema_contract_failures(*changed),
                    f"source mutation went unnoticed: {SOURCES[index].name}: {old}",
                )


if __name__ == "__main__":
    unittest.main()
