//! Observation-vs-observation comparator. Walks outcome, memory
//! regions (with per-region byte-divergence coalescing into
//! [`ByteDivergence`] runs), events, state hashes, and steps. A
//! region pair's identity / length mismatch short-circuits the
//! pair but not subsequent pairs.

mod compare;
mod format;
mod types;

pub use compare::*;
pub use format::*;
pub use types::*;

#[cfg(test)]
#[path = "tests/observation_compare_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/observation_compare_identity_tests.rs"]
mod identity_tests;
