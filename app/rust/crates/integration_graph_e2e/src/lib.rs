//! Independent real Neo4j schema, scope, link and capability regression owner.
//! Exactly two mandatory graph_capability_ tests are executed by its CI shard.

#[cfg(all(test, feature = "e2e"))]
#[allow(unused, warnings)]
mod tests {
    use integration_graph_fixtures::Neo4jHarness;
    use integration_test_env as env_overrides;
    use serial_test::serial;
    use uuid::Uuid;

    include!("tests/graph_capability_link_projection_tests.rs");
    include!("tests/graph_capability_probe_tests.rs");
}
