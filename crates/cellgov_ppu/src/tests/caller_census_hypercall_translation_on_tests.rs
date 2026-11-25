use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{lis_sf, mtmsr, mtmsrd, ori, window, SC, WINDOW};
use super::hypercall_tests::{li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, MSR_PR};

/// `mfmsr rt`.
///
/// [PPC-Book3 p:119 s:Appendix G] `mfmsr` is an X-form word with primary opcode 31 and extended opcode 83.
fn mfmsr(rt: u8) -> u32 {
    (31 << 26) | (u32::from(rt) << 21) | (83 << 1)
}

/// A stub at [`VECTOR`], where control enters in real mode. It runs
/// `set` with the value that `msr` builds in r5, then branches to the
/// path past the window's `blr`.
fn vector(msr: &[u32], set: u32) -> Segment {
    let mut words = msr.to_vec();
    words.push(set);
    let at = VECTOR + 4 * words.len() as u64;
    let to_past = WINDOW + 0x10;
    words.push(word(PpuInstruction::B {
        offset: i32::try_from(to_past.wrapping_sub(at).cast_signed()).expect("a near branch"),
        aa: false,
        link: false,
    }));
    code(VECTOR, &words)
}

/// The window's site, with `other` on the path past its `blr`.
fn site(other: i16, msr: &[u32], set: u32) -> Vec<(u64, Option<u16>)> {
    ordinals(0, &[vector(msr, set), window(other)])
}

/// The low MSR bit `bit` as the immediate of an `li`.
fn low(bit: u64) -> i16 {
    i16::try_from(bit).expect("a low MSR bit")
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
/// [PPC-Book3 p:4 s:1.6.1] `mtmsrd` with L = 0 is context synchronizing, so the words that follow it are fetched and run in the context that it establishes.
#[test]
fn a_real_mode_mtmsrd_that_ors_ir_into_its_value_puts_no_later_word_in_real_mode() {
    let built = [lis_sf(5), ori(5, MSR_IR)];
    assert_eq!(site(76, &built, mtmsrd(5, false)), [(SC, Some(75))]);
    let read = [mfmsr(5), ori(5, MSR_IR)];
    assert_eq!(site(76, &read, mtmsrd(5, false)), [(SC, Some(75))]);
}

/// The constants that an `li` builds carry no known MSR bits.
///
/// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
/// [PPC-Book3 p:4 s:1.6.1] `mtmsr` with L = 0 is context synchronizing, so the words that follow it are fetched and run in the context that it establishes.
#[test]
fn a_real_mode_mtmsr_of_a_constant_with_ir_or_pr_puts_no_later_word_in_real_mode() {
    for bit in [MSR_IR, MSR_PR] {
        let set = site(76, &[li(5, low(bit))], mtmsr(5, false));
        assert_eq!(set, [(SC, Some(75))], "{bit:#x}");
    }
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 1 copies only RS bits 48 and 62 into the MSR and leaves MSR\[IR\] unchanged.
#[test]
fn a_real_mode_mtmsrd_with_l_set_keeps_the_later_words_in_real_mode() {
    let built = [lis_sf(5), ori(5, MSR_IR)];
    assert_eq!(site(76, &built, mtmsrd(5, true)), [(SC, None)]);
    assert_eq!(site(75, &built, mtmsrd(5, true)), [(SC, Some(75))]);
}

/// A second segment holds a `nop` where the first holds the `mtmsr`. The
/// fall-through of that segment runs the later words in real mode. The
/// second segment is at [`VECTOR`], and then also holds the word before
/// the `mtmsr`, or it holds only the `nop`.
#[test]
fn a_word_that_two_segments_hold_keeps_the_later_words_in_real_mode() {
    let seconds: [fn() -> Segment; 2] = [|| vector(&[li(5, 0)], NOP), || code(VECTOR + 4, &[NOP])];
    let overlapped = |second: Segment, other: i16| {
        ordinals(
            0,
            &[
                vector(&[li(5, low(MSR_IR))], mtmsr(5, false)),
                second,
                window(other),
            ],
        )
    };
    for (layout, second) in seconds.iter().enumerate() {
        assert_eq!(overlapped(second(), 76), [(SC, None)], "layout {layout}");
        assert_eq!(
            overlapped(second(), 75),
            [(SC, Some(75))],
            "layout {layout}"
        );
    }
}

/// A second segment at [`VECTOR`] holds `li r5,0` where the first holds
/// the word before the `mtmsr`. Its fall-through runs the `mtmsr` with
/// MSR\[IR\] and MSR\[PR\] at 0 in r5, so the later words run in real mode.
///
/// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
#[test]
fn a_word_after_a_word_that_two_segments_hold_keeps_the_later_words_in_real_mode() {
    let overlapped = |other| {
        ordinals(
            0,
            &[
                vector(&[li(5, low(MSR_IR))], mtmsr(5, false)),
                code(VECTOR, &[li(5, 0)]),
                window(other),
            ],
        )
    };
    assert_eq!(overlapped(76), [(SC, None)]);
    assert_eq!(overlapped(75), [(SC, Some(75))]);
}

/// The value holds the MSR of the word, so the mode stays real, and the
/// path past the window's `blr` reaches the word after its `li r11`.
///
/// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT.
/// [PPC-Book1 p:24 s:2.4] `ba` reaches EXTS(LI || 0b00).
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_mtmsrd_of_the_msr_keeps_the_later_words_in_real_mode() {
    assert_eq!(site(76, &[mfmsr(5)], mtmsrd(5, false)), [(SC, None)]);
    assert_eq!(site(75, &[mfmsr(5)], mtmsrd(5, false)), [(SC, Some(75))]);
}
