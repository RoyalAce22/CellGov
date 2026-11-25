use super::hypercall_entry_tests::{code, Segment};
use super::hypercall_real_mode_switch_tests::{
    build, lis_sf, mtspr, ori, shift_left, site, OUTSIDE, PAST, RFID, SC,
};
use super::hypercall_tests::li;
use cellgov_ps3_abi::hw::ppu_supervisor::{SPR_SRR0, SPR_SRR1};

/// The real address of the window's `li r11,75`.
const WINDOW_LOW: u16 = 0x3000;

/// The real address of the window's `nop`. Bit 61 is set, so the mask
/// keeps the word only when it clears bits 62 and 63 and no other bit.
const NOP_LOW: u16 = WINDOW_LOW + 4;

/// A stub at [`OUTSIDE`] that gives SRR0 `srr0` and SRR1 the value SF,
/// with IR and PR at 0, then runs `rfid`.
fn rfid_sf(srr0: u16) -> Segment {
    let mut words = build(3, srr0).to_vec();
    words.extend([lis_sf(4), mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 4), RFID]);
    code(OUTSIDE, &words)
}

/// A stub at [`OUTSIDE`] that gives SRR0 the value 0x1_0000_0000 |
/// `low` and SRR1 the value 0, then runs `rfid`.
fn rfid_sf_0(low: u16) -> Segment {
    let words = [
        li(3, 1),
        shift_left(3, 32),
        ori(3, u64::from(low)),
        li(4, 0),
        mtspr(SPR_SRR0, 3),
        mtspr(SPR_SRR1, 4),
        RFID,
    ];
    code(OUTSIDE, &words)
}

/// SRR0 names [`PAST`] with bit 62 or 63 set, or names the window's
/// `li r11,75` or `nop` with both bits set.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\], and fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR.
#[test]
fn an_rfid_that_clears_ir_to_srr0_with_low_bits_set_enters_its_word_in_real_mode() {
    for low_bits in 1..=3 {
        assert_eq!(
            site(76, rfid_sf(PAST | low_bits)),
            [(SC, None)],
            "{low_bits}"
        );
    }
    assert_eq!(site(76, rfid_sf(WINDOW_LOW | 3)), [(SC, Some(75))]);
    assert_eq!(site(76, rfid_sf(NOP_LOW | 3)), [(SC, None)]);
}

/// SRR0 has a high word, and its low word names [`PAST`] with bit 62 or
/// 63 set, or names the window's `li r11,75` or `nop` with both bits set.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\], and fetches from 32 zeros || SRR0 32:61 || 0b00 when SF = 0 in the new MSR.
#[test]
fn an_rfid_that_clears_ir_with_sf_0_to_srr0_with_low_bits_set_enters_its_word_in_real_mode() {
    for low_bits in 1..=3 {
        assert_eq!(
            site(76, rfid_sf_0(PAST | low_bits)),
            [(SC, None)],
            "{low_bits}"
        );
    }
    assert_eq!(site(76, rfid_sf_0(WINDOW_LOW | 3)), [(SC, Some(75))]);
    assert_eq!(site(76, rfid_sf_0(NOP_LOW | 3)), [(SC, None)]);
}
