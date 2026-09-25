//! Microtest comparison harness.
//!
//! Collects observable outcomes from CellGov and an external oracle
//! (RPCS3), normalizes them into a shared [`Observation`] schema, and
//! reports agreement or classifiable divergence. Each runner's adapter
//! coalesces raw outputs into the shared schema; the comparison layer
//! never touches runner-specific internals.
//!
//! [McKeeman1998 p:100 s:Differential Testing] Two or more comparable
//! systems run the same input; a differing result is a candidate for
//! a bug-exposing test.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod baseline;
pub mod bench;
pub mod boot_history;
pub mod boot_summary;
pub mod checkpoint_manifest;
pub mod classify;
pub mod compare;
pub mod diverge;
pub mod format;
pub mod identity;
pub mod manifest;
pub mod observation;
pub mod observation_compare;
pub mod report;
pub mod runner_cellgov;
#[cfg(feature = "rpcs3-runner")]
pub mod runner_rpcs3;
pub mod summary;
pub mod sync_primitive_scan;
pub mod trace_decode;
pub mod witness_parse;
pub mod witnesses;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "tests/scheme_mismatch_tests.rs"]
mod scheme_mismatch_tests;

#[cfg(test)]
#[path = "tests/checkpoint_scheme_tests.rs"]
mod checkpoint_scheme_tests;

pub use boot_summary::{BootSummary, BootSummaryError, CheckpointKind};
pub use cellgov_core::AddressSpaceId;
pub use checkpoint_manifest::{CheckpointManifest, CheckpointManifestError, CheckpointRegion};
pub use classify::{classify, ClassifierContext, DivergenceClass, ELF_HEADER_SIZE};
pub use compare::{
    compare, compare_multi, Classification, CompareMode, CompareResult, EventDivergence,
    MemoryDivergence, MultiCompareResult, StateHashDivergence,
};
pub use diverge::{
    diverge, trace_scheme, zoom_lookup, DivergeField, DivergeReport, RegDiff, TraceSchemes,
    ZoomLookup,
};
pub use format::format_with_commas;
pub use identity::{
    cross_identity_warning, cross_trace_identity_warning, identity_report, trace_identity,
    AppVersion, BootOverrides, FirmwareIdentity, GameIdentity, RunIdentity, SentinelParseError,
    TraceIdentity, TwoVersionKeys, BASE_VERSION, RUN_IDENTITY_SENTINEL,
};
pub use observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedEvent, ObservedEventKind,
    ObservedHashes, ObservedOutcome, CHECKPOINT_HASH_SCHEME, CODE_REGION_NAME,
    LEGACY_CHECKPOINT_HASH_SCHEME,
};
pub use observation_compare::{
    compare_observations, format_observation_compare_human, format_observation_compare_json,
    ByteDivergence, EventCompare, ObservationCompareResult, RegionCompareSummary,
    RegionPairOutcome, StateHashCompare, StepCompare,
};
pub use report::{format_human, format_json, format_multi_human, format_multi_json};
pub use runner_cellgov::{
    observe, observe_checked, observe_from_boot, observe_with_determinism_check, BootOutcome,
    BootOutcomeParseError, CheckedRun, DeterminismError, ObserveDisagreement, ObserveError,
    RegionDescriptor, RegionExtractError, SpaceSnapshots,
};
pub use summary::{
    summarize, ByteParity, ByteParityDivergeReason, Convergence, ConvergenceFailure,
    CrossRunnerSummary, RegionIdent, UnclassifiedRun,
};
pub use trace_decode::TraceDecodeError;
