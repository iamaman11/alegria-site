//! Cross-provider wiring for SEO runtime operations.
//!
//! Each provider retains its own implementation; this crate binds PostgreSQL
//! persistence to graph/vector search and workflow ports. It is intentionally
//! downstream of `infrastructure` and must never be imported by it.

pub mod adapters;
