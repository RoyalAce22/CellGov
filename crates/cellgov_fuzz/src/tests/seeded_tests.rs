use super::*;

use std::collections::BTreeMap;
use std::path::Path;

use cellgov_effects::EffectKind;
use cellgov_ppu::state::PpuState;
use cellgov_spu::exec::SpuStepOutcome;
use cellgov_spu::state::{SpuObservableSnapshot, SpuState};

use crate::artifact::{
    ArtifactCheckSelection, ArtifactExecutionPolicy, ArtifactReductionRequest, ArtifactReference,
    ArtifactReplayError, FuzzFindingArtifact,
};
use crate::ppu_paths::{first_path_divergence, run_all_paths};
use crate::ppu_reference::{
    compare_reference as compare_ppu_reference, PpuReferenceComponent, PpuReferenceObservation,
    PpuReferenceState, PpuReferenceStop,
};
use crate::reduce::{
    classify_run, evaluate_case, reduce_finding, CandidateVerdict, ReductionPolicy,
    ReductionRequest, DEFAULT_REDUCTION_BUDGET,
};
use crate::reference::ReferenceField;
use crate::report::{CheckIdentity, DivergenceClass, FindingKind, FuzzReport, FuzzRun, RunOutcome};
use crate::semantic_sweep::{sweep_ppu, sweep_spu, SemanticSweepFinding, SemanticTargetStage};
use crate::spu_reference::{
    replay_reference as replay_spu_reference, SpuReferenceArtifact, SpuReferenceChannels,
    SpuReferenceComponent, SpuReferenceExpected, SpuReferenceInput, SpuReferenceOutcome,
    SpuReferenceProvenance, SPU_REFERENCE_SCHEMA_VERSION,
};
use crate::sweep::{ppu_decode_partition, spu_decode_partition};
use crate::{
    ppu, spu, CampaignSchedule, CampaignShard, CaseRange, FuzzConfig, FuzzTarget,
    GenerationStrategy, ReductionError, StateTransitionClass, TargetPanicPayload,
};

const CASES: u64 = 32;
const SEQUENCE_WORDS: u32 = 6;
const SEARCH_LIMIT: u64 = 512;

const TARGETS: [FuzzTarget; 4] = [
    FuzzTarget::PpuInstruction,
    FuzzTarget::PpuSequence,
    FuzzTarget::SpuInstruction,
    FuzzTarget::SpuSequence,
];
const INSTRUCTION_TARGETS: [FuzzTarget; 2] =
    [FuzzTarget::PpuInstruction, FuzzTarget::SpuInstruction];
const STRATEGIES: [GenerationStrategy; 2] =
    [GenerationStrategy::Structured, GenerationStrategy::RawWords];

const REQUEST: ReductionRequest = ReductionRequest {
    policy: ReductionPolicy::Deterministic,
    budget: DEFAULT_REDUCTION_BUDGET,
};

/// `ori 0,0,0`: a PPU instruction that changes no architectural state.
const PPU_NOP: u32 = 0x6000_0000;
/// `addi r3,r0,7`.
const PPU_ADDI_R3_7: u32 = 0x3860_0007;
/// `ai r3,r3,0`: an SPU instruction whose only footprint register keeps its value.
const SPU_AI_R3_R3_0: u32 = (0x1c << 24) | (3 << 7) | 3;
/// `ai r3,r4,5`.
const SPU_AI_R3_R4_5: u32 = (0x1c << 24) | (5 << 14) | (4 << 7) | 3;

fn config(strategy: GenerationStrategy) -> FuzzConfig {
    FuzzConfig {
        seed: 11,
        strategy,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: CASES,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        },
        max_findings: 64,
        sequence_words: SEQUENCE_WORDS,
        ..FuzzConfig::default()
    }
}

fn run(target: FuzzTarget, config: FuzzConfig) -> FuzzRun {
    match target {
        FuzzTarget::PpuInstruction => ppu::run_instructions(config),
        FuzzTarget::PpuSequence => ppu::run_sequences(config),
        FuzzTarget::SpuInstruction => spu::run_instructions(config),
        FuzzTarget::SpuSequence => spu::run_sequences(config),
    }
}

fn count(report: &FuzzReport, kind: FindingKind) -> u64 {
    report.finding_counts.get(&kind).copied().unwrap_or(0)
}

/// A run that reached no check is evidence for nothing, seeded or not.
fn assert_exercised(target: FuzzTarget, strategy: GenerationStrategy, report: &FuzzReport) {
    assert_eq!(
        report.cases, CASES,
        "{target:?} {strategy:?} considered every case"
    );
    assert!(
        report.decoded > 0,
        "{target:?} {strategy:?} decoded nothing"
    );
    assert!(
        report.eligible_cases > 0,
        "{target:?} {strategy:?} had no eligible case"
    );
    assert!(
        report.executed_steps >= report.eligible_cases,
        "{target:?} {strategy:?} executed {} steps for {} eligible cases",
        report.executed_steps,
        report.eligible_cases
    );
    assert!(
        !report.instruction_kinds.is_empty(),
        "{target:?} {strategy:?} reached no instruction kind"
    );
}

fn clean_baseline(target: FuzzTarget, strategy: GenerationStrategy) -> FuzzRun {
    let run = run(target, config(strategy));
    assert_exercised(target, strategy, &run.report);
    assert!(
        run.report.is_clean(),
        "{target:?} {strategy:?} baseline findings: {:?}",
        run.report.finding_counts
    );
    assert_eq!(
        run.outcome,
        RunOutcome::CleanCompletion,
        "{target:?} {strategy:?}"
    );
    run
}

/// Runs `target` with `defect` seeded and returns the run beside its clean baseline.
fn seeded_run(target: FuzzTarget, strategy: GenerationStrategy, defect: SeededDefect) -> FuzzRun {
    clean_baseline(target, strategy);
    let _guard = seed(defect);
    let run = run(target, config(strategy));
    assert_exercised(target, strategy, &run.report);
    run
}

/// The first generated case below the search limit whose single-case run satisfies `wanted`.
fn find_case(
    target: FuzzTarget,
    strategy: GenerationStrategy,
    wanted: impl Fn(&FuzzReport) -> bool,
) -> (u64, Vec<u32>) {
    let config = config(strategy);
    (0..SEARCH_LIMIT)
        .map(|index| {
            let words = crate::reduce::case_words(target, config, index).expect("case words");
            (index, words)
        })
        .find(|(index, words)| wanted(&evaluate_case(target, config, *index, words).report))
        .unwrap_or_else(|| panic!("{target:?} {strategy:?}: no case below {SEARCH_LIMIT} matches"))
}

/// `bi` with the reserved E and D options both set: an undefined operand
/// combination the structured generator never emits.
fn spu_undefined_word() -> u32 {
    let word = cellgov_spu::fuzz::generation_descriptors()
        .iter()
        .find(|descriptor| descriptor.kind == cellgov_spu::instruction::SpuInstructionKind::Bi)
        .expect("bi descriptor")
        .canonical_word
        | 0x000c_0000;
    assert!(cellgov_spu::fuzz::encoding_has_undefined_operands(word));
    word
}

fn retained_transition(run: &FuzzRun) -> StateTransitionClass {
    let mut entries = run.report.retained_cases.entries();
    let entry = entries.next().expect("a single-case run retains its case");
    entry.observation.state_transition
}

#[test]
fn seeding_is_scoped_to_the_guard_and_restores_the_previous_defect() {
    assert_eq!(active(), None);
    {
        let _outer = seed(SeededDefect::DecoderPanic);
        assert_eq!(active(), Some(SeededDefect::DecoderPanic));
        {
            let _inner = seed(SeededDefect::CommonMode);
            assert_eq!(active(), Some(SeededDefect::CommonMode));
        }
        assert_eq!(active(), Some(SeededDefect::DecoderPanic));
    }
    assert_eq!(active(), None);
}

#[test]
fn every_engine_and_strategy_runs_an_exercised_clean_baseline() {
    let mut effects = BTreeMap::new();
    for target in TARGETS {
        for strategy in STRATEGIES {
            let run = clean_baseline(target, strategy);
            let report = &run.report;
            effects.extend(report.effect_classes.clone());
            if matches!(target, FuzzTarget::PpuSequence | FuzzTarget::SpuSequence) {
                assert!(
                    report.max_executed_depth >= 2,
                    "{target:?} {strategy:?} never executed two instructions in one case"
                );
                assert!(
                    report.executed_steps >= 2 * report.eligible_cases,
                    "{target:?} {strategy:?} averaged under two instructions per eligible case"
                );
            }
            if strategy == GenerationStrategy::Structured && INSTRUCTION_TARGETS.contains(&target) {
                assert!(
                    !report.metamorphic_executions.is_empty(),
                    "{target:?} executed no metamorphic partner"
                );
            }
            assert!(
                report.instruction_kinds.len() >= 4,
                "{target:?} {strategy:?} reached {} kinds",
                report.instruction_kinds.len()
            );
        }
    }
    assert!(
        !effects.is_empty(),
        "no baseline campaign observed a guest-visible effect"
    );
}

#[test]
fn a_run_that_reaches_no_check_is_not_a_clean_completion() {
    let config = config(GenerationStrategy::RawWords);
    for (target, refused) in [
        (FuzzTarget::PpuInstruction, 0u32),
        (FuzzTarget::SpuInstruction, u32::MAX),
    ] {
        let run = evaluate_case(target, config, 0, &[refused]);
        assert_eq!(run.report.decoded, 0, "{target:?}");
        assert_eq!(run.report.eligible_cases, 0, "{target:?}");
        assert_eq!(run.report.executed_steps, 0, "{target:?}");
        assert!(run.report.is_clean(), "{target:?}");
        assert_eq!(run.outcome, RunOutcome::NoEligibleCases, "{target:?}");
    }
}

#[test]
fn a_seeded_decoder_panic_is_a_target_panic_finding_in_every_engine() {
    for target in TARGETS {
        for strategy in STRATEGIES {
            clean_baseline(target, strategy);
            let _guard = seed(SeededDefect::DecoderPanic);
            let run = run(target, config(strategy));
            assert_eq!(
                run.outcome,
                RunOutcome::TargetPanic,
                "{target:?} {strategy:?}"
            );
            assert_eq!(
                count(&run.report, FindingKind::TargetPanic),
                CASES,
                "{target:?} {strategy:?}"
            );
            // A structured sequence decodes inside the executor boundary; every
            // other engine decodes at the decoder boundary.
            let expected_check = match (target, strategy) {
                (FuzzTarget::PpuSequence, GenerationStrategy::Structured) => {
                    CheckIdentity::PpuExecutor
                }
                (FuzzTarget::SpuSequence, GenerationStrategy::Structured) => {
                    CheckIdentity::SpuExecutor
                }
                (FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence, _) => {
                    CheckIdentity::PpuDecoder
                }
                (FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence, _) => {
                    CheckIdentity::SpuDecoder
                }
            };
            for finding in &run.report.findings {
                assert_eq!(
                    finding.fingerprint.check, expected_check,
                    "{target:?} {strategy:?}"
                );
                assert_eq!(finding.fingerprint.divergence, DivergenceClass::TargetPanic);
                assert_eq!(
                    finding.panic_payload,
                    Some(TargetPanicPayload::StaticStr("seeded decoder panic".into()))
                );
            }
        }
    }
}

#[test]
fn a_seeded_executor_panic_is_a_target_panic_finding_in_every_engine() {
    for target in TARGETS {
        let run = seeded_run_without_exercise(target, SeededDefect::ExecutorPanic);
        assert_eq!(run.outcome, RunOutcome::TargetPanic, "{target:?}");
        assert_eq!(
            count(&run.report, FindingKind::TargetPanic),
            CASES,
            "{target:?}"
        );
        let expected_check = match target {
            FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence => CheckIdentity::PpuExecutor,
            FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence => CheckIdentity::SpuExecutor,
        };
        for finding in &run.report.findings {
            assert_eq!(finding.fingerprint.check, expected_check, "{target:?}");
            assert_eq!(
                finding.panic_payload,
                Some(TargetPanicPayload::StaticStr(
                    "seeded executor panic".into()
                ))
            );
        }
    }
}

/// A panic before execution leaves nothing eligible, so the exercise gate cannot apply.
fn seeded_run_without_exercise(target: FuzzTarget, defect: SeededDefect) -> FuzzRun {
    clean_baseline(target, GenerationStrategy::Structured);
    let _guard = seed(defect);
    let run = run(target, config(GenerationStrategy::Structured));
    assert_eq!(run.report.cases, CASES);
    assert_eq!(
        run.report.eligible_cases, 0,
        "{target:?} assessed a case that panicked"
    );
    run
}

#[test]
fn an_illegal_outcome_is_detected_on_every_eligible_instruction_case() {
    for target in INSTRUCTION_TARGETS {
        let run = seeded_run(
            target,
            GenerationStrategy::Structured,
            SeededDefect::IllegalOutcome,
        );
        assert_eq!(run.outcome, RunOutcome::SemanticFinding, "{target:?}");
        assert_eq!(
            count(&run.report, FindingKind::IllegalOutcome),
            run.report.eligible_cases,
            "{target:?}: one illegal outcome per eligible case and none elsewhere"
        );
        assert!(
            run.report.unsupported_cases + run.report.undefined_cases > 0,
            "{target:?}: the campaign met no inapplicable case, so the count proves nothing about them"
        );
        for finding in &run.report.findings {
            assert_eq!(finding.kind, FindingKind::IllegalOutcome);
            assert_eq!(finding.fingerprint.check, CheckIdentity::LegalOutcome);
            assert_eq!(finding.fingerprint.divergence, DivergenceClass::Outcome);
            assert!(finding.fingerprint.outcome.is_some());
        }
    }
}

#[test]
fn an_illegal_effect_is_detected_on_every_eligible_instruction_case() {
    for target in INSTRUCTION_TARGETS {
        let run = seeded_run(
            target,
            GenerationStrategy::Structured,
            SeededDefect::IllegalEffect,
        );
        assert_eq!(run.outcome, RunOutcome::SemanticFinding, "{target:?}");
        assert_eq!(
            count(&run.report, FindingKind::IllegalEffect),
            run.report.eligible_cases,
            "{target:?}"
        );
        for finding in &run.report.findings {
            assert_eq!(finding.kind, FindingKind::IllegalEffect);
            assert_eq!(finding.fingerprint.check, CheckIdentity::LegalEffect);
            assert_eq!(finding.fingerprint.divergence, DivergenceClass::Effect);
            assert!(matches!(
                finding.fingerprint.effect,
                Some(EffectKind::ClockRead | EffectKind::MailboxSend | EffectKind::TraceMarker)
            ));
        }
    }
}

#[test]
fn an_illegal_footprint_is_detected_on_spu_instructions_and_sequences() {
    for target in [FuzzTarget::SpuInstruction, FuzzTarget::SpuSequence] {
        let run = seeded_run(
            target,
            GenerationStrategy::Structured,
            SeededDefect::IllegalFootprint,
        );
        assert_eq!(run.outcome, RunOutcome::SemanticFinding, "{target:?}");
        assert_eq!(
            count(&run.report, FindingKind::IllegalFootprint),
            run.report.eligible_cases,
            "{target:?}"
        );
        assert_eq!(
            count(&run.report, FindingKind::Nondeterministic),
            0,
            "{target:?}: a defect in every run is not nondeterminism"
        );
        for finding in &run.report.findings {
            assert_eq!(finding.fingerprint.check, CheckIdentity::AllowedFootprint);
            assert_eq!(
                finding.fingerprint.divergence,
                DivergenceClass::ArchitecturalState
            );
        }
    }
}

#[test]
fn an_invalid_program_counter_is_detected_on_spu_sequences() {
    let run = seeded_run(
        FuzzTarget::SpuSequence,
        GenerationStrategy::Structured,
        SeededDefect::InvalidProgramCounter,
    );
    assert_eq!(run.outcome, RunOutcome::SemanticFinding);
    assert_eq!(
        count(&run.report, FindingKind::InvalidProgramCounter),
        run.report.eligible_cases
    );
    assert_eq!(count(&run.report, FindingKind::Nondeterministic), 0);
    for finding in &run.report.findings {
        assert_eq!(finding.fingerprint.check, CheckIdentity::ProgramCounter);
        assert_eq!(finding.fingerprint.divergence, DivergenceClass::ControlFlow);
    }
}

#[test]
fn nondeterminism_is_detected_in_every_engine() {
    for target in TARGETS {
        let run = seeded_run(
            target,
            GenerationStrategy::Structured,
            SeededDefect::Nondeterministic,
        );
        assert_eq!(run.outcome, RunOutcome::SemanticFinding, "{target:?}");
        let found = count(&run.report, FindingKind::Nondeterministic);
        assert!(found > 0, "{target:?}");
        assert!(
            found <= run.report.eligible_cases,
            "{target:?}: replay runs only for eligible cases"
        );
        assert_eq!(
            run.report.finding_counts.len(),
            1,
            "{target:?}: a replay defect trips no other check: {:?}",
            run.report.finding_counts
        );
        for finding in &run.report.findings {
            assert_eq!(
                finding.fingerprint.check,
                CheckIdentity::DeterministicReplay
            );
            assert_eq!(
                finding.fingerprint.divergence,
                DivergenceClass::ArchitecturalState
            );
        }
    }
}

#[test]
fn a_metamorphic_mismatch_is_detected_once_per_executed_relation() {
    for target in INSTRUCTION_TARGETS {
        let run = seeded_run(
            target,
            GenerationStrategy::Structured,
            SeededDefect::MetamorphicMismatch,
        );
        assert_eq!(run.outcome, RunOutcome::SemanticFinding, "{target:?}");
        let executed: u64 = run.report.metamorphic_executions.values().sum();
        assert!(executed > 0, "{target:?}");
        assert_eq!(
            count(&run.report, FindingKind::MetamorphicViolation),
            executed,
            "{target:?}"
        );
        assert_eq!(run.report.finding_counts.len(), 1, "{target:?}");
        for finding in &run.report.findings {
            assert_eq!(
                finding.fingerprint.divergence,
                DivergenceClass::ArchitecturalState
            );
            assert!(run
                .report
                .metamorphic_executions
                .contains_key(&finding.fingerprint.check));
        }
    }
}

#[test]
fn an_inapplicable_case_is_refused_before_any_seeded_defect_can_fire() {
    let defects = [
        SeededDefect::IllegalOutcome,
        SeededDefect::IllegalEffect,
        SeededDefect::IllegalFootprint,
        SeededDefect::Nondeterministic,
        SeededDefect::MetamorphicMismatch,
    ];
    for target in INSTRUCTION_TARGETS {
        let strategy = GenerationStrategy::Structured;
        // The structured SPU generator emits only defined operand combinations,
        // so its undefined case is a substituted word.
        let undefined = match target {
            FuzzTarget::SpuInstruction => (0, vec![spu_undefined_word()]),
            _ => find_case(target, strategy, |report| report.undefined_cases == 1),
        };
        let unsupported = find_case(target, strategy, |report| report.unsupported_cases == 1);
        for ((index, words), label, outcome) in [
            (undefined, "undefined", RunOutcome::UndefinedCase),
            (unsupported, "unsupported", RunOutcome::UnsupportedCase),
        ] {
            let plain = evaluate_case(target, config(strategy), index, &words);
            assert_eq!(plain.report.eligible_cases, 0, "{target:?} {label}");
            assert!(
                plain.report.undefined_cases + plain.report.unsupported_cases == 1,
                "{target:?} {label} case {index} is not inapplicable"
            );
            assert_eq!(plain.outcome, outcome, "{target:?} {label}");
            for defect in defects {
                let _guard = seed(defect);
                let run = evaluate_case(target, config(strategy), index, &words);
                assert_eq!(
                    run.report.eligible_cases, 0,
                    "{target:?} {label} case {index}"
                );
                assert!(
                    run.report.findings.is_empty(),
                    "{target:?} {label} case {index} produced {:?} under {defect:?}",
                    run.report.finding_counts
                );
                // The case keeps its inapplicability counter under the defect; a
                // dropped counter would read as no eligible case, not as clean.
                assert_eq!(run.outcome, outcome, "{target:?} {label} under {defect:?}");
            }
        }
    }
}

#[test]
fn a_common_mode_defect_is_invisible_to_every_self_differential_check() {
    for target in INSTRUCTION_TARGETS {
        let run = seeded_run(
            target,
            GenerationStrategy::Structured,
            SeededDefect::CommonMode,
        );
        assert!(
            run.report.is_clean(),
            "{target:?} self-checks saw the common-mode defect: {:?}",
            run.report.finding_counts
        );
        assert_eq!(run.outcome, RunOutcome::CleanCompletion, "{target:?}");
        assert!(
            !run.report.metamorphic_executions.is_empty(),
            "{target:?}: the metamorphic tier did not run, so its silence is not evidence"
        );
    }
    // The defect reached the engines: a state-preserving instruction now
    // changes architectural state, and only the seeding differs between runs.
    let strategy = GenerationStrategy::Structured;
    for (target, word) in [
        (FuzzTarget::PpuInstruction, PPU_NOP),
        (FuzzTarget::SpuInstruction, SPU_AI_R3_R3_0),
    ] {
        let plain = evaluate_case(target, config(strategy), 0, &[word]);
        assert_eq!(plain.report.eligible_cases, 1, "{target:?}");
        assert_eq!(
            retained_transition(&plain),
            StateTransitionClass::Unchanged,
            "{target:?}"
        );
        let _guard = seed(SeededDefect::CommonMode);
        let corrupted = evaluate_case(target, config(strategy), 0, &[word]);
        assert!(corrupted.report.is_clean(), "{target:?}");
        assert_eq!(
            retained_transition(&corrupted),
            StateTransitionClass::ArchitecturalState,
            "{target:?}"
        );
    }
}

fn unsupported<T>() -> ReferenceField<T> {
    ReferenceField::Unsupported {
        reason: "not under test".into(),
    }
}

fn ppu_state_reference(gpr: Vec<u64>) -> PpuReferenceState {
    PpuReferenceState {
        gpr: ReferenceField::Value { value: gpr },
        fpr: unsupported(),
        vr_hex: unsupported(),
        pc: unsupported(),
        cr: unsupported(),
        lr: unsupported(),
        ctr: unsupported(),
        xer: unsupported(),
        vrsave: unsupported(),
        tb: unsupported(),
        reservation: unsupported(),
    }
}

fn ppu_reference(gpr: Vec<u64>) -> PpuReferenceObservation {
    PpuReferenceObservation {
        state: ppu_state_reference(gpr),
        memory: unsupported(),
        stop: PpuReferenceStop {
            reason: unsupported(),
            fault: unsupported(),
            pc: unsupported(),
            lr: unsupported(),
            syscall_lev: unsupported(),
            faulting_ea: unsupported(),
            fault_registers: unsupported(),
            syscall_args: unsupported(),
        },
        retired: unsupported(),
        staged_effects: unsupported(),
        committed_effects: unsupported(),
        reservations: unsupported(),
        store_buffer: unsupported(),
        commit_error: unsupported(),
        fault_discarded: unsupported(),
    }
}

#[test]
fn a_common_mode_ppu_defect_is_visible_to_an_independent_reference() {
    let runs = run_all_paths(&[PPU_ADDI_R3_7], &PpuState::new(), &[0; 64]).expect("paths");
    let truth = runs.first().expect("one path");
    assert_eq!(truth.observation.state.gpr[3], 7);
    let expected = ppu_reference(truth.observation.state.gpr.to_vec());
    assert!(runs
        .iter()
        .all(|run| compare_ppu_reference(&expected, run).is_match()));

    let _guard = seed(SeededDefect::CommonMode);
    let corrupted = run_all_paths(&[PPU_ADDI_R3_7], &PpuState::new(), &[0; 64]).expect("paths");
    assert_eq!(corrupted.len(), runs.len());
    // Every internal path carries the same defect, so the paths still agree
    // with each other; only the reference disagrees.
    assert!(first_path_divergence(&corrupted).is_none());
    for run in &corrupted {
        assert_ne!(run.observation.state.gpr, truth.observation.state.gpr);
        let comparison = compare_ppu_reference(&expected, run);
        assert!(!comparison.is_match(), "{:?}", run.path);
        assert_eq!(comparison.differences.len(), 1);
        assert_eq!(
            comparison.differences[0].field,
            PpuReferenceComponent::StateGpr
        );
    }
}

fn hex(register: &[u8; 16]) -> String {
    register.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn word(register: &[u8; 16]) -> u32 {
    u32::from_be_bytes([register[0], register[1], register[2], register[3]])
}

/// `ai r3,r4,5` with r4 holding 10 in every word slot.
fn spu_artifact(expected: SpuReferenceExpected) -> SpuReferenceArtifact {
    let mut initial = SpuState::new();
    initial.set_reg_word_splat(4, 10);
    SpuReferenceArtifact {
        schema_version: SPU_REFERENCE_SCHEMA_VERSION,
        case_id: "seeded common-mode ai".into(),
        // [SPU-ISA p:61 s:5. Integer and Logical Instructions] ai adds the
        // sign-extended I10 field to each word of RA and writes RT.
        provenance: SpuReferenceProvenance::DocumentedVector {
            citation: "SPU-ISA p:61 s:5. Integer and Logical Instructions".into(),
            vector_id: "ai-r3-r4-5".into(),
        },
        words: vec![SPU_AI_R3_R4_5],
        initial_state: SpuReferenceInput {
            regs_hex: BTreeMap::from([("4".to_string(), hex(&initial.regs[4]))]),
            local_store: BTreeMap::new(),
            pc: 0,
            channels: None,
            reservation: None,
        },
        expected,
    }
}

fn spu_reference(truth: &SpuObservableSnapshot) -> SpuReferenceExpected {
    SpuReferenceExpected {
        regs_hex: ReferenceField::Value {
            value: truth
                .regs
                .iter()
                .enumerate()
                .map(|(index, register)| (index.to_string(), hex(register)))
                .collect(),
        },
        local_store: unsupported(),
        pc: ReferenceField::Value { value: truth.pc },
        channels: ReferenceField::Value {
            value: SpuReferenceChannels::from(&truth.channels),
        },
        reservation: ReferenceField::Value {
            value: truth.reservation.map(|line| line.addr()),
        },
        outcome: ReferenceField::Value {
            value: SpuReferenceOutcome::Continue,
        },
        effects: ReferenceField::Value { value: Vec::new() },
        fault_discarded: ReferenceField::Value { value: false },
    }
}

#[test]
fn a_common_mode_spu_defect_is_visible_to_an_independent_reference() {
    // A placeholder expectation replays the vector once to learn the truth.
    let placeholder = SpuReferenceExpected {
        regs_hex: unsupported(),
        local_store: unsupported(),
        pc: unsupported(),
        channels: unsupported(),
        reservation: unsupported(),
        outcome: unsupported(),
        effects: unsupported(),
        fault_discarded: unsupported(),
    };
    let truth = replay_spu_reference(&spu_artifact(placeholder)).expect("replays");
    assert_eq!(truth.outcome, SpuStepOutcome::Continue);
    assert_eq!(word(&truth.state.regs[3]), 15);
    let artifact = spu_artifact(spu_reference(&truth.state));
    assert!(replay_spu_reference(&artifact)
        .expect("replays")
        .comparison
        .is_match());

    let _guard = seed(SeededDefect::CommonMode);
    let corrupted = replay_spu_reference(&artifact).expect("replays");
    assert_eq!(corrupted.outcome, SpuStepOutcome::Continue);
    assert_ne!(
        corrupted.state.regs[3], truth.state.regs[3],
        "the footprint register carries the wrong value"
    );
    assert_eq!(corrupted.state.regs[4], truth.state.regs[4]);
    assert!(!corrupted.comparison.is_match());
    assert_eq!(
        corrupted.comparison.differences,
        [SpuReferenceComponent::Registers].into_iter().collect()
    );
}

#[test]
fn sweeps_record_a_seeded_decoder_panic_for_every_word() {
    assert!(ppu_decode_partition(0..=7).panics.is_empty());
    assert!(spu_decode_partition(0..=7).panics.is_empty());
    let (plain_ppu, plain_spu) = (
        sweep_ppu(&cellgov_ppu::instruction::fuzz::generation_descriptors()),
        sweep_spu(&cellgov_spu::fuzz::generation_descriptors()),
    );
    assert!(plain_ppu.is_clean() && plain_spu.is_clean());

    let _guard = seed(SeededDefect::DecoderPanic);
    for report in [ppu_decode_partition(0..=7), spu_decode_partition(0..=7)] {
        assert_eq!(report.panics.len(), 8);
        assert_eq!(report.accepted + report.refused, 0);
        assert!(report.panics.iter().enumerate().all(|(index, panic)| {
            panic.raw == index as u32
                && panic.payload == TargetPanicPayload::StaticStr("seeded decoder panic".into())
        }));
    }
    for report in [
        sweep_ppu(&cellgov_ppu::instruction::fuzz::generation_descriptors()),
        sweep_spu(&cellgov_spu::fuzz::generation_descriptors()),
    ] {
        assert!(!report.is_clean());
        assert!(report.witnesses.is_empty());
        assert!(report.findings.iter().all(|finding| matches!(
            finding,
            SemanticSweepFinding::TargetPanic {
                stage: SemanticTargetStage::Decoder,
                ..
            } | SemanticSweepFinding::UnwitnessedKind { .. }
        )));
        assert!(report.findings.iter().any(|finding| matches!(
            finding,
            SemanticSweepFinding::TargetPanic {
                stage: SemanticTargetStage::Decoder,
                ..
            }
        )));
    }
}

#[test]
fn reduction_keeps_a_seeded_finding_and_refuses_it_once_the_defect_is_gone() {
    for (target, defect) in [
        (FuzzTarget::PpuInstruction, SeededDefect::IllegalOutcome),
        (FuzzTarget::SpuSequence, SeededDefect::IllegalFootprint),
    ] {
        let config = config(GenerationStrategy::Structured);
        let finding = {
            let _guard = seed(defect);
            let run = run(target, config);
            run.report
                .findings
                .first()
                .cloned()
                .expect("a seeded finding")
        };
        let report = {
            let _guard = seed(defect);
            let report = reduce_finding(config, &finding, REQUEST).expect("reduction succeeds");
            assert!(!report.finding_words.is_empty(), "{target:?}");
            assert!(report.fixpoint, "{target:?}");
            let final_run = evaluate_case(
                target,
                config,
                finding.replay.case_index,
                &report.case_words,
            );
            assert_eq!(
                classify_run(&finding, &final_run).expect("evaluated"),
                CandidateVerdict::Reproduced {
                    finding_words: report.finding_words.clone()
                },
                "{target:?}"
            );
            report
        };
        if target == FuzzTarget::SpuSequence {
            assert!(report.reduced(), "{target:?}: no word was dropped");
            assert!(report.case_words.len() < finding.original_words.len());
        }
        assert_eq!(
            reduce_finding(config, &finding, REQUEST).unwrap_err(),
            ReductionError::OriginalNotReproduced,
            "{target:?}: the reproducer depends on the seeded defect"
        );
    }
}

#[test]
fn an_artifact_replays_a_seeded_finding_and_refuses_it_once_the_defect_is_gone() {
    let config = config(GenerationStrategy::Structured);
    let policy = ArtifactExecutionPolicy {
        workers: 1,
        deadline_ms: None,
        progress: false,
        check: ArtifactCheckSelection::All,
        reduction: ArtifactReductionRequest::None,
    };
    for (target, defect, kind) in [
        (
            FuzzTarget::SpuInstruction,
            SeededDefect::IllegalEffect,
            FindingKind::IllegalEffect,
        ),
        (
            FuzzTarget::PpuSequence,
            SeededDefect::Nondeterministic,
            FindingKind::Nondeterministic,
        ),
    ] {
        let artifact = {
            let _guard = seed(defect);
            let run = run(target, config);
            let finding = run.report.findings.first().expect("a seeded finding");
            assert_eq!(finding.kind, kind);
            FuzzFindingArtifact::from_finding(
                config,
                policy,
                &run.report,
                finding,
                ArtifactReference::Local,
                Path::new("evidence/seeded.json"),
            )
            .expect("artifact")
        };
        {
            let _guard = seed(defect);
            let replayed = artifact.replay().expect("the seeded finding replays");
            assert_eq!(replayed.kind, kind, "{target:?}");
            assert_eq!(replayed.replay, artifact.original.replay);
        }
        assert!(
            matches!(
                artifact.replay(),
                Err(ArtifactReplayError::NotReproduced { case_index })
                    if case_index == artifact.original.replay.case_index
            ),
            "{target:?}: the artifact replays without its defect"
        );
    }
}
