use super::*;
use cellgov_spu::instruction::SpuInstructionKind;

use crate::campaign::{CampaignSchedule, CaseRange};
use crate::{ConfigurationError, RunOutcome};

fn schedule(count: u64) -> CampaignSchedule {
    CampaignSchedule {
        cases: CaseRange { first: 0, count },
        ..CampaignSchedule::default()
    }
}

#[test]
fn a_substituted_word_replaces_the_generated_word_of_every_case() {
    let nop = 0x4020_007fu32;
    let config = FuzzConfig {
        schedule: schedule(3),
        ..FuzzConfig::default()
    };

    let run = run_instructions_with(config, Some(&[nop]));

    assert_eq!(run.report.cases, 3);
    assert_eq!(
        run.report.instruction_kinds,
        BTreeSet::from([InstructionIdentity::Spu(SpuInstructionKind::Nop)])
    );
}

#[test]
fn raw_case_words_are_the_generator_draw_and_structured_case_words_decode() {
    let raw = FuzzConfig {
        strategy: GenerationStrategy::RawWords,
        ..FuzzConfig::default()
    };
    for case in [0u64, 1, 4_095] {
        let mut rng = Rng::for_case(raw.campaign_version, raw.seed, case);
        assert_eq!(
            instruction_case_words(raw, case).expect("raw case words"),
            vec![rng.next_u32()],
            "case {case}"
        );
    }

    let structured = FuzzConfig {
        strategy: GenerationStrategy::Structured,
        ..FuzzConfig::default()
    };
    for case in 0..64u64 {
        let words = instruction_case_words(structured, case).expect("structured case words");
        assert_eq!(words.len(), 1, "case {case}");
        assert!(
            cellgov_spu::decode::decode(words[0]).is_ok(),
            "case {case} word {:#010x}",
            words[0]
        );
    }
}

#[test]
fn a_zero_case_instruction_campaign_is_a_typed_refusal() {
    let run = run_instructions(FuzzConfig {
        schedule: schedule(0),
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(ConfigurationError::ZeroIterations))
    ));
    assert_eq!(run.report.cases, 0);
}
