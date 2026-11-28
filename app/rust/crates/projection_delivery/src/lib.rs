//! The only effectful projection delivery implementation. PostgreSQL owns
//! Outbox leases/fencing, Temporal observes durable completion, and the
//! standalone Outbox Worker invokes this crate's typed leased dispatcher.
//!
//! Deliberately independent of `runtime_composition`, `infrastructure`,
//! Temporal, Playwright and the broader SEO application graph.

pub mod admitted_qdrant_projection_v1;
pub mod neo4j_materialization_adapter;
pub mod projection_materialize_adapter;
