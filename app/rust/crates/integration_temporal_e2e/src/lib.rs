//! Independent canonical Temporal and genuine all-provider E2E owner.
//! The broad harness only supplies its real runtime/environment fixture API;
//! its legacy `#[cfg(test)]` suite is not a transitive Cargo target.

#[cfg(feature = "e2e")]
pub use integration_harness::{env_overrides, stub_servers, IntegrationCtx};
#[cfg(feature = "e2e")]
pub use integration_harness::containers::{
    InfraHarness, Neo4jHarness, PostgresHarness, QdrantHarness, TemporalHarness,
};

#[cfg(all(test, feature = "e2e"))]
#[allow(unused, warnings)]
mod tests {
    // Single-source shared E2E support. Cargo compiles these helper bytes into
    // each opt-in owner; only this crate owns the five selected test bodies.
    include!("../../integration_harness/src/tests/preamble.rs");
    include!("../../integration_harness/src/tests/truth_certification_helpers.rs");
    include!("../../integration_harness/src/tests/synthetic_runtime_helpers.rs");
    include!("../../integration_harness/src/tests/pure_dag_temporal_helpers.rs");

    include!("tests/manual_llm_temporal_tests.rs");
    include!("tests/pure_v3_temporal_tests.rs");
    include!("tests/scenario_parity_tests.rs");
    include!("tests/canonical_temporal_wire_shape_tests.rs");
}
