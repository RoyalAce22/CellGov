//! Pure result records returned by the fuzz engines.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_effects::EffectKind;
use cellgov_ppu::instruction::fuzz::PpuFuzzKind;
use cellgov_spu::instruction::SpuInstructionKind;

use crate::case::{CaseAssessment, CaseEligibility, CaseFeature, EligibilityReason};
use crate::error::{FuzzError, InvariantError};
use crate::TargetPanicPayload;

/// Stable interpreter-owned identity of a decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InstructionIdentity {
    /// Exact PPU instruction identity, including family sub-operations.
    Ppu(PpuFuzzKind),
    /// Exact SPU instruction identity.
    Spu(SpuInstructionKind),
}

/// Fuzz engine whose case produced a finding.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum FuzzTarget {
    /// One decoded PPU instruction.
    PpuInstruction,
    /// A short PPU instruction sequence.
    PpuSequence,
    /// One decoded SPU instruction.
    SpuInstruction,
    /// A short SPU instruction sequence.
    SpuSequence,
}

use crate::{GenerationStrategy, ReplayCoordinates};

/// Validation rule or target boundary associated with a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckIdentity {
    /// PPU decoder call.
    PpuDecoder,
    /// SPU decoder call.
    SpuDecoder,
    /// PPU executor call.
    PpuExecutor,
    /// SPU executor call.
    SpuExecutor,
    /// Deterministic replay comparison.
    DeterministicReplay,
    /// Interpreter-owned legal-outcome contract.
    LegalOutcome,
    /// Interpreter-owned legal-effect contract.
    LegalEffect,
    /// Sequence program-counter contract.
    ProgramCounter,
    /// Caller-supplied external reference.
    ExternalReference,
}

/// Semantic class of a disagreement or target failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DivergenceClass {
    /// Target code panicked.
    TargetPanic,
    /// Replayed architectural state differed.
    ArchitecturalState,
    /// Two explicitly named references disagreed.
    ReferenceDisagreement,
    /// Execution returned an outcome outside its contract.
    Outcome,
    /// Execution emitted an effect outside its contract.
    Effect,
    /// Execution produced an invalid control-flow state.
    ControlFlow,
    /// The case is outside implemented behavior.
    Unsupported,
    /// The architecture leaves the case undefined.
    Undefined,
}

/// Typed target outcome used when it contributes to finding identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutcomeIdentity {
    /// Ordinary PPU completion.
    PpuContinue,
    /// PPU branch.
    PpuBranch,
    /// PPU system-call boundary.
    PpuSyscall,
    /// PPU architectural fault.
    PpuFault,
    /// PPU memory fault.
    PpuMemoryFault,
    /// PPU store-buffer refusal.
    PpuBufferFull,
    /// Ordinary SPU completion.
    SpuContinue,
    /// SPU branch.
    SpuBranch,
    /// SPU runtime yield.
    SpuYield,
    /// SPU committed-memory read.
    SpuMemoryRead,
    /// SPU architectural fault.
    SpuFault,
}

/// Stable identity used to bucket and rank semantically equal findings.
///
/// Fingerprints let callers group redundant failures before they rank diverse cases.
/// [Chen2013 p:1 s:Abstract]
/// The check and divergence fields record behavioral asymmetry across references.
/// [Petsios2017 p:615 s:Abstract]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticFingerprint {
    /// Engine that produced the finding.
    pub target: FuzzTarget,
    /// Stable decoded instruction kind, when decode succeeded.
    pub instruction_kind: Option<InstructionIdentity>,
    /// Reference or validation rule that detected the failure.
    pub check: CheckIdentity,
    /// Kind of semantic disagreement.
    pub divergence: DivergenceClass,
    /// Relevant execution outcome, when one exists.
    pub outcome: Option<OutcomeIdentity>,
    /// Relevant effect class, when one exists.
    pub effect: Option<EffectKind>,
}

/// A validation rule that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    /// Target code panicked inside a decoder or executor boundary.
    TargetPanic,
    /// Identical inputs produced a different result.
    Nondeterministic,
    /// Execution returned a result class outside the descriptor.
    IllegalOutcome,
    /// Execution emitted an effect class outside the descriptor.
    IllegalEffect,
    /// An instruction sequence produced an invalid program counter.
    InvalidProgramCounter,
    /// The target explicitly classified the case as unsupported.
    Unsupported,
    /// The target explicitly classified the case as architecturally undefined.
    Undefined,
}

/// One reproducible validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Stable semantic identity used for bucketing and triage.
    pub fingerprint: SemanticFingerprint,
    /// Broad finding category.
    pub kind: FindingKind,
    /// Exact coordinates for deterministic replay.
    pub replay: ReplayCoordinates,
    /// Original unreduced instruction words.
    pub original_words: Vec<u32>,
    /// Reduction state; failures retain the original and replay coordinates above.
    pub reduction: ReductionOutcome,
    /// Deterministic panic payload classification for target panics.
    pub panic_payload: Option<TargetPanicPayload>,
}

/// Result of attempting to reduce a finding reproducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReductionOutcome {
    /// No reduction attempt exists.
    NotAttempted,
    /// Reduction produced a smaller reproducer with the same fingerprint.
    Reduced(Vec<u32>),
    /// Reduction failed; the finding still carries the original reproducer.
    Failed(crate::ReductionError),
}

/// Pure summary returned by a fuzz engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzReport {
    /// Engine that ran.
    pub target: FuzzTarget,
    /// Master seed.
    pub seed: u64,
    /// Records how the run constructed input.
    pub strategy: GenerationStrategy,
    /// Cases considered, including decode refusals.
    pub cases: u64,
    /// Successfully decoded cases.
    pub decoded: u64,
    /// Cases eligible for their selected semantic check.
    pub eligible_cases: u64,
    /// Counts cases that CellGov refuses for an unmodeled precondition.
    pub unsupported_cases: u64,
    /// Counts cases for which the architecture does not define the selected check.
    pub undefined_cases: u64,
    /// Counts eligibility reasons in assessed cases.
    pub eligibility_reasons: BTreeMap<EligibilityReason, u64>,
    /// Counts semantic features in assessed cases.
    pub case_features: BTreeMap<CaseFeature, u64>,
    /// Counts guest-visible effect classes that executed cases emit.
    pub effect_classes: BTreeMap<EffectKind, u64>,
    /// Counts instructions that assessed cases execute.
    pub executed_steps: u64,
    /// Records the largest instruction depth that one case executes.
    pub max_executed_depth: u64,
    /// Stable instruction kinds reached.
    pub instruction_kinds: BTreeSet<InstructionIdentity>,
    /// Finding counts by rule.
    pub finding_counts: BTreeMap<FindingKind, u64>,
    /// First bounded set of reproducible findings.
    pub findings: Vec<Finding>,
    max_findings: usize,
    pub(crate) sequence_words: u32,
}

impl FuzzReport {
    pub(crate) fn new(
        target: FuzzTarget,
        seed: u64,
        strategy: GenerationStrategy,
        max_findings: usize,
        sequence_words: u32,
    ) -> Self {
        Self {
            target,
            seed,
            strategy,
            cases: 0,
            decoded: 0,
            eligible_cases: 0,
            unsupported_cases: 0,
            undefined_cases: 0,
            eligibility_reasons: BTreeMap::new(),
            case_features: BTreeMap::new(),
            effect_classes: BTreeMap::new(),
            executed_steps: 0,
            max_executed_depth: 0,
            instruction_kinds: BTreeSet::new(),
            finding_counts: BTreeMap::new(),
            findings: Vec::new(),
            max_findings,
            sequence_words,
        }
    }

    pub(crate) fn considered(&mut self) -> Result<(), InvariantError> {
        self.cases = self
            .cases
            .checked_add(1)
            .ok_or(InvariantError::CounterOverflow { counter: "cases" })?;
        Ok(())
    }

    pub(crate) fn reached(&mut self, kind: InstructionIdentity) -> Result<(), InvariantError> {
        self.decoded = self
            .decoded
            .checked_add(1)
            .ok_or(InvariantError::CounterOverflow { counter: "decoded" })?;
        self.instruction_kinds.insert(kind);
        Ok(())
    }

    pub(crate) fn reached_many(
        &mut self,
        count: u64,
        kinds: impl IntoIterator<Item = InstructionIdentity>,
    ) -> Result<(), InvariantError> {
        self.decoded = self
            .decoded
            .checked_add(count)
            .ok_or(InvariantError::CounterOverflow { counter: "decoded" })?;
        self.instruction_kinds.extend(kinds);
        Ok(())
    }

    pub(crate) fn assessed(&mut self, assessment: &CaseAssessment) -> Result<(), InvariantError> {
        let counter = match assessment.eligibility {
            CaseEligibility::Eligible => &mut self.eligible_cases,
            CaseEligibility::Unsupported => &mut self.unsupported_cases,
            CaseEligibility::Undefined => &mut self.undefined_cases,
        };
        *counter = counter
            .checked_add(1)
            .ok_or(InvariantError::CounterOverflow {
                counter: "case eligibility",
            })?;
        for reason in &assessment.reasons {
            let count = self.eligibility_reasons.entry(*reason).or_insert(0);
            *count = count
                .checked_add(1)
                .ok_or(InvariantError::CounterOverflow {
                    counter: "eligibility reason",
                })?;
        }
        for feature in &assessment.features {
            let count = self.case_features.entry(*feature).or_insert(0);
            *count = count
                .checked_add(1)
                .ok_or(InvariantError::CounterOverflow {
                    counter: "case feature",
                })?;
        }
        Ok(())
    }

    pub(crate) fn executed(&mut self, depth: u64) -> Result<(), InvariantError> {
        self.executed_steps =
            self.executed_steps
                .checked_add(depth)
                .ok_or(InvariantError::CounterOverflow {
                    counter: "executed steps",
                })?;
        self.max_executed_depth = self.max_executed_depth.max(depth);
        Ok(())
    }

    pub(crate) fn observed_effects(
        &mut self,
        effects: impl IntoIterator<Item = EffectKind>,
    ) -> Result<(), InvariantError> {
        for effect in effects {
            let count = self.effect_classes.entry(effect).or_insert(0);
            *count = count
                .checked_add(1)
                .ok_or(InvariantError::CounterOverflow {
                    counter: "effect class",
                })?;
        }
        Ok(())
    }

    /// Reports the exact eligible-case ratio without floating-point rounding.
    pub fn eligibility_rate(&self) -> Option<(u64, u64)> {
        let classified = self
            .eligible_cases
            .checked_add(self.unsupported_cases)?
            .checked_add(self.undefined_cases)?;
        (classified != 0).then_some((self.eligible_cases, classified))
    }

    pub(crate) fn finding(&mut self, finding: Finding) -> Result<(), InvariantError> {
        let count = self.finding_counts.entry(finding.kind).or_insert(0);
        *count = count
            .checked_add(1)
            .ok_or(InvariantError::CounterOverflow {
                counter: "finding count",
            })?;
        if self.findings.len() < self.max_findings {
            self.findings.push(finding);
        }
        Ok(())
    }

    /// Whether the engine found no validation failure.
    pub fn is_clean(&self) -> bool {
        self.finding_counts.is_empty()
    }
}

/// Terminal classification of a fuzz-engine invocation.
///
/// Target disagreement and target crashes remain distinct findings.
/// [McKeeman1998 p:100 s:Abstract]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// Every scheduled case completed without a finding.
    CleanCompletion,
    /// At least one semantic check found a disagreement.
    SemanticFinding,
    /// Target code panicked inside a decoder or executor boundary.
    TargetPanic,
    /// The caller stopped scheduling at a deterministic boundary.
    Cancelled,
    /// Every classified case was explicitly unsupported.
    UnsupportedCase,
    /// Every classified case was architecturally undefined.
    UndefinedCase,
    /// The campaign encountered both unsupported and undefined cases.
    UnsupportedAndUndefinedCases,
    /// The harness itself failed; this is not a target finding.
    HarnessFailure(FuzzError),
}

/// A structured fuzz-engine result, including partial evidence on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzRun {
    /// Terminal run classification.
    pub outcome: RunOutcome,
    /// Cases and findings completed before the terminal outcome.
    pub report: FuzzReport,
}

impl FuzzRun {
    pub(crate) fn completed(report: FuzzReport) -> Self {
        let has_unsupported = report.unsupported_cases != 0
            || report
                .finding_counts
                .contains_key(&FindingKind::Unsupported);
        let has_undefined = report.undefined_cases != 0
            || report.finding_counts.contains_key(&FindingKind::Undefined);
        let outcome = if report
            .finding_counts
            .contains_key(&FindingKind::TargetPanic)
        {
            RunOutcome::TargetPanic
        } else if report
            .finding_counts
            .keys()
            .any(|kind| !matches!(kind, FindingKind::Unsupported | FindingKind::Undefined))
        {
            RunOutcome::SemanticFinding
        } else if report.is_clean() && report.eligible_cases != 0 {
            RunOutcome::CleanCompletion
        } else {
            match (has_unsupported, has_undefined) {
                (true, false) => RunOutcome::UnsupportedCase,
                (false, true) => RunOutcome::UndefinedCase,
                (true, true) => RunOutcome::UnsupportedAndUndefinedCases,
                (false, false) => RunOutcome::CleanCompletion,
            }
        };
        Self { outcome, report }
    }

    pub(crate) fn failed(report: FuzzReport, error: impl Into<FuzzError>) -> Self {
        Self {
            outcome: RunOutcome::HarnessFailure(error.into()),
            report,
        }
    }

    pub(crate) fn cancelled(report: FuzzReport) -> Self {
        Self {
            outcome: RunOutcome::Cancelled,
            report,
        }
    }
}
