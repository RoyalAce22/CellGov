//! PPU fuzz engines built on interpreter-owned descriptors.

use std::panic::{catch_unwind, AssertUnwindSafe};

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::RegionView;
use cellgov_ppu::differential::PpuStateSnapshot;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::instruction::fuzz::{PpuMetamorphicRelation, PpuOutcomeClass};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_sync::ReservedLine;

use crate::report::{Finding, FindingKind, FuzzReport, FuzzTarget, InstructionIdentity};
use crate::rng::{Rng, WordGenerationFailure};
use crate::FuzzConfig;

const UNIT: UnitId = UnitId::new(0);
const DATA_BASE: u64 = 0x1000_0000;
const DATA_LEN: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedStep {
    verdict: ExecuteVerdict,
    state: PpuStateSnapshot,
    pc: u64,
    vrsave: u32,
    vrsave_written: bool,
    tb: u64,
    effects: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedSequence {
    state: PpuStateSnapshot,
    pc: u64,
    vrsave: u32,
    vrsave_written: bool,
    tb: u64,
    terminal_verdict: Option<ExecuteVerdict>,
    decode_refusal: Option<(u64, u32)>,
    deterministic: bool,
    effects: Vec<Effect>,
}

/// Checks each decoded PPU instruction against its descriptor.
pub fn run_instructions(config: FuzzConfig) -> FuzzReport {
    let mut report = FuzzReport::new(FuzzTarget::PpuInstruction, config.seed, config.max_findings);
    for iteration in config.iterations() {
        report.cases += 1;
        let mut rng = Rng::for_iter(config.seed, iteration);
        let raw = rng.next_u32();
        let decoded = catch_unwind(AssertUnwindSafe(|| cellgov_ppu::decode::decode(raw)));
        let Ok(Ok(instruction)) = decoded else {
            if decoded.is_err() {
                record(&mut report, FindingKind::Panic, None, raw, iteration);
            }
            continue;
        };
        let descriptor = instruction.fuzz_descriptor(raw);
        let identity = InstructionIdentity::Ppu(descriptor.kind);
        report.reached(identity);
        let initial = random_state(&mut rng);
        let mut memory = vec![0u8; DATA_LEN];
        rng.fill(&mut memory);
        let first = catch_unwind(AssertUnwindSafe(|| {
            run_once(&instruction, &initial, &memory)
        }));
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
            let second = run_once(&instruction, &initial, &memory);
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
            .contains(&PpuOutcomeClass::from_verdict(&first.verdict))
        {
            record(
                &mut report,
                FindingKind::IllegalOutcome,
                Some(identity),
                raw,
                iteration,
            );
        }
        if first
            .effects
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

/// Replays PPU instruction sequences to find nondeterministic outcomes.
pub fn run_sequences(config: FuzzConfig) -> FuzzReport {
    let mut report = FuzzReport::new(FuzzTarget::PpuSequence, config.seed, config.max_findings);
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
            catch_unwind(AssertUnwindSafe(|| cellgov_ppu::decode::decode(raw)))
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
        let mut memory = vec![0u8; DATA_LEN];
        rng.fill(&mut memory);
        let first = catch_unwind(AssertUnwindSafe(|| run_sequence(&words, &initial, &memory)));
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
            let second = run_sequence(&words, &initial, &memory);
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
    }
    report
}

fn run_once(instruction: &PpuInstruction, initial: &PpuState, memory: &[u8]) -> ObservedStep {
    let mut state = initial.clone();
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let views = [RegionView::plain(DATA_BASE, memory)];
    let verdict = execute(
        instruction,
        &mut state,
        UNIT,
        &views,
        &mut effects,
        &mut stores,
    );
    stores.flush(&mut effects, UNIT);
    // Mirror `PpuExecutionUnit::close_block`: clock reads emit one guest-visible ClockRead effect at the block boundary.
    if state.clock_read {
        effects.push(Effect::ClockRead { source: UNIT });
    }
    ObservedStep {
        verdict,
        state: PpuStateSnapshot::capture(&state),
        pc: state.pc,
        vrsave: state.vrsave,
        vrsave_written: state.vrsave_written,
        tb: state.tb,
        effects,
    }
}

fn run_sequence(
    words: &[u32],
    initial: &PpuState,
    memory: &[u8],
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    let mut state = initial.clone();
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let views = [RegionView::plain(DATA_BASE, memory)];
    let mut decoded = 0u64;
    let mut kinds = Vec::new();
    let mut terminal_verdict = None;
    let mut decode_refusal = None;
    let mut deterministic = true;
    for _ in 0..words.len() {
        // Mirror `PpuExecutionUnit::run_batch`: a branch sets the PC for the next fetched word.
        let Some(index) = state
            .pc
            .checked_div(4)
            .and_then(|index| usize::try_from(index).ok())
        else {
            break;
        };
        let Some(&raw) = words.get(index) else {
            break;
        };
        let Ok(instruction) = cellgov_ppu::decode::decode(raw) else {
            decode_refusal = Some((state.pc, raw));
            break;
        };
        let descriptor = instruction.fuzz_descriptor(raw);
        deterministic &= requests_replay(descriptor.relations);
        decoded += 1;
        kinds.push(InstructionIdentity::Ppu(descriptor.kind));
        let verdict = execute(
            &instruction,
            &mut state,
            UNIT,
            &views,
            &mut effects,
            &mut stores,
        );
        terminal_verdict = Some(verdict.clone());
        match verdict {
            ExecuteVerdict::Continue => state.pc = state.pc.wrapping_add(4),
            ExecuteVerdict::Branch => {}
            ExecuteVerdict::Fault(_) | ExecuteVerdict::MemFault(_) => {
                // The runtime fault-discard rule hides state and effects from a faulting batch.
                state = initial.clone();
                effects.clear();
                stores.clear();
                break;
            }
            ExecuteVerdict::Syscall { .. } | ExecuteVerdict::BufferFull => break,
        }
    }
    stores.flush(&mut effects, UNIT);
    if state.clock_read {
        effects.push(Effect::ClockRead { source: UNIT });
    }
    (
        ObservedSequence {
            state: PpuStateSnapshot::capture(&state),
            pc: state.pc,
            vrsave: state.vrsave,
            vrsave_written: state.vrsave_written,
            tb: state.tb,
            terminal_verdict,
            decode_refusal,
            deterministic,
            effects,
        },
        decoded,
        kinds,
    )
}

fn random_state(rng: &mut Rng) -> PpuState {
    let mut state = PpuState::new();
    let mut gpr = [0u64; 32];
    let mut fpr = [0u64; 32];
    let mut vr = [0u128; 32];
    for value in &mut gpr {
        *value = if rng.chance(1, 3) {
            DATA_BASE + rng.below(DATA_LEN as u64)
        } else {
            rng.mixed_u64()
        };
    }
    for value in &mut fpr {
        *value = rng.fp_bits();
    }
    for value in &mut vr {
        *value = (u128::from(rng.next_u64()) << 64) | u128::from(rng.next_u64());
    }
    state.set_gpr_all(gpr);
    state.set_fpr_all(fpr);
    state.set_vr_all(vr);
    state.pc = rng.next_u64() & !3;
    state.set_cr(rng.next_u32());
    state.set_lr(rng.next_u64());
    state.set_ctr(rng.next_u64());
    state.set_xer(rng.next_u64());
    state.vrsave = rng.next_u32();
    // The executor treats a seeded VRSAVE value as initialized, as after `mtvrsave`.
    state.vrsave_written = true;
    state.tb = rng.next_u64();
    state.set_reservation(
        rng.chance(1, 2)
            .then(|| ReservedLine::containing(DATA_BASE + rng.below(DATA_LEN as u64))),
    );
    state
}

fn requests_replay(relations: &[PpuMetamorphicRelation]) -> bool {
    relations.contains(&PpuMetamorphicRelation::Deterministic)
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
#[path = "tests/ppu_tests.rs"]
mod tests;
