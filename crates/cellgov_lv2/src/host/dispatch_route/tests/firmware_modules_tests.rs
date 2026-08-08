use super::*;
use cellgov_ps3_abi::dev_flash::FIRMWARE_MODULE_STEMS;

#[test]
fn stems_are_sorted_and_unique() {
    for w in FIRMWARE_MODULE_STEMS.windows(2) {
        assert!(w[0] < w[1], "{:?} >= {:?}", w[0], w[1]);
    }
}

#[test]
fn known_and_unknown_stems_classify() {
    assert!(is_known_firmware_stem("libsysmodule"));
    assert!(is_known_firmware_stem("liblv2"));
    assert!(!is_known_firmware_stem("libtypo"));
    assert!(!is_known_firmware_stem(""));
}
