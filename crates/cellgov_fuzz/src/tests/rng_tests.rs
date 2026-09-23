use super::*;

#[test]
fn accepted_word_generation_reports_bound_exhaustion() {
    let mut rng = Rng::for_iter(7, 0);

    assert!(matches!(
        rng.decoder_accepted_words(1, |_| Ok(false)),
        Err(WordGenerationFailure::Exhausted(_))
    ));
}

#[test]
fn accepted_word_generation_reports_decoder_panics() {
    let mut rng = Rng::for_iter(7, 0);

    assert!(matches!(
        rng.decoder_accepted_words(1, |_| Err(crate::TargetPanicPayload::NonString)),
        Err(WordGenerationFailure::DecoderPanic(..))
    ));
}

#[test]
fn invalid_probability_bounds_are_typed_refusals() {
    let mut rng = Rng::for_iter(7, 0);

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
