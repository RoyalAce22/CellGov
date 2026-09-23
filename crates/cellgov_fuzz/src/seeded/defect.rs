//! The defects a test can seed.

/// One controlled defect a test injects at a target boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SeededDefect {
    /// Every decoder call panics.
    DecoderPanic,
    /// Every executor call panics.
    ExecutorPanic,
    /// The first run reports an outcome class outside the descriptor.
    IllegalOutcome,
    /// The first run emits an effect class outside the descriptor.
    IllegalEffect,
    /// Execution replaces an SPU register outside the allowed footprint.
    IllegalFootprint,
    /// An SPU sequence ends at a misaligned program counter.
    InvalidProgramCounter,
    /// The replay run differs from the first run.
    Nondeterministic,
    /// The metamorphic partner's observation differs from the baseline's.
    MetamorphicMismatch,
    /// Every run stores the wrong value in a register it may write.
    CommonMode,
}
