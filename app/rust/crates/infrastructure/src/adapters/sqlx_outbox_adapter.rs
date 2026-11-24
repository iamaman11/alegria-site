//! Compatibility import path for the PostgreSQL-owned Outbox transaction API.
//! The authoritative SQL implementation belongs to `postgres_store`.

pub use postgres_store::sqlx_outbox_adapter::*;
