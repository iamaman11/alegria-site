//! Experimental GraphFlow integration point.
//!
//! This module is intentionally minimal and feature-gated.
//! It allows enabling GraphFlow in selected activities later,
//! without changing production workflow behavior today.

#[cfg(feature = "graphflow_experimental")]
pub(crate) fn graphflow_runtime_label() -> &'static str {
    "graphflow_enabled"
}
