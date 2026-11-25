use std::collections::BTreeSet;

use super::partial_constant_tests::building_r9_around;
use super::tests::{at, word};
use crate::decode::decode;
use crate::instruction::{ppe_illegal_mnemonic, ppe_privileged_mnemonic, PpuInstruction};

/// Asserts that each word of `cases`, between `lis r9,1` and
/// `addi r3,r9,0x10`, ends the straight-line path: only `lis` names a
/// word.
fn assert_path_ends(cases: &[(u32, &str)]) {
    assert!(!cases.is_empty(), "a path-end check needs a case");
    for &(between, text) in cases {
        assert_eq!(
            building_r9_around(between),
            BTreeSet::from([at(0)]),
            "{text}"
        );
    }
}

/// Asserts that r9 keeps its constant across each word of `cases`, so
/// `addi` also names a word.
fn assert_path_kept(cases: &[(u32, &str)]) {
    assert!(!cases.is_empty(), "a path-kept check needs a case");
    for &(between, text) in cases {
        assert_eq!(
            building_r9_around(between),
            BTreeSet::from([at(0), at(4)]),
            "{text}"
        );
    }
}

/// The raw XL-form or X-form word with primary opcode `primary`, RT
/// `rt` and the 10-bit extended opcode `xo`.
fn x_word(primary: u32, rt: u32, xo: u32) -> u32 {
    (primary << 26) | (rt << 21) | (xo << 1)
}

fn privileged(raw: u32, mnemonic: &str) -> u32 {
    assert_eq!(decode(raw), Ok(PpuInstruction::PpePrivileged { raw }));
    assert_eq!(ppe_privileged_mnemonic(raw), Some(mnemonic));
    raw
}

fn illegal(raw: u32, mnemonic: &str) -> u32 {
    assert_eq!(decode(raw), Ok(PpuInstruction::PpeIllegal { raw }));
    assert_eq!(ppe_illegal_mnemonic(raw), Some(mnemonic));
    raw
}

fn tw(to: u8, ra: u8, rb: u8) -> u32 {
    word(PpuInstruction::Tw { to, ra, rb })
}

fn td(to: u8, ra: u8, rb: u8) -> u32 {
    word(PpuInstruction::Td { to, ra, rb })
}

fn twi(to: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Twi { to, ra, imm })
}

fn tdi(to: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Tdi { to, ra, imm })
}

/// [PPC-Book3 p:119 s:Appendix G] rfid is XL-form 19/18 and hrfid is XL-form 19/274.
/// [PPC-Book3 p:13 s:2.3] rfid and hrfid fetch the next instruction from SRR0 or HSRR0.
#[test]
fn rfid_and_hrfid_end_the_straight_line_path() {
    assert_path_ends(&[
        (privileged(x_word(19, 0, 18), "rfid"), "rfid"),
        (privileged(x_word(19, 0, 274), "hrfid"), "hrfid"),
    ]);
}

/// [PPC-Book3 p:54 s:4.11.3.2] tlbiel is X-form 31/274, and it does not branch.
#[test]
fn tlbiel_which_shares_the_hrfid_extended_opcode_keeps_the_path() {
    let tlbiel_r10 = privileged(x_word(31, 0, 274) | (10 << 11), "tlbiel");
    assert_path_kept(&[(tlbiel_r10, "tlbiel r10")]);
}

/// [PPC-Book1 p:62 s:3.3.10] A trap invokes the trap handler when any condition that TO selects is met.
#[test]
fn a_trap_with_every_to_bit_set_ends_the_path() {
    assert_path_ends(&[
        (tw(31, 0, 0), "tw 31,r0,r0"),
        (td(31, 5, 6), "td 31,r5,r6"),
        (twi(31, 0, 0), "twi 31,r0,0"),
        (tdi(31, 5, 1), "tdi 31,r5,1"),
    ]);
}

/// [PPC-Book1 p:62 s:3.3.10] TO bits 0 to 4 select signed less than, signed greater than, equal, unsigned less than and unsigned greater than.
#[test]
fn a_trap_whose_to_bits_cover_every_comparison_ends_the_path() {
    assert_path_ends(&[
        (tw(0b11100, 5, 6), "tw 28,r5,r6"),
        (tdi(0b00111, 5, 1), "tdi 7,r5,1"),
    ]);
}

/// [PPC-Book1 p:62 s:3.3.10] TO bit 2 selects the equal condition.
#[test]
fn an_equal_trap_of_a_register_with_itself_ends_the_path() {
    assert_path_ends(&[
        (tw(0b00100, 5, 5), "tw 4,r5,r5"),
        (td(0b00100, 5, 5), "td 4,r5,r5"),
    ]);
}

/// [PPC-Book1 p:62 s:3.3.10] When no condition that TO selects is met, execution continues normally.
#[test]
fn a_trap_that_some_operand_values_pass_keeps_the_path() {
    assert_path_kept(&[
        (tw(0b00100, 5, 6), "tw 4,r5,r6"),
        (td(0b11011, 5, 6), "td 27,r5,r6"),
        (tw(0b11011, 5, 5), "tw 27,r5,r5"),
        (twi(0b10110, 5, 1), "twi 22,r5,1"),
        (tdi(0b01101, 5, 1), "tdi 13,r5,1"),
    ]);
}

/// [CBE-Handbook p:738 s:A.2.4.1] The PPE does not implement popcntb.
/// [CBE-Handbook p:739 s:A.2.4.1] An attempt to execute popcntb or mcrxr, which the PPE does not implement, causes an illegal instruction exception.
/// [CBE-Handbook p:739 s:A.2.4.2] An attempt to execute eciwx, which the PPE does not implement, causes an illegal instruction exception.
#[test]
fn a_word_the_ppe_does_not_implement_keeps_the_path() {
    assert_path_kept(&[
        (
            word(PpuInstruction::Popcntb { ra: 4, rs: 5 }),
            "popcntb r4,r5",
        ),
        (word(PpuInstruction::Mcrxr { bf: 0 }), "mcrxr cr0"),
        (
            illegal(x_word(31, 4, 310) | (5 << 11), "eciwx"),
            "eciwx r4,r0,r5",
        ),
        (illegal(x_word(31, 4, 339), "mfspr"), "mfspr r4,0"),
        (illegal(x_word(31, 4, 467), "mtspr"), "mtspr 0,r4"),
    ]);
}
