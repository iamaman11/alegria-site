//! Independent real Qdrant/PG integration owner for A13 vector readbacks and
//! the REST-projection-to-gRPC durability boundary. No fake vector store.

#[cfg(test)]
#[allow(unused, warnings)]
mod tests {
    #[cfg(feature = "e2e")]
    use integration_data_fixtures::{PostgresHarness, QdrantHarness};
    #[cfg(feature = "e2e")]
    use integration_harness::{env_overrides, stub_servers};
    #[cfg(feature = "e2e")]
    use runtime_composition::adapters::seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository;
    #[cfg(feature = "e2e")]
    use serial_test::serial;
    #[cfg(feature = "e2e")]
    use uuid::Uuid;

    include!("tests/knowledge_read_tests.rs");
    include!("tests/qdrant_rest_projection_tests.rs");
}
