use super::*;
use crate::{ConfigurationError, RunOutcome};

#[test]
fn a_zero_word_spu_sequence_is_an_invalid_configuration() {
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
    let run = run_sequences(FuzzConfig {
        sequence_words: (SPU_LS_SIZE / 4 + 1) as u32,
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::SequenceTooLong { .. }
        ))
    ));
}
