//! Typed failures raised by fuzz-harness stages.

/// Invalid caller-supplied campaign configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigurationError {
    /// The campaign artifact uses a generator format this build cannot replay.
    #[error("campaign version {found:?} does not match supported version {supported:?}")]
    UnsupportedCampaignVersion {
        /// Version carried by the campaign artifact.
        found: crate::CampaignVersion,
        /// Version understood by this engine.
        supported: crate::CampaignVersion,
    },
    /// The campaign contains no cases.
    #[error("fuzz campaign must contain at least one case")]
    ZeroIterations,
    /// The requested range exceeds the case-index space.
    #[error("case range starting at {first} with count {count} overflows the index space")]
    CaseRangeOverflow {
        /// First requested case index.
        first: u64,
        /// Number of requested cases.
        count: u64,
    },
    /// The shard does not name one member of a nonempty partition.
    #[error("campaign shard {index} is invalid for a partition of {count}")]
    InvalidShard {
        /// Zero-based shard number.
        index: u32,
        /// Total number of shards.
        count: u32,
    },
    /// The cancellation point lies beyond the declared case range.
    #[error("cancellation offset {offset} exceeds campaign count {count}")]
    CancellationOutOfRange {
        /// Requested stop offset.
        offset: u64,
        /// Declared case count.
        count: u64,
    },
    /// A sequence campaign contains no instructions.
    #[error("fuzz sequence must contain at least one instruction")]
    ZeroSequenceWords,
    /// A sequence cannot fit in the target's instruction store.
    #[error("fuzz sequence has {requested} words; target limit is {maximum}")]
    SequenceTooLong {
        /// Requested word count.
        requested: usize,
        /// Largest supported word count.
        maximum: usize,
    },
    /// The retained-finding bound could exhaust host memory before useful work.
    #[error("fuzz campaign retains {requested} findings; limit is {maximum}")]
    TooManyRetainedFindings {
        /// Requested retained-finding count.
        requested: usize,
        /// Largest supported retained-finding count.
        maximum: usize,
    },
}

/// A deterministic case generator could not produce a valid case.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GeneratorError {
    /// Structural constraints rejected all candidates within the deterministic attempt limit.
    #[error("{target} structured generation exhausted {attempts} constraint attempts")]
    ConstraintAttemptsExhausted {
        /// Names the target.
        target: &'static str,
        /// Records the number of rejected candidates.
        attempts: usize,
    },
    /// An interpreter declared no structured generation recipes.
    #[error("{target} descriptor registry is empty")]
    EmptyDescriptorRegistry {
        /// Names the target.
        target: &'static str,
    },
    /// A structural mutation selected an index outside the parameter stream.
    #[error("parameter index {index} is outside stream length {length}")]
    ParameterIndex {
        /// Records the requested parameter index.
        index: usize,
        /// Records the number of parameters in the stream.
        length: usize,
    },
    /// An interpreter-owned PPU descriptor rejected its operand values.
    #[error("PPU structural generation failed: {0}")]
    Ppu(#[from] cellgov_ppu::instruction::fuzz::PpuGenerationError),
    /// An interpreter-owned SPU descriptor rejected its operand values.
    #[error("SPU structural generation failed: {0}")]
    Spu(#[from] cellgov_spu::fuzz::SpuGenerationError),
    /// A probability denominator was zero.
    #[error("generator probability denominator must be nonzero")]
    ZeroProbabilityDenominator,
    /// A probability numerator exceeded its denominator.
    #[error("generator probability numerator {numerator} exceeds denominator {denominator}")]
    InvalidProbability {
        /// Probability numerator.
        numerator: u64,
        /// Probability denominator.
        denominator: u64,
    },
    /// The bounded search found no word accepted by the decoder.
    #[error("generator exhausted its bounded decoder search at raw word 0x{last_raw:08x}")]
    Exhausted {
        /// Final candidate inspected.
        last_raw: u32,
    },
}

/// Two explicitly named reference implementations disagreed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("reference {left} disagreed with {right}")]
pub struct ReferenceDisagreement {
    /// First reference identity.
    pub left: &'static str,
    /// Second reference identity.
    pub right: &'static str,
}

/// A campaign worker did not complete its assigned cases.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerError {
    /// The worker returned a typed harness failure.
    #[error("worker {worker} failed: {failure}")]
    Failed {
        /// Stable worker index.
        worker: usize,
        /// Failure returned by the worker.
        #[source]
        failure: Box<FuzzError>,
    },
    /// The worker panicked outside a target-call boundary.
    #[error("worker {worker} panicked outside the target boundary")]
    Panicked {
        /// Stable worker index.
        worker: usize,
    },
}

/// Synchronization state became unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("fuzz synchronization object {object} was poisoned")]
pub struct SynchronizationError {
    /// Stable name of the poisoned object.
    pub object: &'static str,
}

/// A caller-supplied finding sink rejected a report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("finding sink {sink} rejected a report")]
pub struct ReportingError {
    /// Stable sink identity.
    pub sink: &'static str,
}

/// Automatic reduction could not preserve a finding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReductionError {
    /// A transform produced an invalid case.
    #[error("reduction transform {transform} produced an invalid case")]
    InvalidCandidate {
        /// Stable transform identity.
        transform: &'static str,
    },
    /// A transform changed the semantic finding identity.
    #[error("reduction transform {transform} changed the finding fingerprint")]
    FingerprintChanged {
        /// Stable transform identity.
        transform: &'static str,
    },
}

/// Replay coordinates use an unsupported campaign version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("replay version {found} does not match supported version {supported}")]
pub struct ReplayVersionError {
    /// Version carried by the replay record.
    pub found: crate::CampaignVersion,
    /// Version understood by this engine.
    pub supported: crate::CampaignVersion,
}

/// The harness refused to continue after an internal invariant failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvariantError {
    /// A report counter overflowed.
    #[error("fuzz report counter {counter} overflowed")]
    CounterOverflow {
        /// Stable counter identity.
        counter: &'static str,
    },
    /// A generated value is outside the supported range.
    #[error("generated {value_kind} value {value} is outside the supported range")]
    ValueOutOfRange {
        /// Stable value identity.
        value_kind: &'static str,
        /// Rejected value.
        value: u64,
    },
    /// Harness code panicked outside a target-call boundary.
    #[error("fuzz harness panicked outside the target boundary during {stage}")]
    UnexpectedPanic {
        /// Stable harness stage identity.
        stage: &'static str,
    },
    /// A validated nonempty generated sequence was unexpectedly empty.
    #[error("generated instruction sequence was unexpectedly empty")]
    EmptyGeneratedSequence,
}

/// Failure of the fuzz harness rather than the target under test.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FuzzError {
    /// Caller configuration was invalid.
    #[error("invalid configuration: {0}")]
    Configuration(#[from] ConfigurationError),
    /// Case generation failed.
    #[error("case generation failed: {0}")]
    Generator(#[from] GeneratorError),
    /// Independent references disagreed.
    #[error("reference comparison failed: {0}")]
    Reference(#[from] ReferenceDisagreement),
    /// A worker failed.
    #[error("campaign worker failed: {0}")]
    Worker(#[from] WorkerError),
    /// Synchronization failed.
    #[error("campaign synchronization failed: {0}")]
    Synchronization(#[from] SynchronizationError),
    /// A finding sink rejected its report.
    #[error("finding reporting failed: {0}")]
    Reporting(#[from] ReportingError),
    /// Reduction did not preserve a finding.
    #[error("finding reduction failed: {0}")]
    Reduction(#[from] ReductionError),
    /// Replay coordinates are incompatible with this engine.
    #[error("finding replay failed: {0}")]
    Replay(#[from] ReplayVersionError),
    /// An internal harness invariant failed.
    #[error("fuzz harness invariant failed: {0}")]
    Invariant(#[from] InvariantError),
}
