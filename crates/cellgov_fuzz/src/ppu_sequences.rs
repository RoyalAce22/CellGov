//! Generates PPU sequence campaigns and records execution bias.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_exec::YieldReason;
use cellgov_ppu::instruction::fuzz::PpuFuzzKind;
use cellgov_ppu::state::PpuState;
use cellgov_sync::ReservedLine;

use crate::case::{CaseAssessment, CaseEligibility, CaseFeature, EligibilityReason};
use crate::ppu_paths::{
    run_all_paths, run_all_paths_after_code_mutation, PpuPathError, PpuPathRun,
};

const DATA_BASE: u64 = 0x1000_0000;
const DATA_LEN: usize = 64;

/// Named structural intent for one generated PPU sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PpuSequenceFamily {
    /// Source and destination operands alias one register.
    RegisterAlias,
    /// Consecutive stores overlap in memory.
    OverlappingMemory,
    /// A load consumes a preceding buffered store.
    StoreLoadForwarding,
    /// A conditional store consumes a seeded reservation.
    Reservation,
    /// A branch skips one generated instruction.
    ControlledBranch,
    /// A shadow-eligible instruction exercises quickening.
    Quickening,
    /// A fusable pair exercises fusion and consumed-slot classification.
    FusionInvalidation,
}

impl PpuSequenceFamily {
    /// Lists every sequence family in deterministic campaign order.
    pub const ALL: [Self; 7] = [
        Self::RegisterAlias,
        Self::OverlappingMemory,
        Self::StoreLoadForwarding,
        Self::Reservation,
        Self::ControlledBranch,
        Self::Quickening,
        Self::FusionInvalidation,
    ];
}

/// Boundary that a reducer must preserve as one structural unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuReductionBoundary {
    /// First word in the unit.
    pub start: usize,
    /// Exclusive end of the unit.
    pub end: usize,
}

/// Code rewrite that targets a generated shadow-invalidation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuCodeMutation {
    /// Word index rewritten after the initial shadow build.
    pub word_index: usize,
    /// Replacement instruction word.
    pub replacement: u32,
}

/// Deterministic generated sequence and its semantic intent.
#[derive(Clone)]
pub struct PpuGeneratedSequence {
    /// Stable case index.
    pub case_index: u64,
    /// Named structural family.
    pub family: PpuSequenceFamily,
    /// Instruction words in address order.
    pub words: Vec<u32>,
    /// Initial architectural state.
    pub initial_state: PpuState,
    /// Initial data region.
    pub data: Vec<u8>,
    /// Check eligibility established during generation.
    pub assessment: CaseAssessment,
    /// Atomic structural units for reduction.
    pub reduction_boundaries: Vec<PpuReductionBoundary>,
    /// Planned code rewrite, when shadow invalidation is the intent.
    pub code_mutation: Option<PpuCodeMutation>,
}

/// Why an executed sequence stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PpuSequenceStopClass {
    /// Every intended word was dispatched in sequence.
    Complete,
    /// A control transfer skipped one or more intended words.
    ControlTransfer,
    /// Execution faulted.
    Fault,
    /// Execution reached a syscall.
    Syscall,
    /// Another runtime yield stopped the sequence.
    Other,
}

/// Exact replay result retaining intent and actual trace classification.
#[derive(Clone)]
pub struct PpuSequenceReplay {
    /// Generated input and intent.
    pub generated: PpuGeneratedSequence,
    /// Four internal execution-path results.
    pub runs: Vec<PpuPathRun>,
    /// Stop class derived from the plain execution trace.
    pub stop: PpuSequenceStopClass,
    /// Counts by exact instruction kind in generated address order.
    pub intended_opcodes: BTreeMap<PpuFuzzKind, u64>,
    /// Counts by exact instruction kind in plain-path dispatch order.
    pub executed_opcodes: BTreeMap<PpuFuzzKind, u64>,
}

/// Aggregate distribution for a deterministic sequence campaign.
#[derive(Clone, Default)]
pub struct PpuSequenceCampaignReport {
    /// Cases generated per family.
    pub intended_families: BTreeMap<PpuSequenceFamily, u64>,
    /// Stop-class distribution.
    pub stop_causes: BTreeMap<PpuSequenceStopClass, u64>,
    /// Distribution of generated instruction kinds.
    pub intended_opcodes: BTreeMap<PpuFuzzKind, u64>,
    /// Distribution of dispatched instruction kinds.
    pub executed_opcodes: BTreeMap<PpuFuzzKind, u64>,
    /// Exact case replays in case-index order.
    pub cases: Vec<PpuSequenceReplay>,
}

/// Failure to generate or replay a dependency-rich campaign.
#[derive(Debug, thiserror::Error)]
pub enum PpuSequenceCampaignError {
    /// The internal path harness refused a generated case.
    #[error("PPU sequence campaign replay failed: {0}")]
    Path(#[from] PpuPathError),
    /// A generated instruction word was not decodable.
    #[error("PPU sequence opcode decode failed: {0}")]
    Decode(#[from] cellgov_ppu::instruction::PpuDecodeError),
    /// A generated trace named a PC outside its words.
    #[error("PPU sequence trace PC 0x{pc:016x} is outside {words} words")]
    TracePc {
        /// Rejected program counter.
        pc: u64,
        /// Generated word count.
        words: usize,
    },
    /// A campaign counter overflowed.
    #[error("PPU sequence campaign counter overflowed for {counter}")]
    CounterOverflow {
        /// Counter name.
        counter: &'static str,
    },
}

/// Generates one sequence from a stable seed and case index.
// [Wang2024 p:340:1 s:Abstract] Generation tracks state and dependencies while it constructs a program.
// [Padhye2019 p:329 s:Abstract] Typed parameters map deterministic mutations into structural program changes.
pub fn generate_dependency_sequence(seed: u64, case_index: u64) -> PpuGeneratedSequence {
    let family =
        PpuSequenceFamily::ALL[(case_index % PpuSequenceFamily::ALL.len() as u64) as usize];
    let value = ((seed ^ case_index.rotate_left(17)) as u16).max(1);
    let mut state = PpuState::new();
    state.set_gpr(3, u64::from(value));
    state.set_gpr(4, DATA_BASE);
    state.set_gpr(5, u64::from(value ^ 0x55));
    let mut features = BTreeSet::from([CaseFeature::MappedMemory]);
    let mut code_mutation = None;
    let words = match family {
        PpuSequenceFamily::RegisterAlias => {
            features.extend([CaseFeature::OperandAlias, CaseFeature::DependencyChain]);
            vec![addi(3, 3, 1), addi(3, 3, 1)]
        }
        PpuSequenceFamily::OverlappingMemory => {
            features.insert(CaseFeature::DependencyChain);
            vec![stw(3, 4, 0), sth(5, 4, 1)]
        }
        PpuSequenceFamily::StoreLoadForwarding => {
            features.insert(CaseFeature::DependencyChain);
            vec![stw(3, 4, 0), lwz(6, 4, 0)]
        }
        PpuSequenceFamily::Reservation => {
            features.extend([CaseFeature::Reservation, CaseFeature::DependencyChain]);
            state.set_gpr(5, 0);
            state.set_reservation(Some(ReservedLine::containing(DATA_BASE)));
            vec![stwcx(3, 4, 5), lwz(6, 4, 0)]
        }
        PpuSequenceFamily::ControlledBranch => {
            features.insert(CaseFeature::ControlledFlow);
            vec![branch_relative(8), li(3, value ^ 1), li(3, value)]
        }
        PpuSequenceFamily::Quickening => {
            features.insert(CaseFeature::DependencyChain);
            vec![ori(3, 3, 0), addi(3, 3, 1)]
        }
        PpuSequenceFamily::FusionInvalidation => {
            features.insert(CaseFeature::DependencyChain);
            code_mutation = Some(PpuCodeMutation {
                word_index: 0,
                replacement: li(3, value ^ 1),
            });
            vec![li(3, value), stw(3, 4, 0)]
        }
    };
    PpuGeneratedSequence {
        case_index,
        family,
        reduction_boundaries: vec![PpuReductionBoundary {
            start: 0,
            end: words.len(),
        }],
        code_mutation,
        words,
        initial_state: state,
        data: vec![0; DATA_LEN],
        assessment: CaseAssessment::new(
            CaseEligibility::Eligible,
            EligibilityReason::StatePreconditions,
            features,
        )
        .with_reason(EligibilityReason::InterpreterContract),
    }
}

/// Replays a generated sequence while retaining its intent and actual trace class.
pub fn replay_dependency_sequence(
    generated: PpuGeneratedSequence,
) -> Result<PpuSequenceReplay, PpuSequenceCampaignError> {
    let mut executed_code = generated.words.clone();
    let runs = if let Some(mutation) = generated.code_mutation {
        if let Some(word) = executed_code.get_mut(mutation.word_index) {
            *word = mutation.replacement;
        }
        run_all_paths_after_code_mutation(
            &generated.words,
            &generated.initial_state,
            &generated.data,
            mutation.word_index,
            mutation.replacement,
        )?
    } else {
        run_all_paths(&generated.words, &generated.initial_state, &generated.data)?
    };
    let plain = &runs[0];
    let intended_opcodes = counts(generated.words.iter().copied())?;
    let mut executed_words = Vec::with_capacity(plain.executed_pcs.len());
    let mut executed_intended_pcs = Vec::new();
    for &pc in &plain.executed_pcs {
        if pc % 4 != 0 {
            return Err(PpuSequenceCampaignError::TracePc {
                pc,
                words: generated.words.len(),
            });
        }
        let index = usize::try_from(pc / 4).map_err(|_| PpuSequenceCampaignError::TracePc {
            pc,
            words: generated.words.len(),
        })?;
        let Some(&word) = executed_code.get(index) else {
            continue;
        };
        executed_intended_pcs.push(pc);
        executed_words.push(word);
    }
    let executed_opcodes = counts(executed_words)?;
    let stop = match plain.stop.reason {
        YieldReason::Fault => PpuSequenceStopClass::Fault,
        YieldReason::Syscall => PpuSequenceStopClass::Syscall,
        _ if generated
            .assessment
            .features
            .contains(&CaseFeature::ControlledFlow) =>
        {
            let sequential = (0..generated.words.len())
                .map(|index| index as u64 * 4)
                .collect::<Vec<_>>();
            if executed_intended_pcs == sequential {
                PpuSequenceStopClass::Complete
            } else {
                PpuSequenceStopClass::ControlTransfer
            }
        }
        _ if executed_intended_pcs.len() == generated.words.len() => PpuSequenceStopClass::Complete,
        _ => PpuSequenceStopClass::Other,
    };
    Ok(PpuSequenceReplay {
        generated,
        runs,
        stop,
        intended_opcodes,
        executed_opcodes,
    })
}

/// Runs a bounded deterministic campaign and aggregates intent and execution bias.
pub fn run_dependency_campaign(
    seed: u64,
    cases: u64,
) -> Result<PpuSequenceCampaignReport, PpuSequenceCampaignError> {
    let mut report = PpuSequenceCampaignReport::default();
    for case_index in 0..cases {
        let replay = replay_dependency_sequence(generate_dependency_sequence(seed, case_index))?;
        increment(
            &mut report.intended_families,
            replay.generated.family,
            "families",
        )?;
        increment(&mut report.stop_causes, replay.stop, "stop causes")?;
        merge_counts(
            &mut report.intended_opcodes,
            &replay.intended_opcodes,
            "intended opcodes",
        )?;
        merge_counts(
            &mut report.executed_opcodes,
            &replay.executed_opcodes,
            "executed opcodes",
        )?;
        report.cases.push(replay);
    }
    Ok(report)
}

fn counts(
    values: impl IntoIterator<Item = u32>,
) -> Result<BTreeMap<PpuFuzzKind, u64>, PpuSequenceCampaignError> {
    let mut counts = BTreeMap::new();
    for value in values {
        let instruction = cellgov_ppu::decode::decode(value)?;
        increment(
            &mut counts,
            instruction.fuzz_descriptor(value).kind,
            "opcode counts",
        )?;
    }
    Ok(counts)
}

fn increment<K: Ord>(
    counts: &mut BTreeMap<K, u64>,
    key: K,
    counter: &'static str,
) -> Result<(), PpuSequenceCampaignError> {
    let value = counts.entry(key).or_insert(0);
    *value = value
        .checked_add(1)
        .ok_or(PpuSequenceCampaignError::CounterOverflow { counter })?;
    Ok(())
}

fn merge_counts<K: Ord + Copy>(
    target: &mut BTreeMap<K, u64>,
    source: &BTreeMap<K, u64>,
    counter: &'static str,
) -> Result<(), PpuSequenceCampaignError> {
    for (&key, &amount) in source {
        let value = target.entry(key).or_insert(0);
        *value = value
            .checked_add(amount)
            .ok_or(PpuSequenceCampaignError::CounterOverflow { counter })?;
    }
    Ok(())
}

fn addi(rt: u32, ra: u32, value: u16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | u32::from(value)
}

fn li(rt: u32, value: u16) -> u32 {
    addi(rt, 0, value)
}

fn ori(ra: u32, rs: u32, value: u16) -> u32 {
    (24 << 26) | (rs << 21) | (ra << 16) | u32::from(value)
}

fn stw(rs: u32, ra: u32, offset: u16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

fn lwz(rt: u32, ra: u32, offset: u16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(offset)
}

fn sth(rs: u32, ra: u32, offset: u16) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

fn stwcx(rs: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (150 << 1) | 1
}

fn branch_relative(bytes: u32) -> u32 {
    (18 << 26) | (bytes & 0x03ff_fffc)
}

#[cfg(test)]
#[path = "tests/ppu_sequences_tests.rs"]
mod tests;
