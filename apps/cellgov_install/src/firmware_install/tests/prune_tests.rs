//! The install-time dev_flash exclusion predicate.

use super::*;

#[test]
fn install_exclusion_prunes_the_backward_compat_emulators() {
    assert!(is_install_excluded("dev_flash/ps1emu/ps1_emu.self"));
    assert!(is_install_excluded("dev_flash/ps2emu/ps2_emu.self"));
    assert!(is_install_excluded("dev_flash/pspemu/flash0/font/x.pgf"));
    assert!(is_install_excluded("ps2emu/ps2_netemu.self"));
}

#[test]
fn install_exclusion_keeps_real_firmware_paths() {
    assert!(!is_install_excluded("dev_flash/sys/external/liblv2.sprx"));
    assert!(!is_install_excluded("dev_flash/vsh/module/mcore_tk.self"));
    // "pspemu" matches only as a leading path component, not a substring.
    assert!(!is_install_excluded("dev_flash/data/pspemu_notes.txt"));
    // A sibling mount is not dev_flash content, so the dev_flash-rooted
    // prune list must not reach into it.
    assert!(!is_install_excluded("dev_flash2/ps2emu/x.self"));
}

#[test]
fn install_exclusion_prunes_through_the_packaging_prefixes_the_extractor_strips() {
    // The extractor routes all four of these to dev_flash/ps2emu/...,
    // so the prune has to see them as the same entry.
    assert!(is_install_excluded("ps2emu/ps2_netemu.self"));
    assert!(is_install_excluded("/ps2emu/ps2_netemu.self"));
    assert!(is_install_excluded("000/ps2emu/ps2_netemu.self"));
    assert!(is_install_excluded("000/dev_flash/ps2emu/ps2_netemu.self"));
}
