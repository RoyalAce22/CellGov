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
        rng.decoder_accepted_words(1, |_| Err(())),
        Err(WordGenerationFailure::DecoderPanic(_))
    ));
}
