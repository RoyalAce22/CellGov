use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{
    lis_sf, mtspr, ori, shift_left, window, RFID, SC, WINDOW,
};
use super::hypercall_tests::NOP;
use super::*;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, SPR_SRR0, SPR_SRR1};

/// The index of the word after the return word in [`vector`].
const AFTER: u64 = 8;

/// A stub at [`VECTOR`], where control enters in real mode. It gives SRR0
/// the address of its word [`AFTER`] and SRR1 an MSR value with SF and IR
/// at 1, then runs `last`. Word [`AFTER`] branches to the path past the
/// window's `blr`.
fn vector(last: u32) -> Segment {
    let after = VECTOR + 4 * AFTER;
    let to_past = WINDOW + 0x10;
    let words = [
        lis_sf(3),
        shift_left(3, 32),
        ori(3, after - VECTOR),
        lis_sf(4),
        ori(4, MSR_IR),
        mtspr(SPR_SRR0, 3),
        mtspr(SPR_SRR1, 4),
        last,
        word(PpuInstruction::B {
            offset: i32::try_from(to_past.wrapping_sub(after).cast_signed()).expect("near"),
            aa: false,
            link: false,
        }),
    ];
    code(VECTOR, &words)
}

/// The window's site, with `other` on the path past its `blr`.
fn site(other: i16, last: u32) -> Vec<(u64, Option<u16>)> {
    ordinals(0, &[vector(last), window(other)])
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets NIA to SRR0 0:61 || 0b00 and sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
#[test]
fn a_real_mode_rfid_to_translated_code_puts_no_later_word_in_real_mode() {
    assert_eq!(site(76, RFID), [(SC, Some(75))]);
}

/// A `nop` in place of the return falls through, so the branch after it
/// and the path past the window run in real mode, and that path's
/// absolute branch reaches the window's `nop`.
///
/// [PPC-Book1 p:24 s:2.4] `ba` reaches EXTS(LI || 0b00).
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_fall_through_in_place_of_the_return_puts_the_later_words_in_real_mode() {
    assert_eq!(site(76, NOP), [(SC, None)]);
    assert_eq!(site(75, NOP), [(SC, Some(75))]);
}
