//! Integration tests for the public fuzz-engine API.

use cellgov_fuzz::{
    ppu, spu, CampaignSchedule, CaseRange, FuzzConfig, GenerationStrategy, InstructionIdentity,
    RunOutcome,
};
use cellgov_spu::instruction::SpuInstructionKind;

fn small_config() -> FuzzConfig {
    FuzzConfig {
        seed: 7,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 16,
            },
            ..CampaignSchedule::default()
        },
        max_findings: 4,
        sequence_words: 4,
        ..FuzzConfig::default()
    }
}

#[test]
fn engines_are_deterministic_library_calls() {
    let config = small_config();
    assert_eq!(ppu::run_instructions(config), ppu::run_instructions(config));
    assert_eq!(ppu::run_sequences(config), ppu::run_sequences(config));
    assert_eq!(spu::run_instructions(config), spu::run_instructions(config));
    assert_eq!(spu::run_sequences(config), spu::run_sequences(config));
}

#[test]
fn concurrent_runs_keep_independent_results() {
    let config = small_config();
    let expected = ppu::run_instructions(config);
    let workers = (0..4)
        .map(|_| std::thread::spawn(move || ppu::run_instructions(config)))
        .collect::<Vec<_>>();

    for worker in workers {
        assert_eq!(worker.join().expect("worker must not panic"), expected);
    }
}

#[test]
fn structured_instruction_campaigns_decode_every_generated_case() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 2_048,
            },
            ..CampaignSchedule::default()
        },
        ..small_config()
    };

    for run in [ppu::run_instructions(config), spu::run_instructions(config)] {
        assert!(!matches!(run.outcome, RunOutcome::HarnessFailure(_)));
        assert_eq!(run.report.cases, 2_048);
        assert_eq!(run.report.decoded, 2_048);
    }
}

#[test]
fn raw_word_campaigns_remain_a_separate_decoder_strategy() {
    let config = FuzzConfig {
        strategy: GenerationStrategy::RawWords,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 256,
            },
            ..CampaignSchedule::default()
        },
        ..small_config()
    };
    let ppu_run = ppu::run_instructions(config);
    let spu_run = spu::run_instructions(config);

    assert_eq!(ppu_run.report.strategy, GenerationStrategy::RawWords);
    assert_eq!(spu_run.report.strategy, GenerationStrategy::RawWords);
    assert!(ppu_run.report.decoded < ppu_run.report.cases);
    assert!(spu_run.report.decoded < spu_run.report.cases);
}

#[test]
fn structured_spu_constraints_retry_without_aborting_the_campaign() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 4_096,
            },
            ..CampaignSchedule::default()
        },
        ..small_config()
    };
    let run = spu::run_instructions(config);

    assert!(!matches!(
        run.outcome,
        RunOutcome::HarnessFailure(_) | RunOutcome::TargetPanic
    ));
    assert_eq!(run.report.cases, 4_096);
    assert_eq!(run.report.decoded, 4_096);
    for kind in [
        SpuInstructionKind::Bi,
        SpuInstructionKind::Bisl,
        SpuInstructionKind::Biz,
        SpuInstructionKind::Binz,
        SpuInstructionKind::Bihz,
        SpuInstructionKind::Bihnz,
        SpuInstructionKind::Hbr,
        SpuInstructionKind::Hbra,
        SpuInstructionKind::Hbrr,
    ] {
        assert!(
            run.report
                .instruction_kinds
                .contains(&InstructionIdentity::Spu(kind)),
            "structured campaign did not reach {kind:?}"
        );
    }
}
