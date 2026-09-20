//! SPU fuzz engines built on interpreter-owned descriptors.

use std::panic::{catch_unwind, AssertUnwindSafe};

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{SpuMetamorphicRelation, SpuOutcomeClass};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState, SPU_LS_SIZE};
use cellgov_sync::ReservedLine;

use crate::report::{Finding, FindingKind, FuzzReport, FuzzTarget, InstructionIdentity};
use crate::rng::{Rng, WordGenerationFailure};
use crate::FuzzConfig;

const UNIT: UnitId = UnitId::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedStep {
    outcome: SpuStepOutcome,
    state: SpuObservableSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedSequence {
    state: SpuObservableSnapshot,
    terminal_outcome: Option<SpuStepOutcome>,
    decode_refusal: Option<(u32, u32)>,
    deterministic: bool,
}

/// Checks each decoded SPU instruction against its descriptor.
pub fn run_instructions(config: FuzzConfig) -> FuzzReport {
    let mut report = FuzzReport::new(FuzzTarget::SpuInstruction, config.seed, config.max_findings);
    for iteration in config.iterations() {
        report.cases += 1;
        let mut rng = Rng::for_iter(config.seed, iteration);
        let raw = rng.next_u32();
        let decoded = catch_unwind(AssertUnwindSafe(|| cellgov_spu::decode::decode(raw)));
        let Ok(Ok(instruction)) = decoded else {
            if decoded.is_err() {
                record(&mut report, FindingKind::Panic, None, raw, iteration);
            }
            continue;
        };
        let descriptor = instruction.fuzz_descriptor();
        let identity = InstructionIdentity::Spu(descriptor.kind);
        report.reached(identity);
        let initial = random_state(&mut rng);
        let first = catch_unwind(AssertUnwindSafe(|| run_once(&instruction, &initial)));
        let Ok(first) = first else {
            record(
                &mut report,
                FindingKind::Panic,
                Some(identity),
                raw,
                iteration,
            );
            continue;
        };
        if requests_replay(descriptor.relations) {
            let second = run_once(&instruction, &initial);
            if first != second {
                record(
                    &mut report,
                    FindingKind::Nondeterministic,
                    Some(identity),
                    raw,
                    iteration,
                );
            }
        }
        if !descriptor
            .outcomes
            .contains(&SpuOutcomeClass::from_outcome(&first.outcome))
        {
            record(
                &mut report,
                FindingKind::IllegalOutcome,
                Some(identity),
                raw,
                iteration,
            );
        }
        if outcome_effects(&first.outcome)
            .iter()
            .any(|effect| !descriptor.effects.contains(&effect.kind()))
        {
            record(
                &mut report,
                FindingKind::IllegalEffect,
                Some(identity),
                raw,
                iteration,
            );
        }
    }
    report
}

/// Replays SPU instruction sequences to find nondeterministic outcomes.
pub fn run_sequences(config: FuzzConfig) -> FuzzReport {
    let mut report = FuzzReport::new(FuzzTarget::SpuSequence, config.seed, config.max_findings);
    if config.sequence_words == 0 {
        record(
            &mut report,
            FindingKind::InvalidConfiguration,
            None,
            0,
            config.first_iteration,
        );
        return report;
    }
    for iteration in config.iterations() {
        report.cases += 1;
        let mut rng = Rng::for_iter(config.seed, iteration);
        let words = rng.decoder_accepted_words(config.sequence_words, |raw| {
            catch_unwind(AssertUnwindSafe(|| cellgov_spu::decode::decode(raw)))
                .map(|decoded| decoded.is_ok())
                .map_err(|_| ())
        });
        let words = match words {
            Ok(words) => words,
            Err(WordGenerationFailure::DecoderPanic(raw)) => {
                record(&mut report, FindingKind::Panic, None, raw, iteration);
                continue;
            }
            Err(WordGenerationFailure::Exhausted(raw)) => {
                record(
                    &mut report,
                    FindingKind::GenerationExhausted,
                    None,
                    raw,
                    iteration,
                );
                continue;
            }
        };
        let mut initial = random_state(&mut rng);
        initial.pc = 0;
        for (index, word) in words.iter().enumerate() {
            let start = index * 4;
            if start + 4 > initial.ls.len() {
                break;
            }
            initial.ls[start..start + 4].copy_from_slice(&word.to_be_bytes());
        }
        let first = catch_unwind(AssertUnwindSafe(|| {
            run_sequence(&initial, config.sequence_words)
        }));
        let Ok(first) = first else {
            record(
                &mut report,
                FindingKind::Panic,
                None,
                words.first().copied().unwrap_or(0),
                iteration,
            );
            continue;
        };
        report.decoded += first.1;
        report.instruction_kinds.extend(first.2.iter().copied());
        if first.0.deterministic {
            let second = run_sequence(&initial, config.sequence_words);
            if first.0 != second.0 {
                record(
                    &mut report,
                    FindingKind::Nondeterministic,
                    None,
                    words.first().copied().unwrap_or(0),
                    iteration,
                );
            }
        }
        if first.0.state.pc as usize >= SPU_LS_SIZE || first.0.state.pc & 3 != 0 {
            record(
                &mut report,
                FindingKind::InvalidProgramCounter,
                None,
                words.first().copied().unwrap_or(0),
                iteration,
            );
        }
    }
    report
}

fn run_once(
    instruction: &cellgov_spu::instruction::SpuInstruction,
    initial: &SpuState,
) -> ObservedStep {
    let mut state = initial.clone();
    let outcome = execute(instruction, &mut state, UNIT);
    ObservedStep {
        outcome,
        state: SpuObservableSnapshot::capture(&state),
    }
}

fn run_sequence(
    initial: &SpuState,
    budget: usize,
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    let mut state = initial.clone();
    let mut decoded = 0u64;
    let mut kinds = Vec::new();
    let mut terminal_outcome = None;
    let mut decode_refusal = None;
    let mut deterministic = true;
    for _ in 0..budget {
        let Some(raw) = state.fetch() else {
            break;
        };
        let Ok(instruction) = cellgov_spu::decode::decode(raw) else {
            decode_refusal = Some((state.pc, raw));
            break;
        };
        let descriptor = instruction.fuzz_descriptor();
        deterministic &= requests_replay(descriptor.relations);
        decoded += 1;
        kinds.push(InstructionIdentity::Spu(descriptor.kind));
        let outcome = execute(&instruction, &mut state, UNIT);
        terminal_outcome = Some(outcome.clone());
        match outcome {
            SpuStepOutcome::Continue => state.pc = state.pc.wrapping_add(4),
            SpuStepOutcome::Branch => {}
            SpuStepOutcome::Yield { .. } | SpuStepOutcome::MemoryRead { .. } => break,
            SpuStepOutcome::Fault(_) => {
                // The runtime fault-discard rule hides state from a faulting batch.
                state = initial.clone();
                break;
            }
        }
    }
    (
        ObservedSequence {
            state: SpuObservableSnapshot::capture(&state),
            // Sequence replay retains the terminal outcome because the descriptor observes its effects.
            terminal_outcome,
            decode_refusal,
            deterministic,
        },
        decoded,
        kinds,
    )
}

fn random_state(rng: &mut Rng) -> SpuState {
    let mut state = SpuState::new();
    for register in &mut state.regs {
        rng.fill(register);
    }
    rng.fill(&mut state.ls);
    state.pc = pc_for_slot(rng.below((SPU_LS_SIZE / 4) as u64));
    state.channels.mfc_lsa = rng.next_u32();
    state.channels.mfc_eah = rng.next_u32();
    state.channels.mfc_eal = rng.next_u32();
    state.channels.mfc_size = rng.next_u32();
    state.channels.mfc_tag_id = rng.next_u32();
    state.channels.tag_mask = rng.next_u32();
    state.channels.tag_status = rng.next_u32();
    state.channels.atomic_status = rng.next_u32();
    state.channels.pending_mbox_rt = None;
    state.channels.pending_get = None;
    state.reservation = rng
        .chance(1, 2)
        .then(|| ReservedLine::containing(rng.next_u64() & ((1u64 << 42) - 1)));
    state
}

fn pc_for_slot(slot: u64) -> u32 {
    debug_assert!(slot < (SPU_LS_SIZE / 4) as u64);
    (slot * 4) as u32
}

fn requests_replay(relations: &[SpuMetamorphicRelation]) -> bool {
    relations.contains(&SpuMetamorphicRelation::Deterministic)
}

fn outcome_effects(outcome: &SpuStepOutcome) -> &[Effect] {
    match outcome {
        SpuStepOutcome::Yield { effects, .. } => effects,
        SpuStepOutcome::Continue
        | SpuStepOutcome::Branch
        | SpuStepOutcome::MemoryRead { .. }
        | SpuStepOutcome::Fault(_) => &[],
    }
}

fn record(
    report: &mut FuzzReport,
    kind: FindingKind,
    instruction_kind: Option<InstructionIdentity>,
    raw: u32,
    iteration: u64,
) {
    report.finding(Finding {
        target: report.target,
        kind,
        instruction_kind,
        raw,
        seed: report.seed,
        iteration,
    });
}

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;
