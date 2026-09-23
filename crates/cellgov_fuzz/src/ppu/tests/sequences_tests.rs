use super::*;
use crate::campaign::{CampaignSchedule, CaseRange};
use crate::{ConfigurationError, RunOutcome};

#[test]
fn a_zero_word_ppu_sequence_is_an_invalid_configuration() {
    let run = run_sequences(FuzzConfig {
        sequence_words: 0,
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::ZeroSequenceWords
        ))
    ));
    assert_eq!(run.report.cases, 0);
}

#[test]
fn oversized_sequences_are_typed_refusals() {
    // One case: a run that accepts the oversized config fuzzes one case, then
    // fails at the assertion. The default schedule is a million cases.
    let run = run_sequences(FuzzConfig {
        sequence_words: (crate::MAX_SEQUENCE_WORDS + 1) as u32,
        schedule: CampaignSchedule {
            cases: CaseRange { first: 0, count: 1 },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::SequenceTooLong { .. }
        ))
    ));
    assert_eq!(run.report.cases, 0);
}
