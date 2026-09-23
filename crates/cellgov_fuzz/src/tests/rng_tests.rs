use super::*;

#[test]
fn campaign_version_two_preserves_the_seed_index_mapping() {
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 7, 0);

    assert_eq!(rng.next_u64(), 0x044c_3cd7_f43c_661c);
}

#[test]
fn accepted_word_generation_reports_bound_exhaustion() {
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 7, 0);

    assert!(matches!(
        rng.decoder_accepted_words(1, |_| Ok(false)),
        Err(WordGenerationFailure::Exhausted(_))
    ));
}

#[test]
fn accepted_word_generation_reports_decoder_panics() {
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 7, 0);

    assert!(matches!(
        rng.decoder_accepted_words(1, |_| Err(crate::TargetPanicPayload::NonString)),
        Err(WordGenerationFailure::DecoderPanic(..))
    ));
}

#[test]
fn invalid_probability_bounds_are_typed_refusals() {
    let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 7, 0);

    assert_eq!(
        rng.chance(2, 1),
        Err(GeneratorError::InvalidProbability {
            numerator: 2,
            denominator: 1,
        })
    );
    assert_eq!(
        rng.chance(0, 0),
        Err(GeneratorError::ZeroProbabilityDenominator)
    );
}
