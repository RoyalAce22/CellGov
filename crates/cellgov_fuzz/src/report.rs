//! Pure result records returned by the fuzz engines.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ppu::instruction::fuzz::PpuFuzzKind;
use cellgov_spu::instruction::SpuInstructionKind;

/// Stable interpreter-owned identity of a decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InstructionIdentity {
    /// Exact PPU instruction identity, including family sub-operations.
    Ppu(PpuFuzzKind),
    /// Exact SPU instruction identity.
    Spu(SpuInstructionKind),
}

/// Fuzz engine whose case produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// A validation rule that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    /// Caller configuration cannot execute a meaningful case.
    InvalidConfiguration,
    /// Bounded deterministic generation found no decodable instruction.
    GenerationExhausted,
    /// Decode or execution panicked.
    Panic,
    /// Identical inputs produced a different result.
    Nondeterministic,
    /// Execution returned a result class outside the descriptor.
    IllegalOutcome,
    /// Execution emitted an effect class outside the descriptor.
    IllegalEffect,
    /// An instruction sequence produced an invalid program counter.
    InvalidProgramCounter,
}

/// One reproducible validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Engine that found the failure.
    pub target: FuzzTarget,
    /// Failed validation rule.
    pub kind: FindingKind,
    /// Stable interpreter-owned instruction kind, when decode succeeded.
    pub instruction_kind: Option<InstructionIdentity>,
    /// Raw instruction word.
    pub raw: u32,
    /// Master seed.
    pub seed: u64,
    /// Iteration number.
    pub iteration: u64,
}

/// Pure summary returned by a fuzz engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzReport {
    /// Engine that ran.
    pub target: FuzzTarget,
    /// Master seed.
    pub seed: u64,
    /// Cases considered, including decode refusals.
    pub cases: u64,
    /// Successfully decoded cases.
    pub decoded: u64,
    /// Stable instruction kinds reached.
    pub instruction_kinds: BTreeSet<InstructionIdentity>,
    /// Finding counts by rule.
    pub finding_counts: BTreeMap<FindingKind, u64>,
    /// First bounded set of reproducible findings.
    pub findings: Vec<Finding>,
    max_findings: usize,
}

impl FuzzReport {
    pub(crate) fn new(target: FuzzTarget, seed: u64, max_findings: usize) -> Self {
        Self {
            target,
            seed,
            cases: 0,
            decoded: 0,
            instruction_kinds: BTreeSet::new(),
            finding_counts: BTreeMap::new(),
            findings: Vec::new(),
            max_findings,
        }
    }

    pub(crate) fn reached(&mut self, kind: InstructionIdentity) {
        self.decoded += 1;
        self.instruction_kinds.insert(kind);
    }

    pub(crate) fn finding(&mut self, finding: Finding) {
        *self.finding_counts.entry(finding.kind).or_insert(0) += 1;
        if self.findings.len() < self.max_findings {
            self.findings.push(finding);
        }
    }

    /// Whether the engine found no validation failure.
    pub fn is_clean(&self) -> bool {
        self.finding_counts.is_empty()
    }
}
