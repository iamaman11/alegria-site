//! PostgreSQL-owned persistence for operational reconciliation.
//!
//! The owner has no Qdrant, Neo4j, Temporal SDK, HTTP or browser dependency.
//! Reconciliation mutates *PostgreSQL outbox state*, not the target projections.
//! Other SQLx adapters can migrate into this owner as their dependencies allow.

pub mod sqlx_adapter;
pub mod sqlx_error;
pub mod sqlx_reconcile_adapter;
pub mod sqlx_reconcile_runtime_adapter;
pub mod sqlx_runtime_health_adapter;

pub use runtime_models::{ReconcileSummary, ReconcileTargetReportRecord};
