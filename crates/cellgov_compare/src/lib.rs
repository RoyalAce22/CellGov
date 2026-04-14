//! cellgov_compare -- microtest comparison harness.
//!
//! Collects observable outcomes from CellGov and an external oracle
//! (RPCS3), normalizes them into a shared `Observation` schema, and
//! reports agreement or classifiable divergence.
//!
//! The comparison layer operates only on normalized observations, never
//! on runner-specific internals. Each runner's adapter is responsible
//! for coalescing raw outputs into the shared schema.

pub mod baseline;
pub mod compare;
pub mod diverge;
pub mod manifest;
pub mod observation;
pub mod report;
pub mod runner_cellgov;
#[cfg(feature = "rpcs3-runner")]
pub mod runner_rpcs3;

#[cfg(test)]
mod test_support;

pub use compare::{
    compare, compare_multi, Classification, CompareMode, CompareResult, EventDivergence,
    MemoryDivergence, MultiCompareResult,
};
pub use diverge::{diverge, zoom_lookup, DivergeField, DivergeReport, RegDiff, ZoomLookup};
pub use observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedEvent, ObservedEventKind,
    ObservedHashes, ObservedOutcome,
};
pub use report::{format_human, format_json, format_multi_human, format_multi_json};
pub use runner_cellgov::{
    observe, observe_from_boot, observe_with_determinism_check, BootOutcome, DeterminismError,
    RegionDescriptor,
};
