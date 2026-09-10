use super::*;

#[test]
fn the_version_payload_reads_as_the_store_key() {
    assert_eq!(parse_pup_version_txt("4.93\n").as_deref(), Some("4.93"));
    assert_eq!(parse_pup_version_txt("2.76"), Some("2.76".to_string()));
    assert_eq!(
        parse_pup_version_txt("1.50\nbuild:20061106\n").as_deref(),
        Some("1.50")
    );
}

#[test]
fn a_zero_padded_major_spells_the_same_version_as_the_installed_tree() {
    assert_eq!(parse_pup_version_txt("04.93").as_deref(), Some("4.93"));
    assert_eq!(parse_pup_version_txt("00.90").as_deref(), Some("0.90"));
}

#[test]
fn a_payload_that_is_not_one_version_names_no_key() {
    for text in [
        "", "\n", "4", "4.", ".93", "4.9", "4.930", "4.9x", "abc", "4.93.1", "-4.93",
    ] {
        assert_eq!(parse_pup_version_txt(text), None, "{text:?}");
    }
}
