//! Field-by-field diff between two `Observation` values, returning the
//! first point of divergence. The public vocabulary ([`CompareResult`],
//! [`CompareMode`], [`MemoryDivergence`], [`EventDivergence`]) is
//! re-exported below; memory and event diffs live in dedicated
//! submodules.

mod driver;
mod events;
mod memory;
mod types;

pub use driver::{compare, compare_multi};
pub use types::{
    Classification, CompareMode, CompareResult, EventDivergence, MemoryDivergence,
    MultiCompareResult,
};
