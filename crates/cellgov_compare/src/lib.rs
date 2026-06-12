//! Microtest comparison harness.
//!
//! Collects observable outcomes from CellGov and an external oracle
//! (RPCS3), normalizes them into a shared [`Observation`] schema, and
//! reports agreement or classifiable divergence. Each runner's adapter
//! coalesces raw outputs into the shared schema; the comparison layer
//! never touches runner-specific internals.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod baseline;
pub mod boot_summary;
pub mod classify;
pub mod compare;
pub mod diverge;
pub mod format;
pub mod manifest;
pub mod observation;
pub mod observation_compare;
pub mod report;
pub mod runner_cellgov;
#[cfg(feature = "rpcs3-runner")]
pub mod runner_rpcs3;
pub mod summary;
pub mod sync_primitive_scan;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

pub use boot_summary::{BootSummary, BootSummaryError, CheckpointKind};
pub use classify::{classify, ClassifierContext, DivergenceClass, ELF_HEADER_SIZE};
pub use compare::{
    compare, compare_multi, Classification, CompareMode, CompareResult, EventDivergence,
    MemoryDivergence, MultiCompareResult,
};
pub use diverge::{diverge, zoom_lookup, DivergeField, DivergeReport, RegDiff, ZoomLookup};
pub use format::format_with_commas;
pub use observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedEvent, ObservedEventKind,
    ObservedHashes, ObservedOutcome, CODE_REGION_NAME,
};
pub use observation_compare::{
    compare_observations, format_observation_compare_human, format_observation_compare_json,
    ByteDivergence, EventCompare, ObservationCompareResult, RegionCompareSummary,
    RegionPairOutcome, StateHashCompare, StepCompare,
};
pub use report::{format_human, format_json, format_multi_human, format_multi_json};
pub use runner_cellgov::{
    observe, observe_from_boot, observe_with_determinism_check, BootOutcome, BootOutcomeParseError,
    DeterminismError, RegionDescriptor,
};
pub use summary::{
    summarize, ByteParity, ByteParityDivergeReason, Convergence, ConvergenceFailure,
    CrossRunnerSummary, RegionIdent, UnclassifiedRun,
};
