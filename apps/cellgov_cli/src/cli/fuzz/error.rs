//! The typed refusals and failures `cellgov dev fuzz` reports.

use std::path::PathBuf;

use cellgov_fuzz::artifact::{
    ArtifactError, ArtifactReduction, ArtifactReplayError, FuzzFindingArtifact,
};
use cellgov_fuzz::decode_census::DecodeCensusError;
use cellgov_fuzz::evaluation::{ComparisonError, EvaluationPlanError, ResultsError};
use cellgov_fuzz::raw_decode::{RawDecodeError, MAX_RAW_DECODE_PANIC_SAMPLES};
use cellgov_fuzz::regression::RegressionError;

use super::outcome;
use crate::cli::exit_codes;

#[derive(Debug, thiserror::Error)]
pub(crate) enum FuzzCliError {
    #[error("fuzz: {0}")]
    Invalid(&'static str),
    #[error("fuzz: campaign range starting at {first} cannot hold {count} cases")]
    Range { first: u64, count: u64 },
    #[error(
        "fuzz: finding-limit must be within 1..={}",
        cellgov_fuzz::MAX_RETAINED_FINDINGS
    )]
    FindingLimit,
    #[error("fuzz: {0}")]
    Campaign(cellgov_fuzz::runner::CampaignError),
    #[error("fuzz: selected check is not independently switchable by this engine")]
    CheckUnavailable,
    #[error("fuzz: raw decoder scans keep panic samples as scanned and do not reduce them")]
    ReductionUnavailable,
    #[error("fuzz: invalid campaign configuration: {0}")]
    Configuration(#[from] cellgov_fuzz::ConfigurationError),
    #[error("fuzz: raw scans retain exactly {MAX_RAW_DECODE_PANIC_SAMPLES} panic samples")]
    RawFindingLimit,
    #[error("fuzz: reference read {}: {source}", path.display())]
    ReferenceRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fuzz: PPU reference: {0}")]
    PpuReference(#[from] cellgov_fuzz::ppu_reference::PpuReferenceError),
    #[error("fuzz: SPU reference: {0}")]
    SpuReference(#[from] cellgov_fuzz::spu_reference::SpuReferenceError),
    #[error("fuzz: independent reference differs from the interpreter")]
    ReferenceMismatch,
    #[error("fuzz: raw decoder scan: {0}")]
    Raw(#[from] RawDecodeError),
    #[error("fuzz: decoder census: {0}")]
    Census(#[from] DecodeCensusError),
    #[error("fuzz: census read {}: {source}", path.display())]
    CensusRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fuzz: census partition: {0}")]
    CensusPartition(#[source] cellgov_fuzz::FinitePartitionError),
    #[error("fuzz: census offset overflows the 32-bit word space")]
    CensusOffsetOverflow,
    #[error("fuzz: census progress channel closed before the workers finished")]
    CensusProgressClosed,
    #[error(
        "fuzz: merged census covers 0x{first:08x} for {count} words, not the whole 32-bit space"
    )]
    CensusIncomplete { first: u32, count: u64 },
    #[error("fuzz: JSON serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("fuzz: write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fuzz: stdout write: {0}")]
    Stdout(#[source] std::io::Error),
    #[error("fuzz: an engine worker panicked outside its target boundary")]
    WorkerPanic,
    #[error("fuzz: start engine worker: {0}")]
    WorkerSpawn(#[source] std::io::Error),
    #[error("fuzz: engine failed: {0}")]
    Harness(#[source] cellgov_fuzz::FuzzError),
    #[error("fuzz: report counters overflowed")]
    CounterOverflow,
    #[error(
        "fuzz: campaign selected no eligible cases (cases={cases}, decoded={decoded}, unsupported={unsupported}, undefined={undefined})"
    )]
    NoEligibleCases {
        cases: u64,
        decoded: u64,
        unsupported: u64,
        undefined: u64,
    },
    #[error("fuzz: finding artifact: {0}")]
    Artifact(#[from] ArtifactError),
    #[error("fuzz: finding replay: {0}")]
    ArtifactReplay(#[from] ArtifactReplayError),
    #[error("fuzz: artifact read {}: {source}", path.display())]
    ArtifactRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fuzz: artifact encoding failed: {source}; original case {} words {:?}", artifact.original.replay.case_index, artifact.original.words)]
    ArtifactEncoding {
        #[source]
        source: serde_json::Error,
        artifact: Box<FuzzFindingArtifact>,
    },
    #[error("fuzz: artifact write {} failed: {source}; original case {} words {:?}", path.display(), artifact.original.replay.case_index, artifact.original.words)]
    ArtifactWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
        artifact: Box<FuzzFindingArtifact>,
    },
    #[error("fuzz: artifact path {} already holds different evidence; original case {} words {:?}", path.display(), artifact.original.replay.case_index, artifact.original.words)]
    ArtifactCollision {
        path: PathBuf,
        artifact: Box<FuzzFindingArtifact>,
    },
    #[error("fuzz: artifact path {} already holds this finding with reduction {stored:?}; reduction {:?} was not stored; original case {} words {:?}", path.display(), artifact.reduction, artifact.original.replay.case_index, artifact.original.words)]
    ArtifactReductionNotStored {
        path: PathBuf,
        stored: ArtifactReduction,
        artifact: Box<FuzzFindingArtifact>,
    },
    #[error("fuzz: evaluation plan: {0}")]
    EvaluationPlan(#[from] EvaluationPlanError),
    #[error("fuzz: evaluation results: {0}")]
    EvaluationResults(#[from] ResultsError),
    #[error("fuzz: evaluation results read {}: {source}", path.display())]
    EvaluationRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fuzz: evaluation comparison: {0}")]
    EvaluationComparison(#[from] ComparisonError),
    #[error("fuzz: evaluation trial seed {seed} failed inside the harness: {message}")]
    TrialHarness { seed: u64, message: String },
    #[error("fuzz: {0}")]
    Regressions(#[from] RegressionError),
}

impl From<cellgov_fuzz::runner::CampaignError> for FuzzCliError {
    fn from(error: cellgov_fuzz::runner::CampaignError) -> Self {
        use cellgov_fuzz::runner::CampaignError;
        match error {
            CampaignError::FindingLimit => Self::FindingLimit,
            CampaignError::Range { first, count } => Self::Range { first, count },
            CampaignError::Configuration(source) => Self::Configuration(source),
            CampaignError::CounterOverflow => Self::CounterOverflow,
            CampaignError::Worker(failure) => failure.into(),
            other => Self::Campaign(other),
        }
    }
}

impl From<cellgov_fuzz::runner::WorkerFailure> for FuzzCliError {
    fn from(failure: cellgov_fuzz::runner::WorkerFailure) -> Self {
        match failure {
            cellgov_fuzz::runner::WorkerFailure::Panicked => Self::WorkerPanic,
            cellgov_fuzz::runner::WorkerFailure::Spawn(source) => Self::WorkerSpawn(source),
        }
    }
}

impl FuzzCliError {
    pub(crate) const fn is_usage(&self) -> bool {
        match self {
            Self::Campaign(source) => source.is_request_refusal(),
            Self::Invalid(_)
            | Self::Range { .. }
            | Self::FindingLimit
            | Self::CheckUnavailable
            | Self::ReductionUnavailable
            | Self::Configuration(_)
            | Self::RawFindingLimit
            | Self::Harness(cellgov_fuzz::FuzzError::Configuration(_))
            | Self::EvaluationPlan(_) => true,
            Self::Raw(source) => source.is_invalid_request(),
            // An interval the census cannot cover is a refused request; a
            // part that cannot be merged or parsed is a failed operation.
            Self::Census(source) => matches!(source, DecodeCensusError::Domain(_)),
            Self::CensusRead { .. }
            | Self::CensusPartition(_)
            | Self::CensusOffsetOverflow
            | Self::CensusProgressClosed
            | Self::CensusIncomplete { .. } => false,
            // Two results the command cannot rank are a refused request. An
            // incomplete or unfinished result is a failed operation.
            Self::EvaluationComparison(source) => !matches!(
                source,
                ComparisonError::Invalid { .. } | ComparisonError::HarnessFailed { .. }
            ),
            Self::EvaluationResults(_)
            | Self::EvaluationRead { .. }
            | Self::TrialHarness { .. }
            | Self::Regressions(_) => false,
            Self::ReferenceRead { .. }
            | Self::PpuReference(_)
            | Self::SpuReference(_)
            | Self::ReferenceMismatch
            | Self::Json(_)
            | Self::Write { .. }
            | Self::Stdout(_)
            | Self::WorkerPanic
            | Self::WorkerSpawn(_)
            | Self::Harness(_)
            | Self::CounterOverflow
            | Self::NoEligibleCases { .. } => false,
            Self::Artifact(_)
            | Self::ArtifactReplay(_)
            | Self::ArtifactRead { .. }
            | Self::ArtifactEncoding { .. }
            | Self::ArtifactWrite { .. }
            | Self::ArtifactCollision { .. }
            | Self::ArtifactReductionNotStored { .. } => false,
        }
    }

    pub(crate) fn is_broken_pipe(&self) -> bool {
        matches!(self, Self::Stdout(source) if source.kind() == std::io::ErrorKind::BrokenPipe)
    }

    /// The documented exit status this failure maps to.
    pub(crate) fn exit_code(&self) -> i32 {
        if self.is_broken_pipe() {
            return exit_codes::BROKEN_PIPE;
        }
        if self.is_usage() {
            return exit_codes::USAGE;
        }
        match self {
            Self::Harness(_)
            | Self::TrialHarness { .. }
            | Self::ArtifactReplay(ArtifactReplayError::HarnessFailure { .. }) => {
                outcome::EXIT_HARNESS_FAILURE
            }
            Self::NoEligibleCases { .. } => outcome::EXIT_NO_ELIGIBLE_CASES,
            Self::ArtifactEncoding { .. }
            | Self::ArtifactWrite { .. }
            | Self::ArtifactCollision { .. }
            | Self::ArtifactReductionNotStored { .. } => outcome::EXIT_EVIDENCE_NOT_STORED,
            Self::ArtifactReplay(ArtifactReplayError::NotReproduced { .. }) => {
                outcome::EXIT_NOT_REPRODUCED
            }
            // A request the artifact cannot serve is usage, like a flag the
            // command refuses.
            Self::Artifact(_)
            | Self::ArtifactReplay(
                ArtifactReplayError::Artifact(_) | ArtifactReplayError::NoReducedCase,
            ) => exit_codes::USAGE,
            Self::Invalid(_)
            | Self::Campaign(_)
            | Self::Range { .. }
            | Self::FindingLimit
            | Self::CheckUnavailable
            | Self::ReductionUnavailable
            | Self::Configuration(_)
            | Self::RawFindingLimit
            | Self::Raw(_)
            | Self::Census(_)
            | Self::CensusRead { .. }
            | Self::CensusPartition(_)
            | Self::CensusOffsetOverflow
            | Self::CensusProgressClosed
            | Self::CensusIncomplete { .. }
            | Self::ReferenceRead { .. }
            | Self::PpuReference(_)
            | Self::SpuReference(_)
            | Self::ReferenceMismatch
            | Self::Json(_)
            | Self::Write { .. }
            | Self::Stdout(_)
            | Self::WorkerPanic
            | Self::WorkerSpawn(_)
            | Self::CounterOverflow
            | Self::ArtifactRead { .. }
            | Self::EvaluationPlan(_)
            | Self::EvaluationResults(_)
            | Self::EvaluationRead { .. }
            | Self::EvaluationComparison(_)
            | Self::Regressions(_)
            | Self::ArtifactReplay(
                ArtifactReplayError::ReferenceMismatch
                | ArtifactReplayError::PpuReference(_)
                | ArtifactReplayError::SpuReference(_),
            ) => exit_codes::FAILED,
        }
    }
}
