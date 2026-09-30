//! Every committed SPU reference file replays and matches, and every
//! architectural unit has one file or one pending entry.

use cellgov_fuzz::spu_reference::{run_reference_directory, SpuReferenceFileOutcome};

#[test]
fn every_committed_reference_matches_and_every_unit_is_covered_or_pending() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spu_reference");
    let campaign = run_reference_directory(&dir).expect("the fixture directory reads");
    assert!(!campaign.files.is_empty());
    for run in &campaign.files {
        match &run.outcome {
            SpuReferenceFileOutcome::Refused(error) => panic!("{}: {error}", run.file),
            outcome => assert!(outcome.is_match(), "{}: {outcome:?}", run.file),
        }
    }
    let completeness = &campaign.completeness;
    assert!(
        completeness.is_complete(),
        "missing:\n{}\n{completeness:?}",
        completeness.missing.join("\n")
    );
    assert_eq!(
        completeness.covered + completeness.pending,
        completeness.units
    );
}
