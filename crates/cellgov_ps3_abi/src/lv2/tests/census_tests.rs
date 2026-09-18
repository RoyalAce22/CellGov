use super::*;

#[test]
fn an_unknown_pup_is_not_extracted() {
    assert_eq!(lookup(&[0; 32], 0), PupCensusClass::NotExtracted);
}

#[test]
fn every_generated_pup_census_has_the_dispatch_width() {
    assert_eq!(PUP_CENSUS.len(), 97);
    assert!(PUP_CENSUS.iter().all(|row| row.classes.len() == 1024));
}

#[test]
fn a_known_pup_distinguishes_its_census_from_an_out_of_range_ordinal() {
    let row = PUP_CENSUS.first().expect("generated table has a PUP row");
    assert_eq!(lookup(&row.pup_sha256, 0), class_from_byte(row.classes[0]));
    assert_eq!(
        lookup(&row.pup_sha256, row.classes.len()),
        PupCensusClass::OutOfRange
    );
}

#[test]
#[should_panic(expected = "invalid generated census class 3")]
fn an_invalid_generated_class_is_loud() {
    let _ = class_from_byte(3);
}
