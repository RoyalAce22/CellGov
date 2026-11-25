//! The census's `lswi` and `stswi` slots held to the
//! `ppu_string_word_slots` capture from the reference console.
//!
//! For each case, the probe:
//! - starts with 0x8000000000008000 in r0, the CTR that
//!   `after_a_load_of_part_of_a_copy` builds with `low` 0x8000;
//! - runs the case's words over a buffer;
//! - records r0.
//!
//! The census runs the same words through r1. The `bctr` must enter the
//! address in the console's r0, and no other address.

use super::hypercall_ctr_decrement_tests::QUIET_SC;
use super::hypercall_ctr_stack_copy_tests::after_a_load_of_part_of_a_copy;
use super::hypercall_entry_tests::word;
use super::*;

/// The probe's done marker, "STRW".
const DONE: u64 = 0x5354_5257;

/// The probe's payload, as big-endian u64 words.
fn console() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_string_word_slots/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 24, "payload words");
    assert_eq!(words[23], DONE, "the probe wrote every word");
    words
}

/// `std rs,imm(r1)`.
fn std(rs: u8, imm: i16) -> u32 {
    word(PpuInstruction::Std { rs, ra: 1, imm })
}

/// `lwz rt,imm(r1)`.
fn lwz(rt: u8, imm: i16) -> u32 {
    word(PpuInstruction::Lwz { rt, ra: 1, imm })
}

/// `lhz rt,0(r1)`.
fn lhz(rt: u8) -> u32 {
    word(PpuInstruction::Lhz { rt, ra: 1, imm: 0 })
}

/// `stswi rs,r1,nb`.
fn stswi(rs: u8, nb: u8) -> u32 {
    word(PpuInstruction::Stswi { rs, ra: 1, nb })
}

/// `lswi rt,r1,nb`.
fn lswi(rt: u8, nb: u8) -> u32 {
    word(PpuInstruction::Lswi { rt, ra: 1, nb })
}

/// Asserts that the `bctr` after `between` enters `r0`, the console's r0.
///
/// The `sc` at the other of 0 and 0x8000 stays resolved, so the `bctr`
/// does not enter it.
fn enters_only(r0: u64, between: &[u32]) {
    let target = u16::try_from(r0).unwrap_or_else(|_| panic!("console r0 {r0:#x}"));
    let other = target ^ 0x8000;
    assert_eq!(
        after_a_load_of_part_of_a_copy(target, 0x8000, between),
        [(u64::from(target) + 4, None), (QUIET_SC, Some(77))],
        "the bctr enters {target:#x}"
    );
    assert_eq!(
        after_a_load_of_part_of_a_copy(other, 0x8000, between),
        [(u64::from(other) + 4, Some(75)), (QUIET_SC, Some(77))],
        "the bctr does not enter {other:#x}"
    );
}

/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
#[test]
fn a_string_store_stores_the_low_word_and_its_high_order_bytes_where_the_console_does() {
    let w = console();
    enters_only(w[1], &[stswi(0, 4), lwz(0, 0)]);
    enters_only(w[3], &[stswi(0, 2), lhz(0)]);
}

/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
#[test]
fn a_string_store_that_wraps_to_r0_or_stores_32_bytes_stores_r0_where_the_console_does() {
    let w = console();
    enters_only(w[16], &[stswi(31, 8), lwz(0, 4)]);
    enters_only(w[22], &[stswi(25, 0), lwz(0, 28)]);
}

/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
#[test]
fn a_string_load_that_wraps_to_r0_or_loads_32_bytes_loads_r0_as_the_console_does() {
    let w = console();
    enters_only(w[5], &[std(0, 0), lswi(31, 8)]);
    enters_only(w[13], &[std(0, 24), lswi(25, 0)]);
}

/// The console's r0 after `lswi r0,r1,2` holds two bytes at the left of
/// its low word. It is neither 0 nor 0x8000, so the census enters neither.
///
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
#[test]
fn a_string_load_of_two_bytes_enters_no_site_that_the_console_r0_does_not_name() {
    let w = console();
    let between = [std(0, 0), lswi(0, 2)];
    for nop in [0u16, 0x8000] {
        assert_ne!(w[14], u64::from(nop), "console r0");
        assert_eq!(
            after_a_load_of_part_of_a_copy(nop, 0x8000, &between),
            [(u64::from(nop) + 4, Some(75)), (QUIET_SC, Some(77))],
            "the bctr does not enter {nop:#x}"
        );
    }
}
