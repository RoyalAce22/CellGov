//! The defects a test can seed.

/// One controlled defect a test injects at a target boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SeededDefect {
    /// Every decoder call panics.
    DecoderPanic,
    /// Every encoder call returns a word one bit away from the canonical one.
    EncoderMismatch,
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
    /// The SPU decoder folds a stop's ignored bits into its signal type.
    IgnoredFieldRead,
    /// A shift or rotate immediate form reads count bits the ISA masks off.
    UnmaskedCount,
    /// An SPU immediate form computes a result its register form does not.
    ImmediateFormOnly,
    /// A symmetric SPU operation's result depends on its operand order.
    OperandOrder,
    /// An element-wise SPU operation computes its first byte slot wrongly.
    FirstSlot,
    /// A not-taken branch on a zero preferred word advances past the next word.
    BranchFallThrough,
    /// A sequence relation's fused reference drops its first write.
    SequencePartnerWrite,
    /// One word of a sequence relation partner's last written register
    /// changes.
    SequencePartnerLane,
}
