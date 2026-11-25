//! Compatibility façade only. Real admitted Qdrant provider work lives in
//! the physically independent `qdrant_effects_core` owner: this crate also
//! contains legacy graph/Voyage dispatch and is not a Temporal worker input.
pub use qdrant_effects_core::*;
