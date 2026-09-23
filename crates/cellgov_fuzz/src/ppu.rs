//! PPU fuzz engines built on interpreter-owned descriptors.

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

use crate::boundary::{call_harness, call_target};
use crate::error::{FuzzError, GeneratorError, InvariantError};
use crate::report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, ReductionOutcome, SemanticFingerprint,
};
use crate::rng::{Rng, WordGenerationFailure};
use crate::{FuzzConfig, ReplayCoordinates, TargetPanicPayload};

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
pub fn run_instructions(config: FuzzConfig) -> FuzzRun {
    guarded_run(FuzzTarget::PpuInstruction, config, |report| {
        run_instructions_inner(config, report)
    })
}

fn run_instructions_inner(config: FuzzConfig, report: &mut FuzzReport) -> Result<(), FuzzError> {
    config.validate(None)?;
    for iteration in config.case_indices()? {
        report.considered()?;
        let mut rng = Rng::for_case(config.campaign_version, config.seed, iteration);
        let raw = rng.next_u32();
        let decoded = match call_target(|| cellgov_ppu::decode::decode(raw)) {
            Ok(decoded) => decoded,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuDecoder,
                    None,
                    vec![raw],
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        let Ok(instruction) = decoded else { continue };
        let descriptor = instruction.fuzz_descriptor(raw);
        let identity = InstructionIdentity::Ppu(descriptor.kind);
        report.reached(identity)?;
        let initial = random_state(&mut rng)?;
        let mut memory = vec![0u8; DATA_LEN];
        rng.fill(&mut memory);
        let first = match call_target(|| run_once(&instruction, &initial, &memory)) {
            Ok(first) => first,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuExecutor,
                    Some(identity),
                    vec![raw],
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        if requests_replay(descriptor.relations) {
            let second = match call_target(|| run_once(&instruction, &initial, &memory)) {
                Ok(second) => second,
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::PpuExecutor,
                        Some(identity),
                        vec![raw],
                        iteration,
                        payload,
                    )?;
                    continue;
                }
            };
            if first != second {
                record(
                    report,
                    FindingKind::Nondeterministic,
                    SemanticFingerprint {
                        target: FuzzTarget::PpuInstruction,
                        instruction_kind: Some(identity),
                        check: CheckIdentity::DeterministicReplay,
                        divergence: DivergenceClass::ArchitecturalState,
                        outcome: None,
                        effect: None,
                    },
                    vec![raw],
                    iteration,
                )?;
            }
        }
        let outcome = PpuOutcomeClass::from_verdict(&first.verdict);
        if !descriptor.outcomes.contains(&outcome) {
            record(
                report,
                FindingKind::IllegalOutcome,
                SemanticFingerprint {
                    target: FuzzTarget::PpuInstruction,
                    instruction_kind: Some(identity),
                    check: CheckIdentity::LegalOutcome,
                    divergence: DivergenceClass::Outcome,
                    outcome: Some(outcome_identity(outcome)),
                    effect: None,
                },
                vec![raw],
                iteration,
            )?;
        }
        if let Some(effect) = first
            .effects
            .iter()
            .map(Effect::kind)
            .find(|effect| !descriptor.effects.contains(effect))
        {
            record(
                report,
                FindingKind::IllegalEffect,
                SemanticFingerprint {
                    target: FuzzTarget::PpuInstruction,
                    instruction_kind: Some(identity),
                    check: CheckIdentity::LegalEffect,
                    divergence: DivergenceClass::Effect,
                    outcome: None,
                    effect: Some(effect),
                },
                vec![raw],
                iteration,
            )?;
        }
    }
    Ok(())
}

/// Replays PPU instruction sequences to find nondeterministic outcomes.
pub fn run_sequences(config: FuzzConfig) -> FuzzRun {
    guarded_run(FuzzTarget::PpuSequence, config, |report| {
        run_sequences_inner(config, report)
    })
}

fn run_sequences_inner(config: FuzzConfig, report: &mut FuzzReport) -> Result<(), FuzzError> {
    config.validate(Some(crate::MAX_SEQUENCE_WORDS))?;
    for iteration in config.case_indices()? {
        report.considered()?;
        let mut rng = Rng::for_case(config.campaign_version, config.seed, iteration);
        let words = rng.decoder_accepted_words(config.sequence_words as usize, |raw| {
            call_target(|| cellgov_ppu::decode::decode(raw)).map(|decoded| decoded.is_ok())
        });
        let words = match words {
            Ok(words) => words,
            Err(WordGenerationFailure::DecoderPanic(raw, payload)) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuDecoder,
                    None,
                    vec![raw],
                    iteration,
                    payload,
                )?;
                continue;
            }
            Err(WordGenerationFailure::Exhausted(error)) => return Err(error.into()),
        };
        let mut initial = random_state(&mut rng)?;
        initial.pc = 0;
        let mut memory = vec![0u8; DATA_LEN];
        rng.fill(&mut memory);
        if words.is_empty() {
            return Err(InvariantError::EmptyGeneratedSequence.into());
        }
        let first = match call_target(|| run_sequence(&words, &initial, &memory)) {
            Ok(first) => first,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuExecutor,
                    None,
                    words.clone(),
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        report.reached_many(first.1, first.2.iter().copied())?;
        if first.0.deterministic {
            let second = match call_target(|| run_sequence(&words, &initial, &memory)) {
                Ok(second) => second,
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::PpuExecutor,
                        None,
                        words.clone(),
                        iteration,
                        payload,
                    )?;
                    continue;
                }
            };
            if first.0 != second.0 {
                record(
                    report,
                    FindingKind::Nondeterministic,
                    SemanticFingerprint {
                        target: FuzzTarget::PpuSequence,
                        instruction_kind: None,
                        check: CheckIdentity::DeterministicReplay,
                        divergence: DivergenceClass::ArchitecturalState,
                        outcome: None,
                        effect: None,
                    },
                    words.clone(),
                    iteration,
                )?;
            }
        }
    }
    Ok(())
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

fn random_state(rng: &mut Rng) -> Result<PpuState, GeneratorError> {
    let mut state = PpuState::new();
    let mut gpr = [0u64; 32];
    let mut fpr = [0u64; 32];
    let mut vr = [0u128; 32];
    for value in &mut gpr {
        *value = if rng.chance(1, 3)? {
            DATA_BASE + rng.below(DATA_LEN as u64)?
        } else {
            rng.mixed_u64()?
        };
    }
    for value in &mut fpr {
        *value = rng.fp_bits()?;
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
    state.set_reservation(if rng.chance(1, 2)? {
        Some(ReservedLine::containing(
            DATA_BASE + rng.below(DATA_LEN as u64)?,
        ))
    } else {
        None
    });
    Ok(state)
}

fn requests_replay(relations: &[PpuMetamorphicRelation]) -> bool {
    relations.contains(&PpuMetamorphicRelation::Deterministic)
}

fn outcome_identity(outcome: PpuOutcomeClass) -> OutcomeIdentity {
    match outcome {
        PpuOutcomeClass::Continue => OutcomeIdentity::PpuContinue,
        PpuOutcomeClass::Branch => OutcomeIdentity::PpuBranch,
        PpuOutcomeClass::Syscall => OutcomeIdentity::PpuSyscall,
        PpuOutcomeClass::Fault => OutcomeIdentity::PpuFault,
        PpuOutcomeClass::MemoryFault => OutcomeIdentity::PpuMemoryFault,
        PpuOutcomeClass::BufferFull => OutcomeIdentity::PpuBufferFull,
    }
}

fn record(
    report: &mut FuzzReport,
    kind: FindingKind,
    fingerprint: SemanticFingerprint,
    original_words: Vec<u32>,
    iteration: u64,
) -> Result<(), InvariantError> {
    report.finding(Finding {
        fingerprint,
        kind,
        replay: ReplayCoordinates::new(
            report.target,
            report.seed,
            iteration,
            report.sequence_words,
        ),
        original_words,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    })
}

fn record_target_panic(
    report: &mut FuzzReport,
    check: CheckIdentity,
    instruction_kind: Option<InstructionIdentity>,
    original_words: Vec<u32>,
    iteration: u64,
    payload: TargetPanicPayload,
) -> Result<(), InvariantError> {
    report.finding(Finding {
        fingerprint: SemanticFingerprint {
            target: report.target,
            instruction_kind,
            check,
            divergence: DivergenceClass::TargetPanic,
            outcome: None,
            effect: None,
        },
        kind: FindingKind::TargetPanic,
        replay: ReplayCoordinates::new(
            report.target,
            report.seed,
            iteration,
            report.sequence_words,
        ),
        original_words,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: Some(payload),
    })
}

fn guarded_run(
    target: FuzzTarget,
    config: FuzzConfig,
    run: impl FnOnce(&mut FuzzReport) -> Result<(), FuzzError>,
) -> FuzzRun {
    let mut report = FuzzReport::new(
        target,
        config.seed,
        config.max_findings as usize,
        config.sequence_words,
    );
    match call_harness(|| run(&mut report)) {
        Ok(Ok(())) if config.schedule.is_cancelled() && report.is_clean() => {
            FuzzRun::cancelled(report)
        }
        Ok(Ok(())) => FuzzRun::completed(report),
        Ok(Err(error)) => FuzzRun::failed(report, error),
        Err(_) => FuzzRun::failed(
            report,
            InvariantError::UnexpectedPanic {
                stage: "PPU campaign",
            },
        ),
    }
}

#[cfg(test)]
#[path = "tests/ppu_tests.rs"]
mod tests;
