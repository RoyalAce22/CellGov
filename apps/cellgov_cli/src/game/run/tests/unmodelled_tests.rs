use super::{census_label, pup_hex};
use cellgov_ps3_abi::lv2::census::PupCensusClass;

#[test]
fn unknown_census_state_is_reported_not_silently_absent() {
    assert_eq!(census_label(PupCensusClass::NotExtracted), "not_extracted");
}

#[test]
fn pup_digest_uses_the_archive_key_spelling() {
    assert_eq!(pup_hex(&[0xab; 32]), "ab".repeat(32));
}
