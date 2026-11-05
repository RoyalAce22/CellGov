//! An update-form load or store whose RA is r11 drops the candidate; the
//! same form without update keeps it.

use super::*;

const LI_R11_22: u32 = (14 << 26) | (11 << 21) | 22;
const SC: u32 = 0x4400_0002;

/// A D- or DS-form word with RT/FRT r3, RA r11, displacement 8, and the
/// low two bits `xo`.
const fn d_form(primary: u32, xo: u32) -> u32 {
    (primary << 26) | (3 << 21) | (11 << 16) | 8 | xo
}

fn ordinal_after(word: u32) -> Option<u16> {
    let sites = scan_words(0, &[LI_R11_22, word, SC]);
    assert_eq!(sites.len(), 1, "0x{word:08x}");
    sites[0].ordinal
}

#[test]
fn an_update_form_with_ra_r11_drops_the_candidate() {
    // ldu, lfsu, lfdu, stfsu, stfdu.
    for word in [
        d_form(58, 1),
        d_form(49, 0),
        d_form(51, 0),
        d_form(53, 0),
        d_form(55, 0),
    ] {
        assert_eq!(ordinal_after(word), None, "0x{word:08x}");
    }
}

#[test]
fn the_same_form_without_update_keeps_the_candidate() {
    // ld, lwa, lfs, lfd, stfs, stfd.
    for word in [
        d_form(58, 0),
        d_form(58, 2),
        d_form(48, 0),
        d_form(50, 0),
        d_form(52, 0),
        d_form(54, 0),
    ] {
        assert_eq!(ordinal_after(word), Some(22), "0x{word:08x}");
    }
}
