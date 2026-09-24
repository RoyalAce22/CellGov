//! Deterministic fuzz engines for the PPU and SPU interpreters; callers provide host policy and campaign configuration.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod artifact;
pub mod decode_census;
pub mod decoder_manifest;
pub mod evaluation;
pub mod loader_images;
pub mod loaders;
pub mod ppu;
pub mod ppu_paths;
pub mod ppu_reference;
pub mod ppu_sequences;
pub mod raw_decode;
pub mod reduce;
pub mod reference;
pub mod regression;
pub mod report;
pub mod runner;
pub mod semantic_sweep;
pub mod smoke;
pub mod spu;
pub mod spu_reference;
pub mod sweep;

mod boundary;
mod campaign;
mod case;
mod config;
mod error;
mod parameters;
mod retention;
mod rng;
mod seeded;

pub use boundary::TargetPanicPayload;
pub use campaign::{
    CampaignSchedule, CampaignShard, CampaignVersion, CancellationBoundary, CaseIndices, CaseRange,
    GenerationStrategy, ReplayCoordinates, CAMPAIGN_VERSION,
};
pub use case::{CaseAssessment, CaseEligibility, CaseFeature, EligibilityReason};
pub(crate) use config::MAX_SEQUENCE_WORDS;
pub use config::{FuzzConfig, DEFAULT_MAX_FINDINGS, DEFAULT_SEQUENCE_WORDS, MAX_RETAINED_FINDINGS};
pub use error::{
    ConfigurationError, FuzzError, GeneratorError, InvariantError, ReductionError,
    ReferenceDisagreement, ReplayVersionError, ReportingError, SynchronizationError, WorkerError,
};
pub use parameters::ParameterStream;
pub use report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, ReductionOutcome, RunOutcome, SemanticFingerprint,
};
pub use retention::{
    BoundaryClass, CampaignDistribution, CrossReferenceAsymmetry, EvaluationDistribution,
    EvaluationError, ExplorationPolicy, OperandAliasClass, RetainedCase, RetainedCases,
    RetentionClass, RetentionConfig, RetentionConfigError, RetentionDecision, SemanticObservation,
    StateTransitionClass, TrialMetrics,
};
pub use sweep::{
    finite_partition_bounds, ppu_decode_partition, reduce_finite_results, spu_decode_partition,
    sweep_finite, DecodePanic, DecodeSweepReport, FiniteCase, FinitePartitionError,
    FiniteSweepError, FiniteSweepReport, FiniteVerdict,
};

#[cfg(test)]
#[path = "tests/retention_tests.rs"]
mod retention_tests;

#[cfg(test)]
#[path = "tests/module_coverage_tests.rs"]
mod module_coverage_tests;
